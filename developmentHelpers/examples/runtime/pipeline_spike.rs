//! Synthetic queue/backpressure and Arc/slab allocation experiment for the planned runtime.
//! Analyzer, recorder and player consumers here are simulated, not product sessions.

use std::env;
use std::error::Error;
use std::fmt::Display;
use std::hint::black_box;
use std::io;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const PACKET_SIZE_BYTES: usize = 188;
const DEFAULT_DURATION_SECONDS: u64 = 10;
const DEFAULT_BATCH_PACKETS: usize = 128;
const DEFAULT_QUEUE_DEPTH: usize = 32;
const DEFAULT_TARGET_MBPS: u64 = 100;
const PIPELINE_BRANCH_COUNT: usize = 3;
const PIPELINE_ACTIVE_BUFFER_COUNT: usize = 4;
const MAX_DURATION_SECONDS: u64 = 86_400;
const MAX_BATCH_PACKETS: usize = 8_192;
const MAX_QUEUE_DEPTH: usize = 65_536;
const MAX_TARGET_MBPS: u64 = 100_000;
const MAX_SLAB_POOL_BYTES: usize = 512 * 1024 * 1024;
const SLOW_PLAYER_DELAY: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MemoryModel {
    Arc,
    Slab,
}

impl Display for MemoryModel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Arc => formatter.write_str("arc"),
            Self::Slab => formatter.write_str("slab"),
        }
    }
}

impl FromStr for MemoryModel {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "arc" => Ok(Self::Arc),
            "slab" => Ok(Self::Slab),
            _ => Err("expected arc or slab"),
        }
    }
}

#[derive(Debug)]
struct Config {
    duration_seconds: u64,
    batch_packets: usize,
    queue_depth: usize,
    target_mbps: u64,
    memory_model: MemoryModel,
    slow_player: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            duration_seconds: DEFAULT_DURATION_SECONDS,
            batch_packets: DEFAULT_BATCH_PACKETS,
            queue_depth: DEFAULT_QUEUE_DEPTH,
            target_mbps: DEFAULT_TARGET_MBPS,
            memory_model: MemoryModel::Arc,
            slow_player: false,
        }
    }
}

impl Config {
    fn validate(&self) -> io::Result<()> {
        validate_range(
            "duration-seconds",
            self.duration_seconds,
            1,
            MAX_DURATION_SECONDS,
        )?;
        validate_range("batch-packets", self.batch_packets, 1, MAX_BATCH_PACKETS)?;
        validate_range("queue-depth", self.queue_depth, 1, MAX_QUEUE_DEPTH)?;
        validate_range("target-mbps", self.target_mbps, 1, MAX_TARGET_MBPS)?;
        if self.memory_model == MemoryModel::Slab {
            let pool_bytes = self
                .batch_bytes()
                .checked_mul(self.slab_pool_capacity())
                .ok_or_else(|| invalid_input("slab pool size overflow"))?;
            if pool_bytes > MAX_SLAB_POOL_BYTES {
                return Err(invalid_input(format!(
                    "slab pool requires {pool_bytes} bytes; maximum is {MAX_SLAB_POOL_BYTES}"
                )));
            }
        }

        Ok(())
    }

    fn batch_bytes(&self) -> usize {
        PACKET_SIZE_BYTES * self.batch_packets
    }

    fn slab_pool_capacity(&self) -> usize {
        self.queue_depth
            .saturating_mul(PIPELINE_BRANCH_COUNT)
            .saturating_add(PIPELINE_ACTIVE_BUFFER_COUNT)
    }
}

#[derive(Clone)]
struct PacketBatch {
    sequence: u64,
    packet_count: usize,
    bytes: Arc<BatchPayload>,
}

struct BatchPayload {
    bytes: Option<Vec<u8>>,
    recycler: Option<Arc<BufferPool>>,
}

impl BatchPayload {
    fn as_slice(&self) -> &[u8] {
        self.bytes.as_deref().unwrap_or_default()
    }

    fn len(&self) -> usize {
        self.bytes.as_ref().map_or(0, Vec::len)
    }
}

impl Drop for BatchPayload {
    fn drop(&mut self) {
        if let (Some(recycler), Some(bytes)) = (&self.recycler, self.bytes.take()) {
            recycler.release(bytes);
        }
    }
}

struct BufferPool {
    buffer_size: usize,
    capacity: usize,
    buffers: Mutex<Vec<Vec<u8>>>,
    buffer_allocations: AtomicU64,
    buffer_reuses: AtomicU64,
    fallback_allocations: AtomicU64,
    discarded_buffers: AtomicU64,
}

impl BufferPool {
    fn new(buffer_size: usize, capacity: usize) -> Self {
        let buffers = (0..capacity).map(|_| vec![0xff; buffer_size]).collect();

        Self {
            buffer_size,
            capacity,
            buffers: Mutex::new(buffers),
            buffer_allocations: AtomicU64::new(capacity as u64),
            buffer_reuses: AtomicU64::new(0),
            fallback_allocations: AtomicU64::new(0),
            discarded_buffers: AtomicU64::new(0),
        }
    }

    fn acquire(&self) -> Vec<u8> {
        let mut buffers = self
            .buffers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(buffer) = buffers.pop() {
            self.buffer_reuses.fetch_add(1, Ordering::Relaxed);
            return buffer;
        }
        drop(buffers);

        self.buffer_allocations.fetch_add(1, Ordering::Relaxed);
        self.fallback_allocations.fetch_add(1, Ordering::Relaxed);
        vec![0xff; self.buffer_size]
    }

    fn release(&self, buffer: Vec<u8>) {
        debug_assert_eq!(buffer.len(), self.buffer_size);
        let mut buffers = self
            .buffers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if buffers.len() < self.capacity {
            buffers.push(buffer);
        } else {
            self.discarded_buffers.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn snapshot(&self) -> PayloadSnapshot {
        let available_buffers = self
            .buffers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len();

        PayloadSnapshot {
            buffer_allocations: self.buffer_allocations.load(Ordering::Relaxed),
            buffer_reuses: self.buffer_reuses.load(Ordering::Relaxed),
            pool_capacity: self.capacity,
            pool_available_end: available_buffers,
            fallback_allocations: self.fallback_allocations.load(Ordering::Relaxed),
            discarded_buffers: self.discarded_buffers.load(Ordering::Relaxed),
        }
    }
}

enum PayloadFactory {
    Arc {
        buffer_size: usize,
        buffer_allocations: AtomicU64,
    },
    Slab {
        pool: Arc<BufferPool>,
    },
}

impl PayloadFactory {
    fn new(config: &Config) -> Self {
        match config.memory_model {
            MemoryModel::Arc => Self::Arc {
                buffer_size: config.batch_bytes(),
                buffer_allocations: AtomicU64::new(0),
            },
            MemoryModel::Slab => Self::Slab {
                pool: Arc::new(BufferPool::new(
                    config.batch_bytes(),
                    config.slab_pool_capacity(),
                )),
            },
        }
    }

    fn acquire(&self) -> (Vec<u8>, Option<Arc<BufferPool>>) {
        match self {
            Self::Arc {
                buffer_size,
                buffer_allocations,
            } => {
                buffer_allocations.fetch_add(1, Ordering::Relaxed);
                (vec![0xff; *buffer_size], None)
            }
            Self::Slab { pool } => (pool.acquire(), Some(Arc::clone(pool))),
        }
    }

    fn snapshot(&self) -> PayloadSnapshot {
        match self {
            Self::Arc {
                buffer_allocations, ..
            } => PayloadSnapshot {
                buffer_allocations: buffer_allocations.load(Ordering::Relaxed),
                buffer_reuses: 0,
                pool_capacity: 0,
                pool_available_end: 0,
                fallback_allocations: 0,
                discarded_buffers: 0,
            },
            Self::Slab { pool } => pool.snapshot(),
        }
    }
}

#[derive(Debug)]
struct PayloadSnapshot {
    buffer_allocations: u64,
    buffer_reuses: u64,
    pool_capacity: usize,
    pool_available_end: usize,
    fallback_allocations: u64,
    discarded_buffers: u64,
}

struct QueueMetrics {
    capacity: usize,
    available_slots: AtomicUsize,
    accepted_batches: AtomicU64,
    processed_batches: AtomicU64,
    processed_packets: AtomicU64,
    processed_bytes: AtomicU64,
    dropped_batches: AtomicU64,
    dropped_packets: AtomicU64,
    disconnected_events: AtomicU64,
    peak_depth: AtomicUsize,
}

impl QueueMetrics {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            available_slots: AtomicUsize::new(capacity),
            accepted_batches: AtomicU64::new(0),
            processed_batches: AtomicU64::new(0),
            processed_packets: AtomicU64::new(0),
            processed_bytes: AtomicU64::new(0),
            dropped_batches: AtomicU64::new(0),
            dropped_packets: AtomicU64::new(0),
            disconnected_events: AtomicU64::new(0),
            peak_depth: AtomicUsize::new(0),
        }
    }

    fn try_reserve_slot(&self) -> Option<usize> {
        self.available_slots
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |available| {
                available.checked_sub(1)
            })
            .ok()
            .map(|previous_available| self.capacity - previous_available + 1)
    }

    fn record_accepted(&self, reserved_depth: usize) {
        self.accepted_batches.fetch_add(1, Ordering::Relaxed);
        self.peak_depth.fetch_max(reserved_depth, Ordering::Relaxed);
    }

    fn record_rejected(&self, packet_count: usize, disconnected: bool) {
        self.dropped_batches.fetch_add(1, Ordering::Relaxed);
        self.dropped_packets
            .fetch_add(packet_count as u64, Ordering::Relaxed);
        if disconnected {
            self.disconnected_events.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn release_slot(&self) {
        let previous_available = self.available_slots.fetch_add(1, Ordering::Release);
        debug_assert!(previous_available < self.capacity);
    }

    fn record_processed(&self, packet_count: usize, byte_count: usize) {
        self.processed_batches.fetch_add(1, Ordering::Relaxed);
        self.processed_packets
            .fetch_add(packet_count as u64, Ordering::Relaxed);
        self.processed_bytes
            .fetch_add(byte_count as u64, Ordering::Relaxed);
    }

    fn snapshot(&self) -> QueueSnapshot {
        QueueSnapshot {
            accepted_batches: self.accepted_batches.load(Ordering::Relaxed),
            processed_batches: self.processed_batches.load(Ordering::Relaxed),
            processed_packets: self.processed_packets.load(Ordering::Relaxed),
            processed_bytes: self.processed_bytes.load(Ordering::Relaxed),
            dropped_batches: self.dropped_batches.load(Ordering::Relaxed),
            dropped_packets: self.dropped_packets.load(Ordering::Relaxed),
            disconnected_events: self.disconnected_events.load(Ordering::Relaxed),
            current_depth: self.capacity - self.available_slots.load(Ordering::Acquire),
            peak_depth: self.peak_depth.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug)]
struct QueueSnapshot {
    accepted_batches: u64,
    processed_batches: u64,
    processed_packets: u64,
    processed_bytes: u64,
    dropped_batches: u64,
    dropped_packets: u64,
    disconnected_events: u64,
    current_depth: usize,
    peak_depth: usize,
}

struct QueueTarget {
    sender: SyncSender<PacketBatch>,
    metrics: Arc<QueueMetrics>,
}

struct ConsumerHandle {
    name: &'static str,
    handle: JoinHandle<()>,
}

struct PipelineResult {
    produced_batches: u64,
    produced_packets: u64,
    produced_bytes: u64,
    production_elapsed: Duration,
    total_elapsed: Duration,
    payload: PayloadSnapshot,
    analyzer: QueueSnapshot,
    recorder: QueueSnapshot,
    player: QueueSnapshot,
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn validate_range<T>(name: &str, value: T, minimum: T, maximum: T) -> io::Result<()>
where
    T: Copy + Display + PartialOrd,
{
    if value < minimum || value > maximum {
        return Err(invalid_input(format!(
            "--{name} must be between {minimum} and {maximum}, found {value}"
        )));
    }

    Ok(())
}

fn parse_value<T>(value: Option<String>, option: &str) -> io::Result<T>
where
    T: FromStr,
    T::Err: Display,
{
    let value = value.ok_or_else(|| invalid_input(format!("missing value for {option}")))?;
    value
        .parse()
        .map_err(|error| invalid_input(format!("invalid value for {option}: {error}")))
}

fn print_usage() {
    println!(
        "Usage: cargo run --release -p tsan-runtime --example pipeline_spike -- \
         [--duration-seconds N] [--batch-packets N] [--queue-depth N] \
         [--target-mbps N] [--memory-model arc|slab] [--slow-player]"
    );
}

fn parse_args() -> io::Result<Option<Config>> {
    let mut config = Config::default();
    let mut args = env::args().skip(1);

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--duration-seconds" => {
                config.duration_seconds = parse_value(args.next(), "--duration-seconds")?;
            }
            "--batch-packets" => {
                config.batch_packets = parse_value(args.next(), "--batch-packets")?;
            }
            "--queue-depth" => {
                config.queue_depth = parse_value(args.next(), "--queue-depth")?;
            }
            "--target-mbps" => {
                config.target_mbps = parse_value(args.next(), "--target-mbps")?;
            }
            "--memory-model" => {
                config.memory_model = parse_value(args.next(), "--memory-model")?;
            }
            "--slow-player" => config.slow_player = true,
            "--help" | "-h" => {
                print_usage();
                return Ok(None);
            }
            _ => {
                return Err(invalid_input(format!(
                    "unknown argument: {argument}; use --help for usage"
                )));
            }
        }
    }

    config.validate()?;
    Ok(Some(config))
}

fn make_batch(sequence: u64, packet_count: usize, payload_factory: &PayloadFactory) -> PacketBatch {
    let (mut bytes, recycler) = payload_factory.acquire();
    bytes.fill(0xff);
    let first_packet_index = sequence.saturating_mul(packet_count as u64);

    for (packet_index, packet) in bytes.chunks_exact_mut(PACKET_SIZE_BYTES).enumerate() {
        let continuity_counter = first_packet_index.wrapping_add(packet_index as u64) as u8 & 0x0f;
        packet[0] = 0x47;
        packet[1] = 0x1f;
        packet[2] = 0xff;
        packet[3] = 0x10 | continuity_counter;
    }

    PacketBatch {
        sequence,
        packet_count,
        bytes: Arc::new(BatchPayload {
            bytes: Some(bytes),
            recycler,
        }),
    }
}

fn spawn_consumer(
    name: &'static str,
    receiver: Receiver<PacketBatch>,
    metrics: Arc<QueueMetrics>,
    processing_delay: Duration,
) -> io::Result<ConsumerHandle> {
    let handle = thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            while let Ok(batch) = receiver.recv() {
                metrics.release_slot();
                if !processing_delay.is_zero() {
                    thread::sleep(processing_delay);
                }
                black_box(batch.sequence);
                black_box(batch.bytes.as_slice());
                metrics.record_processed(batch.packet_count, batch.bytes.len());
            }
        })?;

    Ok(ConsumerHandle { name, handle })
}

fn dispatch_batch(target: &QueueTarget, batch: PacketBatch) {
    let Some(reserved_depth) = target.metrics.try_reserve_slot() else {
        target.metrics.record_rejected(batch.packet_count, false);
        return;
    };

    match target.sender.try_send(batch) {
        Ok(()) => target.metrics.record_accepted(reserved_depth),
        Err(TrySendError::Full(rejected)) => {
            target.metrics.release_slot();
            target.metrics.record_rejected(rejected.packet_count, false);
        }
        Err(TrySendError::Disconnected(rejected)) => {
            target.metrics.release_slot();
            target.metrics.record_rejected(rejected.packet_count, true);
        }
    }
}

fn join_consumers(handles: Vec<ConsumerHandle>) -> io::Result<()> {
    for consumer in handles {
        consumer
            .handle
            .join()
            .map_err(|_| io::Error::other(format!("{} consumer thread panicked", consumer.name)))?;
    }

    Ok(())
}

fn run_pipeline(config: &Config) -> io::Result<PipelineResult> {
    let payload_factory = PayloadFactory::new(config);
    let analyzer_metrics = Arc::new(QueueMetrics::new(config.queue_depth));
    let recorder_metrics = Arc::new(QueueMetrics::new(config.queue_depth));
    let player_metrics = Arc::new(QueueMetrics::new(config.queue_depth));
    let (analyzer_sender, analyzer_receiver) = sync_channel(config.queue_depth);
    let (recorder_sender, recorder_receiver) = sync_channel(config.queue_depth);
    let (player_sender, player_receiver) = sync_channel(config.queue_depth);

    let handles = vec![
        spawn_consumer(
            "analyzer",
            analyzer_receiver,
            Arc::clone(&analyzer_metrics),
            Duration::ZERO,
        )?,
        spawn_consumer(
            "recorder",
            recorder_receiver,
            Arc::clone(&recorder_metrics),
            Duration::ZERO,
        )?,
        spawn_consumer(
            "player",
            player_receiver,
            Arc::clone(&player_metrics),
            if config.slow_player {
                SLOW_PLAYER_DELAY
            } else {
                Duration::ZERO
            },
        )?,
    ];

    let targets = vec![
        QueueTarget {
            sender: analyzer_sender,
            metrics: Arc::clone(&analyzer_metrics),
        },
        QueueTarget {
            sender: recorder_sender,
            metrics: Arc::clone(&recorder_metrics),
        },
        QueueTarget {
            sender: player_sender,
            metrics: Arc::clone(&player_metrics),
        },
    ];

    let batch_bytes = config.batch_bytes();
    let target_bits_per_second = config.target_mbps as f64 * 1_000_000.0;
    let batch_interval = Duration::from_secs_f64(batch_bytes as f64 * 8.0 / target_bits_per_second);
    let requested_duration = Duration::from_secs(config.duration_seconds);
    let start = Instant::now();
    let deadline = start + requested_duration;
    let mut next_batch_at = start;
    let mut produced_batches = 0_u64;

    while next_batch_at < deadline {
        let now = Instant::now();
        if now < next_batch_at {
            thread::sleep(next_batch_at - now);
        }

        let batch = make_batch(produced_batches, config.batch_packets, &payload_factory);
        for target in &targets {
            dispatch_batch(target, batch.clone());
        }
        produced_batches += 1;
        next_batch_at += batch_interval;
    }

    let production_elapsed = start.elapsed();
    drop(targets);
    join_consumers(handles)?;
    let total_elapsed = start.elapsed();
    let produced_packets = produced_batches.saturating_mul(config.batch_packets as u64);
    let produced_bytes = produced_packets.saturating_mul(PACKET_SIZE_BYTES as u64);

    Ok(PipelineResult {
        produced_batches,
        produced_packets,
        produced_bytes,
        production_elapsed,
        total_elapsed,
        payload: payload_factory.snapshot(),
        analyzer: analyzer_metrics.snapshot(),
        recorder: recorder_metrics.snapshot(),
        player: player_metrics.snapshot(),
    })
}

fn validate_queue_result(
    name: &str,
    result: &QueueSnapshot,
    produced_batches: u64,
    queue_depth: usize,
) -> io::Result<()> {
    if result.accepted_batches + result.dropped_batches != produced_batches {
        return Err(io::Error::other(format!(
            "{name} accounting mismatch: accepted {} + dropped {} != produced {}",
            result.accepted_batches, result.dropped_batches, produced_batches
        )));
    }
    if result.processed_batches != result.accepted_batches {
        return Err(io::Error::other(format!(
            "{name} drain mismatch: processed {} != accepted {}",
            result.processed_batches, result.accepted_batches
        )));
    }
    if result.current_depth != 0 {
        return Err(io::Error::other(format!(
            "{name} queue did not drain: depth {}",
            result.current_depth
        )));
    }
    if result.peak_depth > queue_depth {
        return Err(io::Error::other(format!(
            "{name} peak depth {} exceeded queue depth {}",
            result.peak_depth, queue_depth
        )));
    }
    if result.disconnected_events != 0 {
        return Err(io::Error::other(format!(
            "{name} reported {} disconnected events",
            result.disconnected_events
        )));
    }

    Ok(())
}

fn validate_result(config: &Config, result: &PipelineResult) -> io::Result<()> {
    for (name, queue) in [
        ("analyzer", &result.analyzer),
        ("recorder", &result.recorder),
        ("player", &result.player),
    ] {
        validate_queue_result(name, queue, result.produced_batches, config.queue_depth)?;
    }

    if result.analyzer.dropped_batches != 0 || result.recorder.dropped_batches != 0 {
        return Err(io::Error::other(
            "analyzer or recorder dropped data during the spike",
        ));
    }
    if config.slow_player && result.player.dropped_batches == 0 {
        return Err(io::Error::other(
            "slow-player mode did not trigger player queue overflow",
        ));
    }
    if !config.slow_player && result.player.dropped_batches != 0 {
        return Err(io::Error::other(
            "player dropped data without slow-player injection",
        ));
    }
    match config.memory_model {
        MemoryModel::Arc => {
            if result.payload.buffer_allocations != result.produced_batches {
                return Err(io::Error::other(format!(
                    "arc allocation mismatch: allocations {} != produced batches {}",
                    result.payload.buffer_allocations, result.produced_batches
                )));
            }
        }
        MemoryModel::Slab => {
            if result.payload.pool_available_end != result.payload.pool_capacity {
                return Err(io::Error::other(format!(
                    "slab pool did not fully recover: available {} != capacity {}",
                    result.payload.pool_available_end, result.payload.pool_capacity
                )));
            }
            if result.payload.fallback_allocations != 0 || result.payload.discarded_buffers != 0 {
                return Err(io::Error::other(format!(
                    "slab pool bound failed: fallback allocations {}, discarded buffers {}",
                    result.payload.fallback_allocations, result.payload.discarded_buffers
                )));
            }
            if result.payload.buffer_reuses != result.produced_batches {
                return Err(io::Error::other(format!(
                    "slab reuse mismatch: reuses {} != produced batches {}",
                    result.payload.buffer_reuses, result.produced_batches
                )));
            }
        }
    }

    Ok(())
}

fn print_queue_result(name: &str, result: &QueueSnapshot) {
    println!(
        "{name}: accepted_batches={} processed_batches={} processed_packets={} \
         processed_bytes={} dropped_batches={} dropped_packets={} peak_depth={} \
         disconnected_events={}",
        result.accepted_batches,
        result.processed_batches,
        result.processed_packets,
        result.processed_bytes,
        result.dropped_batches,
        result.dropped_packets,
        result.peak_depth,
        result.disconnected_events
    );
}

fn print_result(config: &Config, result: &PipelineResult) {
    let throughput_mbps =
        result.produced_bytes as f64 * 8.0 / result.production_elapsed.as_secs_f64() / 1_000_000.0;
    let batch_bytes = config.batch_bytes();
    let queue_memory_upper_bound = batch_bytes
        .saturating_mul(config.queue_depth)
        .saturating_mul(PIPELINE_BRANCH_COUNT);
    let pipeline_memory_upper_bound = batch_bytes.saturating_mul(config.slab_pool_capacity());

    println!(
        "config: duration_seconds={} target_mbps={} batch_packets={} queue_depth={} \
         memory_model={} slow_player={}",
        config.duration_seconds,
        config.target_mbps,
        config.batch_packets,
        config.queue_depth,
        config.memory_model,
        config.slow_player
    );
    println!(
        "source: produced_batches={} produced_packets={} produced_bytes={} \
         throughput_mbps={throughput_mbps:.3}",
        result.produced_batches, result.produced_packets, result.produced_bytes
    );
    println!(
        "payload: buffer_allocations={} buffer_reuses={} pool_capacity={} \
         pool_available_end={} fallback_allocations={} discarded_buffers={}",
        result.payload.buffer_allocations,
        result.payload.buffer_reuses,
        result.payload.pool_capacity,
        result.payload.pool_available_end,
        result.payload.fallback_allocations,
        result.payload.discarded_buffers
    );
    print_queue_result("analyzer", &result.analyzer);
    print_queue_result("recorder", &result.recorder);
    print_queue_result("player", &result.player);
    println!(
        "bounds: queue_payload_bytes_upper_bound={} pipeline_payload_bytes_upper_bound={} \
         production_seconds={:.3} total_seconds={:.3}",
        queue_memory_upper_bound,
        pipeline_memory_upper_bound,
        result.production_elapsed.as_secs_f64(),
        result.total_elapsed.as_secs_f64()
    );
}

fn main() -> Result<(), Box<dyn Error>> {
    let Some(config) = parse_args()? else {
        return Ok(());
    };
    let result = run_pipeline(&config)?;

    print_result(&config, &result);
    validate_result(&config, &result)?;

    Ok(())
}
