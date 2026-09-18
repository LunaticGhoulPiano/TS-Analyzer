use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tsan_input::InputSourceKind;
use tsan_player::platform::windows::{
    GpuAdapterSelection, GstreamerAdapter, GstreamerDecoderIdentity, GstreamerMediaInfo,
    GstreamerPlayerBackend, PlayerBackendKind,
};
use tsan_player::{PlayerBackend, PlayerEvent, PlayerState, VideoRectangle, VideoSurface};

use crate::logging::{LogCategory, LogEntry, LogLevel};

const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(16);
const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(100);
const DURATION_SAMPLE_INTERVAL: Duration = Duration::from_millis(75);
const DURATION_STABILITY_TOLERANCE: Duration = Duration::from_millis(500);
const DURATION_UPDATE_TOLERANCE: Duration = Duration::from_secs(3);
const DURATION_CONFIRMATION_SAMPLES: u8 = 3;
const MAX_EVENT_LOG_ENTRIES: usize = 4_096;

pub enum PlayerCommand {
    Load {
        input: PathBuf,
        kind: PlayerBackendKind,
        adapter: GpuAdapterSelection,
    },
    Play,
    Pause,
    Stop,
    ResetInput,
    Seek(Duration),
    SetSurface {
        surface: VideoSurface,
        rectangle: VideoRectangle,
    },
    ClearSurface,
    Shutdown,
}

enum WorkerCommand {
    Player(PlayerCommand),
    ApplyLatestSeek,
}

#[derive(Default)]
struct LatestSeekRequest {
    position: Mutex<Option<Duration>>,
    notification_pending: AtomicBool,
}

impl LatestSeekRequest {
    fn replace(&self, position: Duration) -> Result<bool, String> {
        *self
            .position
            .lock()
            .map_err(|_| "latest seek request lock was poisoned".to_owned())? = Some(position);
        Ok(!self.notification_pending.swap(true, Ordering::AcqRel))
    }

    fn take(&self) -> Result<Option<Duration>, String> {
        self.notification_pending.store(false, Ordering::Release);
        Ok(self
            .position
            .lock()
            .map_err(|_| "latest seek request lock was poisoned".to_owned())?
            .take())
    }

    fn cancel_notification(&self) {
        self.notification_pending.store(false, Ordering::Release);
    }
}

#[derive(Default)]
struct StableDuration {
    candidate: Option<Duration>,
    candidate_samples: u8,
    accepted: Option<Duration>,
    last_sample: Option<Instant>,
}

impl StableDuration {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn observe(&mut self, value: Duration, now: Instant) -> Option<Duration> {
        if value.is_zero() {
            return self.accepted;
        }
        if self.last_sample.is_some_and(|last_sample| {
            now.saturating_duration_since(last_sample) < DURATION_SAMPLE_INTERVAL
        }) {
            return self.accepted;
        }
        self.last_sample = Some(now);

        if self
            .candidate
            .is_some_and(|candidate| candidate.abs_diff(value) <= DURATION_STABILITY_TOLERANCE)
        {
            self.candidate = Some(value);
            self.candidate_samples = self.candidate_samples.saturating_add(1);
        } else {
            self.candidate = Some(value);
            self.candidate_samples = 1;
        }

        if self.candidate_samples < DURATION_CONFIRMATION_SAMPLES {
            return self.accepted;
        }

        let plausible_update = match self.accepted {
            Some(accepted) => accepted.abs_diff(value) <= DURATION_UPDATE_TOLERANCE,
            None => true,
        };
        if plausible_update {
            self.accepted = Some(value);
        }
        self.accepted
    }
}

#[derive(Clone, Debug)]
pub struct PlayerSnapshot {
    pub state: PlayerState,
    pub kind: PlayerBackendKind,
    pub adapter: GpuAdapterSelection,
    pub d3d12_adapters: Vec<GstreamerAdapter>,
    pub d3d11_adapters: Vec<GstreamerAdapter>,
    pub input: Option<PathBuf>,
    pub source_kind: Option<InputSourceKind>,
    pub transport_packet_size: Option<usize>,
    pub decoder: Option<GstreamerDecoderIdentity>,
    pub media_info: GstreamerMediaInfo,
    pub video_pad_linked: bool,
    pub position: Option<Duration>,
    pub duration: Option<Duration>,
    pub warning_count: u64,
    pub qos_count: u64,
    pub events: Vec<LogEntry>,
    pub error: Option<String>,
}

impl Default for PlayerSnapshot {
    fn default() -> Self {
        Self {
            state: PlayerState::Idle,
            kind: PlayerBackendKind::D3d12,
            adapter: GpuAdapterSelection::Default,
            d3d12_adapters: Vec::new(),
            d3d11_adapters: Vec::new(),
            input: None,
            source_kind: None,
            transport_packet_size: None,
            decoder: None,
            media_info: GstreamerMediaInfo::default(),
            video_pad_linked: false,
            position: None,
            duration: None,
            warning_count: 0,
            qos_count: 0,
            events: Vec::new(),
            error: None,
        }
    }
}

pub struct PlayerWorkerHandle {
    command_sender: Sender<WorkerCommand>,
    snapshot_receiver: Receiver<PlayerSnapshot>,
    latest_seek: Arc<LatestSeekRequest>,
    worker: Option<JoinHandle<()>>,
}

impl PlayerWorkerHandle {
    pub fn spawn() -> Self {
        let (command_sender, command_receiver) = mpsc::channel();
        let (snapshot_sender, snapshot_receiver) = mpsc::channel();
        let latest_seek = Arc::new(LatestSeekRequest::default());
        let worker_latest_seek = Arc::clone(&latest_seek);
        let worker = thread::Builder::new()
            .name("tsan-player-worker".to_owned())
            .spawn(move || run_worker(command_receiver, snapshot_sender, worker_latest_seek))
            .ok();

        Self {
            command_sender,
            snapshot_receiver,
            latest_seek,
            worker,
        }
    }

    pub fn send(&self, command: PlayerCommand) -> Result<(), String> {
        if let PlayerCommand::Seek(position) = command {
            if !self.latest_seek.replace(position)? {
                return Ok(());
            }
            return self
                .command_sender
                .send(WorkerCommand::ApplyLatestSeek)
                .map_err(|error| {
                    self.latest_seek.cancel_notification();
                    format!("player worker is unavailable: {error}")
                });
        }
        self.command_sender
            .send(WorkerCommand::Player(command))
            .map_err(|error| format!("player worker is unavailable: {error}"))
    }

    pub fn latest_snapshot(&self) -> Option<PlayerSnapshot> {
        self.snapshot_receiver.try_iter().last()
    }
}

impl Drop for PlayerWorkerHandle {
    fn drop(&mut self) {
        let _send_result = self
            .command_sender
            .send(WorkerCommand::Player(PlayerCommand::Shutdown));
        if let Some(worker) = self.worker.take() {
            let _join_result = worker.join();
        }
    }
}

struct PlayerWorker {
    backend: Option<GstreamerPlayerBackend>,
    surface: Option<VideoSurface>,
    rectangle: Option<VideoRectangle>,
    snapshot: PlayerSnapshot,
    snapshot_sender: Sender<PlayerSnapshot>,
    next_event_sequence: u64,
    duration_tracker: StableDuration,
}

impl PlayerWorker {
    fn new(snapshot_sender: Sender<PlayerSnapshot>) -> Self {
        let mut snapshot = PlayerSnapshot::default();
        match GstreamerPlayerBackend::discover_adapters(PlayerBackendKind::D3d12) {
            Ok(adapters) => snapshot.d3d12_adapters = adapters,
            Err(error) => snapshot.events.push(LogEntry::new(
                1,
                LogLevel::Warning,
                LogCategory::Pipeline,
                format!("D3D12 unavailable: {error}"),
            )),
        }
        match GstreamerPlayerBackend::discover_adapters(PlayerBackendKind::D3d11) {
            Ok(adapters) => snapshot.d3d11_adapters = adapters,
            Err(error) => snapshot.events.push(LogEntry::new(
                2,
                LogLevel::Warning,
                LogCategory::Pipeline,
                format!("D3D11 unavailable: {error}"),
            )),
        }

        Self {
            backend: None,
            surface: None,
            rectangle: None,
            snapshot,
            snapshot_sender,
            next_event_sequence: 3,
            duration_tracker: StableDuration::default(),
        }
    }

    fn publish(&self) {
        let _send_result = self.snapshot_sender.send(self.snapshot.clone());
    }

    fn push_event(&mut self, level: LogLevel, category: LogCategory, message: impl Into<String>) {
        if self.snapshot.events.len() == MAX_EVENT_LOG_ENTRIES {
            self.snapshot.events.remove(0);
        }
        self.snapshot.events.push(LogEntry::new(
            self.next_event_sequence,
            level,
            category,
            message,
        ));
        self.next_event_sequence = self.next_event_sequence.saturating_add(1);
    }

    fn set_error(&mut self, category: LogCategory, context: &str, error: impl std::fmt::Display) {
        let message = format!("{context}: {error}");
        self.snapshot.error = Some(message.clone());
        self.snapshot.state = PlayerState::Failed;
        self.push_event(LogLevel::Error, category, message);
    }

    fn clear_error(&mut self) {
        self.snapshot.error = None;
    }

    fn sync_backend(&mut self) {
        let Some(backend) = self.backend.as_ref() else {
            return;
        };

        let previous_decoder = self.snapshot.decoder.clone();
        self.snapshot.state = backend.state();
        self.snapshot.kind = backend.kind();
        self.snapshot.adapter = backend.adapter_selection();
        self.snapshot.input = backend.input().map(PathBuf::from);
        self.snapshot.source_kind = backend
            .transport_stream_probe()
            .map(|_probe| InputSourceKind::TransportStreamFile);
        self.snapshot.transport_packet_size = backend
            .transport_stream_probe()
            .map(|probe| probe.packet_size());
        self.snapshot.decoder = backend.decoder_identity().cloned();
        self.snapshot.media_info = backend.media_info();
        self.snapshot.video_pad_linked = backend.is_video_pad_linked();
        if let Some(position) = backend.position() {
            self.snapshot.position = Some(position);
        }
        if let Some(duration) = backend.duration()
            && let Some(stable_duration) = self.duration_tracker.observe(duration, Instant::now())
        {
            self.snapshot.duration = Some(stable_duration);
        }
        if previous_decoder != self.snapshot.decoder
            && let Some(decoder) = self.snapshot.decoder.as_ref()
        {
            let decoder_name = decoder.name().to_owned();
            let decoder_factory = decoder.factory().to_owned();
            self.push_event(
                LogLevel::Info,
                LogCategory::Pipeline,
                format!("Linked decoder {decoder_name} ({decoder_factory})"),
            );
        }
    }

    fn handle_command(&mut self, command: PlayerCommand) -> bool {
        match command {
            PlayerCommand::Load {
                input,
                kind,
                adapter,
            } => self.load(input, kind, adapter),
            PlayerCommand::Play => self.play(),
            PlayerCommand::Pause => self.pause(),
            PlayerCommand::Stop => self.stop(),
            PlayerCommand::ResetInput => self.reset_input(),
            PlayerCommand::Seek(position) => self.seek(position),
            PlayerCommand::SetSurface { surface, rectangle } => {
                self.set_surface(surface, rectangle)
            }
            PlayerCommand::ClearSurface => self.clear_surface(),
            PlayerCommand::Shutdown => return false,
        }

        self.sync_backend();
        self.publish();
        true
    }

    fn load(&mut self, input: PathBuf, kind: PlayerBackendKind, adapter: GpuAdapterSelection) {
        self.backend = None;
        self.snapshot.kind = kind;
        self.snapshot.adapter = adapter;
        self.snapshot.input = Some(input.clone());
        self.snapshot.source_kind = None;
        self.snapshot.transport_packet_size = None;
        self.snapshot.decoder = None;
        self.snapshot.media_info = GstreamerMediaInfo::default();
        self.snapshot.video_pad_linked = false;
        self.snapshot.position = Some(Duration::ZERO);
        self.snapshot.duration = None;
        self.duration_tracker.reset();
        self.snapshot.warning_count = 0;
        self.snapshot.qos_count = 0;
        self.clear_error();

        let mut backend = match GstreamerPlayerBackend::new_on_adapter(kind, adapter) {
            Ok(backend) => backend,
            Err(error) => {
                self.set_error(
                    LogCategory::Pipeline,
                    "failed to create player backend",
                    error,
                );
                return;
            }
        };
        if let (Some(surface), Some(rectangle)) = (self.surface.clone(), self.rectangle)
            && let Err(error) = backend
                .set_video_surface(Some(surface))
                .and_then(|()| backend.set_video_rectangle(Some(rectangle)))
        {
            self.set_error(
                LogCategory::Pipeline,
                "failed to attach video surface",
                error,
            );
            return;
        }
        if let Err(error) = backend.load(&input) {
            self.set_error(LogCategory::Input, "failed to load input", error);
            return;
        }

        if let Some(probe) = backend.transport_stream_probe() {
            self.push_event(
                LogLevel::Info,
                LogCategory::Pipeline,
                format!(
                    "Prepared GStreamer pipeline for {}-byte MPEG-TS packets",
                    probe.packet_size()
                ),
            );
        }

        self.backend = Some(backend);
        self.push_event(
            LogLevel::Info,
            LogCategory::Input,
            format!("Loaded {} with {} on {}", input.display(), kind, adapter),
        );
    }

    fn play(&mut self) {
        let result = self.backend.as_mut().map(PlayerBackend::play);
        match result {
            Some(Ok(())) => {
                self.clear_error();
                self.push_event(LogLevel::Info, LogCategory::Playback, "Playback started");
            }
            Some(Err(error)) => {
                self.set_error(LogCategory::Playback, "failed to start playback", error)
            }
            None => self.set_error(
                LogCategory::Playback,
                "failed to start playback",
                "no input is loaded",
            ),
        }
    }

    fn pause(&mut self) {
        let result = self.backend.as_mut().map(PlayerBackend::pause);
        match result {
            Some(Ok(())) => {
                self.push_event(LogLevel::Info, LogCategory::Playback, "Playback paused")
            }
            Some(Err(error)) => {
                self.set_error(LogCategory::Playback, "failed to pause playback", error)
            }
            None => self.set_error(
                LogCategory::Playback,
                "failed to pause playback",
                "no player exists",
            ),
        }
    }

    fn stop(&mut self) {
        let result = self.backend.as_mut().map(PlayerBackend::stop);
        match result {
            Some(Ok(())) => {
                self.snapshot.position = Some(Duration::ZERO);
                self.push_event(LogLevel::Info, LogCategory::Playback, "Playback stopped");
            }
            Some(Err(error)) => {
                self.set_error(LogCategory::Playback, "failed to stop playback", error)
            }
            None => self.set_error(
                LogCategory::Playback,
                "failed to stop playback",
                "no player exists",
            ),
        }
    }

    fn reset_input(&mut self) {
        self.backend = None;
        self.snapshot.state = PlayerState::Idle;
        self.snapshot.input = None;
        self.snapshot.source_kind = None;
        self.snapshot.transport_packet_size = None;
        self.snapshot.decoder = None;
        self.snapshot.media_info = GstreamerMediaInfo::default();
        self.snapshot.video_pad_linked = false;
        self.snapshot.position = None;
        self.snapshot.duration = None;
        self.duration_tracker.reset();
        self.snapshot.warning_count = 0;
        self.snapshot.qos_count = 0;
        self.clear_error();
    }

    fn seek(&mut self, position: Duration) {
        let result = self.backend.as_mut().map(|backend| backend.seek(position));
        match result {
            Some(Ok(())) => {
                self.snapshot.position = Some(position);
                self.clear_error();
                self.push_event(
                    LogLevel::Info,
                    LogCategory::Playback,
                    format!("Seek requested for {:.3} seconds", position.as_secs_f64()),
                );
            }
            Some(Err(error)) => self.set_error(LogCategory::Playback, "failed to seek", error),
            None => self.set_error(LogCategory::Playback, "failed to seek", "no player exists"),
        }
    }

    fn set_surface(&mut self, surface: VideoSurface, rectangle: VideoRectangle) {
        let surface_changed = self
            .surface
            .as_ref()
            .is_none_or(|current| current.identity() != surface.identity());
        let rectangle_changed = self.rectangle != Some(rectangle);
        self.surface = Some(surface.clone());
        self.rectangle = Some(rectangle);

        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        let result = if surface_changed {
            backend
                .set_video_surface(Some(surface))
                .and_then(|()| backend.set_video_rectangle(Some(rectangle)))
        } else if rectangle_changed {
            backend.set_video_rectangle(Some(rectangle))
        } else {
            Ok(())
        };
        if let Err(error) = result {
            self.set_error(
                LogCategory::Pipeline,
                "failed to update video surface",
                error,
            );
        }
    }

    fn clear_surface(&mut self) {
        self.surface = None;
        self.rectangle = None;

        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        if let Err(error) = backend
            .set_video_rectangle(None)
            .and_then(|()| backend.set_video_surface(None))
        {
            self.set_error(
                LogCategory::Pipeline,
                "failed to clear video surface",
                error,
            );
        }
    }

    fn poll_events(&mut self) {
        for _ in 0..64 {
            let event = match self.backend.as_mut().map(PlayerBackend::poll_event) {
                Some(Ok(Some(event))) => event,
                Some(Ok(None)) | None => break,
                Some(Err(error)) => {
                    self.set_error(LogCategory::Pipeline, "failed to poll player event", error);
                    break;
                }
            };

            match event {
                PlayerEvent::EndOfStream => {
                    self.push_event(LogLevel::Info, LogCategory::Playback, "End of stream")
                }
                PlayerEvent::Warning(diagnostic) => {
                    self.snapshot.warning_count += 1;
                    self.push_event(
                        LogLevel::Warning,
                        LogCategory::Pipeline,
                        format_diagnostic("Warning", &diagnostic),
                    );
                }
                PlayerEvent::QualityOfService { source } => {
                    self.snapshot.qos_count += 1;
                    self.push_event(
                        LogLevel::Warning,
                        LogCategory::Pipeline,
                        format!("QoS event from {}", source.as_deref().unwrap_or("unknown")),
                    );
                }
                PlayerEvent::Error(diagnostic) => {
                    self.set_error(
                        LogCategory::Pipeline,
                        "player error",
                        format_diagnostic("Error", &diagnostic),
                    );
                }
            }
            self.sync_backend();
            self.publish();
        }
    }
}

fn run_worker(
    command_receiver: Receiver<WorkerCommand>,
    snapshot_sender: Sender<PlayerSnapshot>,
    latest_seek: Arc<LatestSeekRequest>,
) {
    let mut worker = PlayerWorker::new(snapshot_sender);
    let mut last_snapshot = Instant::now();
    worker.publish();

    loop {
        match command_receiver.recv_timeout(WORKER_POLL_INTERVAL) {
            Ok(WorkerCommand::Player(command)) => {
                if !worker.handle_command(command) {
                    break;
                }
            }
            Ok(WorkerCommand::ApplyLatestSeek) => match latest_seek.take() {
                Ok(Some(position)) => {
                    if !worker.handle_command(PlayerCommand::Seek(position)) {
                        break;
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    worker.set_error(LogCategory::Playback, "failed to apply latest seek", error)
                }
            },
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        worker.poll_events();
        if last_snapshot.elapsed() >= SNAPSHOT_INTERVAL {
            worker.sync_backend();
            worker.publish();
            last_snapshot = Instant::now();
        }
    }
}

fn format_diagnostic(prefix: &str, diagnostic: &tsan_player::PlayerDiagnostic) -> String {
    let source = diagnostic.source().unwrap_or("unknown");
    match diagnostic.debug() {
        Some(debug) => format!("{prefix} from {source}: {}; {debug}", diagnostic.message()),
        None => format!("{prefix} from {source}: {}", diagnostic.message()),
    }
}

#[cfg(test)]
mod tests {
    use super::{LatestSeekRequest, StableDuration};
    use std::time::{Duration, Instant};

    #[test]
    fn latest_seek_replaces_queued_targets() -> Result<(), String> {
        let latest = LatestSeekRequest::default();

        assert!(latest.replace(Duration::from_secs(10))?);
        assert!(!latest.replace(Duration::from_secs(20))?);
        assert!(!latest.replace(Duration::from_secs(30))?);
        assert_eq!(latest.take()?, Some(Duration::from_secs(30)));
        assert_eq!(latest.take()?, None);
        Ok(())
    }

    #[test]
    fn latest_seek_requests_a_new_notification_after_take() -> Result<(), String> {
        let latest = LatestSeekRequest::default();

        assert!(latest.replace(Duration::from_secs(10))?);
        assert_eq!(latest.take()?, Some(Duration::from_secs(10)));
        assert!(latest.replace(Duration::from_secs(20))?);
        assert_eq!(latest.take()?, Some(Duration::from_secs(20)));
        Ok(())
    }

    #[test]
    fn stable_duration_rejects_a_doubled_seek_artifact() {
        let mut tracker = StableDuration::default();
        let start = Instant::now();

        assert_eq!(tracker.observe(Duration::from_secs(121), start), None);
        assert_eq!(
            tracker.observe(Duration::from_secs(121), start + Duration::from_millis(75)),
            None
        );
        assert_eq!(
            tracker.observe(Duration::from_secs(121), start + Duration::from_millis(150)),
            Some(Duration::from_secs(121))
        );

        assert_eq!(
            tracker.observe(Duration::from_secs(242), start + Duration::from_millis(225)),
            Some(Duration::from_secs(121))
        );
        assert_eq!(
            tracker.observe(Duration::from_secs(242), start + Duration::from_millis(300)),
            Some(Duration::from_secs(121))
        );
        assert_eq!(
            tracker.observe(Duration::from_secs(242), start + Duration::from_millis(375)),
            Some(Duration::from_secs(121))
        );
    }
}
