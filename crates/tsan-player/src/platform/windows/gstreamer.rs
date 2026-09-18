//! Windows GStreamer playback through Direct3D hardware decoders and sinks.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ::gstreamer as gst;
use gst::prelude::*;
use gst_video::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use tsan_core::{TransportStreamTimeline, index_transport_stream};
use tsan_input::{TransportStreamProbe, probe_file_source};

use super::{GpuAdapterSelection, PlayerBackendKind, native_video_handle};
use crate::{
    PlayerBackend, PlayerDiagnostic, PlayerError, PlayerErrorKind, PlayerEvent, PlayerResult,
    PlayerState, ProgramSelection, VideoRectangle, VideoSurface,
};

const PTS_WRAP_NS: u64 = 95_443_717_688_888;
const MAX_FRAME_FORWARD_NS: u64 = 10_000_000_000;
const MAX_FRAME_REORDER_NS: u64 = 1_000_000_000;
const FEED_PACKETS_PER_BUFFER: usize = 256;
const TRANSPORT_PROBE_BYTES: usize = 64 * 1024;
const MIN_GSTREAMER_MAJOR: u32 = 1;
const MIN_GSTREAMER_MINOR: u32 = 28;
const SEEK_EOS_GUARD: Duration = Duration::from_secs(3);
const SEEK_SUPERSEDE_TIMEOUT: Duration = Duration::from_millis(250);
const SEEK_END_TOLERANCE: Duration = Duration::from_secs(2);
const MAX_SEEK_RECOVERY_ATTEMPTS: u8 = 1;
const COMMON_ELEMENTS: [&str; 10] = [
    "appsrc",
    "tsparse",
    "tsdemux",
    "queue",
    "h264parse",
    "h265parse",
    "decodebin3",
    "audioconvert",
    "audioresample",
    "wasapi2sink",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VideoCodec {
    H264,
    H265,
}

impl VideoCodec {
    const fn display_name(self) -> &'static str {
        match self {
            Self::H264 => "H.264 (AVC)",
            Self::H265 => "H.265 (HEVC)",
        }
    }

    const fn media_type(self) -> &'static str {
        match self {
            Self::H264 => "video/x-h264",
            Self::H265 => "video/x-h265",
        }
    }

    const fn parser_factory(self) -> &'static str {
        match self {
            Self::H264 => "h264parse",
            Self::H265 => "h265parse",
        }
    }

    const fn decoder_prefix(self, kind: PlayerBackendKind) -> &'static str {
        match (kind, self) {
            (PlayerBackendKind::D3d12, Self::H264) => "d3d12h264",
            (PlayerBackendKind::D3d12, Self::H265) => "d3d12h265",
            (PlayerBackendKind::D3d11, Self::H264) => "d3d11h264",
            (PlayerBackendKind::D3d11, Self::H265) => "d3d11h265",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BackendElements {
    sink: &'static str,
}

impl BackendElements {
    fn for_selection(kind: PlayerBackendKind) -> Self {
        match kind {
            PlayerBackendKind::D3d12 => Self {
                sink: "d3d12videosink",
            },
            PlayerBackendKind::D3d11 => Self {
                sink: "d3d11videosink",
            },
        }
    }

    fn decoder_factory(
        kind: PlayerBackendKind,
        codec: VideoCodec,
        adapter: GpuAdapterSelection,
    ) -> String {
        decoder_factory_name(codec.decoder_prefix(kind), adapter)
    }
}

#[derive(Clone, Default)]
struct VideoTarget {
    surface: Option<VideoSurface>,
    rectangle: Option<VideoRectangle>,
}

#[derive(Clone)]
struct VideoBranchContext {
    pipeline: gst::Pipeline,
    kind: PlayerBackendKind,
    adapter: GpuAdapterSelection,
    elements: BackendElements,
    video_overlay: Arc<Mutex<Option<gst_video::VideoOverlay>>>,
    decoder_identity: Arc<Mutex<Option<GstreamerDecoderIdentity>>>,
    media_info: Arc<Mutex<GstreamerMediaInfo>>,
    video_info_pad: Arc<Mutex<Option<gst::Pad>>>,
    video_target: Arc<Mutex<VideoTarget>>,
    position_tracker: Arc<Mutex<PlaybackPositionTracker>>,
    error_sender: SyncSender<PlayerError>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GstreamerAdapter {
    selection: GpuAdapterSelection,
    name: String,
    vendor_id: Option<u32>,
    device_id: Option<u32>,
    adapter_luid: Option<i64>,
}

impl GstreamerAdapter {
    pub const fn selection(&self) -> GpuAdapterSelection {
        self.selection
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn vendor_id(&self) -> Option<u32> {
        self.vendor_id
    }

    pub const fn device_id(&self) -> Option<u32> {
        self.device_id
    }

    pub const fn adapter_luid(&self) -> Option<i64> {
        self.adapter_luid
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GstreamerDecoderIdentity {
    factory: String,
    name: String,
    vendor_id: Option<u32>,
    device_id: Option<u32>,
    adapter_luid: Option<i64>,
}

impl GstreamerDecoderIdentity {
    pub fn factory(&self) -> &str {
        &self.factory
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn vendor_id(&self) -> Option<u32> {
        self.vendor_id
    }

    pub const fn device_id(&self) -> Option<u32> {
        self.device_id
    }

    pub const fn adapter_luid(&self) -> Option<i64> {
        self.adapter_luid
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GstreamerMediaInfo {
    video_codec: Option<String>,
    video_width: Option<u32>,
    video_height: Option<u32>,
    video_frame_rate: Option<(u32, u32)>,
    audio_codecs: Vec<String>,
    audio_tracks: u32,
}

impl GstreamerMediaInfo {
    pub fn video_codec(&self) -> Option<&str> {
        self.video_codec.as_deref()
    }

    pub const fn video_width(&self) -> Option<u32> {
        self.video_width
    }

    pub const fn video_height(&self) -> Option<u32> {
        self.video_height
    }

    pub const fn video_frame_rate(&self) -> Option<(u32, u32)> {
        self.video_frame_rate
    }

    pub fn audio_codecs(&self) -> &[String] {
        &self.audio_codecs
    }

    pub const fn audio_tracks(&self) -> u32 {
        self.audio_tracks
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GstreamerAvailability {
    kind: PlayerBackendKind,
    adapter: GpuAdapterSelection,
    version: (u32, u32, u32, u32),
    missing_elements: Vec<String>,
}

impl GstreamerAvailability {
    pub const fn kind(&self) -> PlayerBackendKind {
        self.kind
    }

    pub const fn adapter_selection(&self) -> GpuAdapterSelection {
        self.adapter
    }

    pub const fn version(&self) -> (u32, u32, u32, u32) {
        self.version
    }

    pub fn missing_elements(&self) -> &[String] {
        &self.missing_elements
    }

    pub const fn meets_minimum_version(&self) -> bool {
        self.version.0 > MIN_GSTREAMER_MAJOR
            || (self.version.0 == MIN_GSTREAMER_MAJOR && self.version.1 >= MIN_GSTREAMER_MINOR)
    }

    pub const fn is_available(&self) -> bool {
        self.meets_minimum_version() && self.missing_elements.is_empty()
    }
}

struct PipelineContext {
    pipeline: gst::Pipeline,
    source_error_receiver: Receiver<PlayerError>,
    video_overlay: Arc<Mutex<Option<gst_video::VideoOverlay>>>,
    dynamic_decoder_identity: Arc<Mutex<Option<GstreamerDecoderIdentity>>>,
    media_info: Arc<Mutex<GstreamerMediaInfo>>,
    video_info_pad: Arc<Mutex<Option<gst::Pad>>>,
    video_target: Arc<Mutex<VideoTarget>>,
    video_pad_linked: Arc<AtomicBool>,
    transport_probe: TransportStreamProbe,
}

#[derive(Clone, Copy)]
struct ActiveSeek {
    seqnum: gst::Seqnum,
    target: Duration,
    source_position: Duration,
    source_offset: u64,
    source_reset_received: bool,
    issued_at: Instant,
    recovery_attempts: u8,
    frame_received: bool,
}

#[derive(Default)]
struct PlaybackPositionTracker {
    active_seek: Option<ActiveSeek>,
    last_raw_pts_ns: Option<u64>,
    stable_position: Duration,
    last_frame_position: Option<Duration>,
}

impl PlaybackPositionTracker {
    fn begin_seek(
        &mut self,
        seqnum: gst::Seqnum,
        target: Duration,
        source_position: Duration,
        source_offset: u64,
        recovery_attempts: u8,
    ) -> Option<ActiveSeek> {
        self.last_raw_pts_ns = None;
        self.active_seek.replace(ActiveSeek {
            seqnum,
            target,
            source_position,
            source_offset,
            source_reset_received: false,
            issued_at: Instant::now(),
            recovery_attempts,
            frame_received: false,
        })
    }

    fn rollback_seek(&mut self, seqnum: gst::Seqnum, previous: Option<ActiveSeek>) {
        if self
            .active_seek
            .is_some_and(|active| active.seqnum == seqnum)
        {
            self.active_seek = previous;
        }
    }

    fn observe_source_seek(&mut self, offset: u64) {
        let Some(active) = self.active_seek.as_mut() else {
            return;
        };
        if active.source_offset == offset {
            active.source_reset_received = true;
        }
    }

    fn observe_async_done(&self, seqnum: gst::Seqnum) -> bool {
        self.active_seek
            .is_some_and(|active| active.seqnum == seqnum)
    }

    fn observe_frame(&mut self, pts: Option<gst::ClockTime>) -> bool {
        let Some(pts) = pts else {
            return !self.is_seek_in_flight();
        };
        let raw_pts_ns = pts.nseconds();
        let seek_position = self.active_seek.and_then(|active| {
            (!active.frame_received
                && self.last_raw_pts_ns.is_none()
                && active.source_reset_received)
                .then_some(active.source_position)
        });
        if self.active_seek.is_some() && self.last_raw_pts_ns.is_none() && seek_position.is_none() {
            return false;
        }

        if let Some(position) = seek_position {
            self.last_raw_pts_ns = Some(raw_pts_ns);
            self.stable_position = position;
            self.last_frame_position = Some(position);
            let reached_target = self
                .active_seek
                .is_none_or(|active| position >= active.target);
            if let Some(active) = self.active_seek.as_mut() {
                active.frame_received = reached_target;
            }
            return reached_target;
        }
        let Some(previous_raw_pts_ns) = self.last_raw_pts_ns else {
            self.last_raw_pts_ns = Some(raw_pts_ns);
            self.stable_position = Duration::ZERO;
            self.last_frame_position = Some(Duration::ZERO);
            return true;
        };
        let forward = (raw_pts_ns + PTS_WRAP_NS - previous_raw_pts_ns) % PTS_WRAP_NS;
        let backward = (previous_raw_pts_ns + PTS_WRAP_NS - raw_pts_ns) % PTS_WRAP_NS;
        if forward <= MAX_FRAME_FORWARD_NS {
            self.stable_position = self
                .stable_position
                .saturating_add(Duration::from_nanos(forward));
            self.last_raw_pts_ns = Some(raw_pts_ns);
            self.last_frame_position = Some(self.stable_position);
        } else if backward <= MAX_FRAME_REORDER_NS {
            self.last_frame_position = Some(
                self.stable_position
                    .saturating_sub(Duration::from_nanos(backward)),
            );
        }
        let reached_target = self.active_seek.is_some_and(|active| {
            self.last_frame_position
                .is_some_and(|position| position >= active.target)
        });
        if reached_target && let Some(active) = self.active_seek.as_mut() {
            active.frame_received = true;
        }
        !self.is_seek_in_flight()
    }

    fn frame_position(&self, duration: Option<Duration>) -> Option<Duration> {
        let position = self
            .active_seek
            .filter(|active| !active.frame_received)
            .map(|active| active.target)
            .or(self.last_frame_position)?;
        Some(duration.map_or(position, |duration| position.min(duration)))
    }

    fn is_seek_in_flight(&self) -> bool {
        self.active_seek
            .is_some_and(|active| !active.frame_received)
    }

    fn can_start_pending_seek(&self) -> bool {
        self.active_seek
            .is_none_or(|active| active.issued_at.elapsed() >= SEEK_SUPERSEDE_TIMEOUT)
    }

    fn is_stale_eos(&self, seqnum: gst::Seqnum, pending_seek: bool) -> bool {
        if pending_seek {
            return true;
        }
        self.active_seek.is_some_and(|active| {
            !active.frame_received
                || (active.seqnum != seqnum && active.issued_at.elapsed() <= SEEK_EOS_GUARD)
        })
    }

    fn eos_recovery(
        &self,
        seqnum: gst::Seqnum,
        duration: Option<Duration>,
    ) -> Option<(Duration, u8)> {
        let active = self.active_seek?;
        let duration = duration?;
        if active.seqnum != seqnum
            || active.recovery_attempts >= MAX_SEEK_RECOVERY_ATTEMPTS
            || active.issued_at.elapsed() > SEEK_EOS_GUARD
            || active.target.saturating_add(SEEK_END_TOLERANCE) >= duration
        {
            return None;
        }
        Some((active.target, active.recovery_attempts.saturating_add(1)))
    }
}

pub struct GstreamerPlayerBackend {
    kind: PlayerBackendKind,
    adapter: GpuAdapterSelection,
    state: PlayerState,
    selection: ProgramSelection,
    input: Option<PathBuf>,
    pipeline: Option<PipelineContext>,
    surface: Option<VideoSurface>,
    video_rectangle: Option<VideoRectangle>,
    decoder_identity: Option<GstreamerDecoderIdentity>,
    transport_probe: Option<TransportStreamProbe>,
    timeline: Option<TransportStreamTimeline>,
    position_tracker: Arc<Mutex<PlaybackPositionTracker>>,
    pending_seek: Option<Duration>,
}

impl GstreamerPlayerBackend {
    pub const fn kind(&self) -> PlayerBackendKind {
        self.kind
    }

    pub const fn adapter_selection(&self) -> GpuAdapterSelection {
        self.adapter
    }

    pub fn probe(kind: PlayerBackendKind) -> PlayerResult<GstreamerAvailability> {
        Self::probe_adapter(kind, GpuAdapterSelection::Default)
    }

    pub fn probe_adapter(
        kind: PlayerBackendKind,
        adapter: GpuAdapterSelection,
    ) -> PlayerResult<GstreamerAvailability> {
        gst::init().map_err(|error| {
            PlayerError::new(
                PlayerErrorKind::Unavailable,
                format!("failed to initialize GStreamer: {error}"),
            )
        })?;

        let backend_elements = BackendElements::for_selection(kind);
        let mut missing_elements = Vec::new();

        for element in COMMON_ELEMENTS {
            if gst::ElementFactory::find(element).is_none() {
                missing_elements.push(element.to_owned());
            }
        }
        let h264_decoder = BackendElements::decoder_factory(kind, VideoCodec::H264, adapter);
        let h265_decoder = BackendElements::decoder_factory(kind, VideoCodec::H265, adapter);
        for element in [
            h264_decoder.as_str(),
            h265_decoder.as_str(),
            backend_elements.sink,
        ] {
            if gst::ElementFactory::find(element).is_none() {
                missing_elements.push(element.to_owned());
            }
        }

        Ok(GstreamerAvailability {
            kind,
            adapter,
            version: gst::version(),
            missing_elements,
        })
    }

    pub fn new(kind: PlayerBackendKind) -> PlayerResult<Self> {
        Self::new_on_adapter(kind, GpuAdapterSelection::Default)
    }

    pub fn new_on_adapter(
        kind: PlayerBackendKind,
        adapter: GpuAdapterSelection,
    ) -> PlayerResult<Self> {
        let availability = Self::probe_adapter(kind, adapter)?;
        if !availability.meets_minimum_version() {
            let version = availability.version();
            return Err(PlayerError::new(
                PlayerErrorKind::Unavailable,
                format!(
                    "GStreamer {}.{}.{}.{} is older than required version {}.{}",
                    version.0,
                    version.1,
                    version.2,
                    version.3,
                    MIN_GSTREAMER_MAJOR,
                    MIN_GSTREAMER_MINOR
                ),
            ));
        }
        if !availability.missing_elements().is_empty() {
            return Err(PlayerError::new(
                PlayerErrorKind::Unavailable,
                format!(
                    "GStreamer backend '{}' on {} is missing elements: {}",
                    kind,
                    adapter,
                    availability.missing_elements().join(", ")
                ),
            ));
        }

        Ok(Self {
            kind,
            adapter,
            state: PlayerState::Idle,
            selection: ProgramSelection::Automatic,
            input: None,
            pipeline: None,
            surface: None,
            video_rectangle: None,
            decoder_identity: None,
            transport_probe: None,
            position_tracker: Arc::new(Mutex::new(PlaybackPositionTracker::default())),
            timeline: None,
            pending_seek: None,
        })
    }

    pub fn discover_adapters(kind: PlayerBackendKind) -> PlayerResult<Vec<GstreamerAdapter>> {
        gst::init().map_err(|error| {
            PlayerError::new(
                PlayerErrorKind::Unavailable,
                format!("failed to initialize GStreamer: {error}"),
            )
        })?;

        let mut decoder_factories = gst::ElementFactory::factories_with_type(
            gst::ElementFactoryType::DECODER | gst::ElementFactoryType::MEDIA_VIDEO,
            gst::Rank::NONE,
        )
        .into_iter()
        .filter_map(|factory| {
            let factory_name = factory.name();
            decoder_factory_adapter_index(kind, factory_name.as_str()).map(|index| (index, factory))
        })
        .collect::<Vec<_>>();
        decoder_factories.sort_by_key(|(index, _factory)| *index);

        let mut adapters = Vec::with_capacity(decoder_factories.len());
        for (index, factory) in decoder_factories {
            let factory_name = factory.name().to_string();
            let decoder = factory
                .create()
                .name("adapter-probe")
                .build()
                .map_err(|error| element_creation_error(&factory_name, error))?;
            let identity = decoder_identity(&decoder, &factory_name);

            adapters.push(GstreamerAdapter {
                selection: GpuAdapterSelection::Index(index),
                name: identity.name,
                vendor_id: identity.vendor_id,
                device_id: identity.device_id,
                adapter_luid: identity.adapter_luid,
            });
        }

        if adapters.is_empty() {
            return Err(PlayerError::new(
                PlayerErrorKind::Unavailable,
                format!("no H.265 hardware decoder was found for backend '{kind}'"),
            ));
        }
        Ok(adapters)
    }

    pub fn input(&self) -> Option<&Path> {
        self.input.as_deref()
    }

    pub fn decoder_identity(&self) -> Option<&GstreamerDecoderIdentity> {
        self.decoder_identity.as_ref()
    }

    pub fn media_info(&self) -> GstreamerMediaInfo {
        let Some(context) = self.pipeline.as_ref() else {
            return GstreamerMediaInfo::default();
        };
        let mut media_info = context
            .media_info
            .lock()
            .map(|media_info| media_info.clone())
            .unwrap_or_default();
        if let Ok(video_info_pad) = context.video_info_pad.lock()
            && let Some(video_info_pad) = video_info_pad.as_ref()
            && let Some(caps) = video_info_pad.current_caps()
        {
            update_video_info_from_caps(&mut media_info, caps.as_ref());
        }
        media_info
    }

    pub const fn transport_stream_probe(&self) -> Option<TransportStreamProbe> {
        self.transport_probe
    }

    pub fn is_video_pad_linked(&self) -> bool {
        self.pipeline
            .as_ref()
            .is_some_and(|context| context.video_pad_linked.load(Ordering::Acquire))
    }

    fn build_pipeline(
        &self,
        input: &Path,
        selection: ProgramSelection,
    ) -> PlayerResult<PipelineContext> {
        let mut input_file = File::open(input).map_err(|error| {
            PlayerError::new(
                PlayerErrorKind::InvalidInput,
                format!("failed to open '{}': {error}", input.display()),
            )
        })?;
        let transport_probe = probe_input(&mut input_file, input)?;

        let elements = BackendElements::for_selection(self.kind);
        let (source_error_sender, source_error_receiver) = mpsc::sync_channel(8);
        let input_size = i64::try_from(
            input_file
                .metadata()
                .map_err(|error| {
                    PlayerError::new(
                        PlayerErrorKind::InvalidInput,
                        format!("failed to inspect '{}': {error}", input.display()),
                    )
                })?
                .len(),
        )
        .map_err(|error| {
            PlayerError::new(
                PlayerErrorKind::InvalidInput,
                format!("input '{}' is too large: {error}", input.display()),
            )
        })?;
        let source_caps = gst::Caps::builder("video/mpegts")
            .field("systemstream", true)
            .field(
                "packetsize",
                i32::try_from(transport_probe.packet_size()).unwrap_or(188),
            )
            .build();
        let source_element = gst::ElementFactory::make("appsrc")
            .name("source")
            .property("caps", source_caps)
            .property("format", gst::Format::Bytes)
            .property("stream-type", gst_app::AppStreamType::RandomAccess)
            .property("size", input_size)
            .property("block", true)
            .property("max-bytes", 32_u64 * 1024 * 1024)
            .build()
            .map_err(|error| element_creation_error("appsrc", error))?;
        let app_src = source_element
            .clone()
            .downcast::<gst_app::AppSrc>()
            .map_err(|_| {
                PlayerError::new(
                    PlayerErrorKind::BackendFailure,
                    "appsrc element has an unexpected runtime type",
                )
            })?;
        configure_app_source(
            &app_src,
            input_file,
            input.display().to_string(),
            transport_probe.packet_size(),
            transport_probe.stream_start_offset() as u64,
            source_error_sender.clone(),
            Arc::clone(&self.position_tracker),
        );

        let source_queue = create_playback_queue("source-queue")?;
        let ts_parse = gst::ElementFactory::make("tsparse")
            .name("ts-parse")
            .property("split-on-rai", true)
            .property("ignore-pcr", true)
            .property("skew-corrections", false)
            .build()
            .map_err(|error| element_creation_error("tsparse", error))?;
        let program_number = match selection {
            ProgramSelection::Automatic => -1,
            ProgramSelection::Program(number) => i32::from(number.get()),
        };
        let demux = gst::ElementFactory::make("tsdemux")
            .name("demux")
            .property("program-number", program_number)
            .property("latency", 0_i32)
            .property("skew-corrections", false)
            .build()
            .map_err(|error| element_creation_error("tsdemux", error))?;
        let pipeline = gst::Pipeline::builder().name("tsan-player").build();
        pipeline
            .add_many([&source_element, &source_queue, &ts_parse, &demux])
            .map_err(|error| backend_failure("failed to add elements to pipeline", error))?;
        gst::Element::link_many([&source_element, &source_queue, &ts_parse, &demux])
            .map_err(|error| backend_failure("failed to link transport elements", error))?;
        let video_overlay = Arc::new(Mutex::new(None));
        let dynamic_decoder_identity = Arc::new(Mutex::new(None));
        let media_info = Arc::new(Mutex::new(GstreamerMediaInfo::default()));
        let video_info_pad = Arc::new(Mutex::new(None));
        let video_target = Arc::new(Mutex::new(VideoTarget {
            surface: self.surface.clone(),
            rectangle: self.video_rectangle,
        }));
        let video_pad_linked = connect_video_pad(
            &demux,
            VideoBranchContext {
                pipeline: pipeline.clone(),
                kind: self.kind,
                adapter: self.adapter,
                elements,
                video_overlay: Arc::clone(&video_overlay),
                decoder_identity: Arc::clone(&dynamic_decoder_identity),
                media_info: Arc::clone(&media_info),
                video_info_pad: Arc::clone(&video_info_pad),
                video_target: Arc::clone(&video_target),
                position_tracker: Arc::clone(&self.position_tracker),
                error_sender: source_error_sender.clone(),
            },
        );
        connect_audio_pad(
            &pipeline,
            &demux,
            Arc::clone(&media_info),
            source_error_sender,
        );

        Ok(PipelineContext {
            pipeline,
            source_error_receiver,
            video_overlay,
            dynamic_decoder_identity,
            media_info,
            video_info_pad,
            video_target,
            video_pad_linked,
            transport_probe,
        })
    }

    fn ensure_stopped_pipeline(&mut self) -> PlayerResult<()> {
        if self.pipeline.is_some() {
            return Ok(());
        }

        let input = self.input.clone().ok_or_else(|| {
            PlayerError::new(PlayerErrorKind::InvalidState, "no input has been loaded")
        })?;
        let pipeline = self.build_pipeline(&input, self.selection)?;
        self.decoder_identity = None;
        self.transport_probe = Some(pipeline.transport_probe);
        self.pipeline = Some(pipeline);
        Ok(())
    }

    fn set_pipeline_state(&mut self, target: gst::State) -> PlayerResult<()> {
        let context = self.pipeline.as_ref().ok_or_else(|| {
            PlayerError::new(
                PlayerErrorKind::InvalidState,
                "player pipeline is not available",
            )
        })?;

        context
            .pipeline
            .set_state(target)
            .map_err(|error| backend_failure("failed to change pipeline state", error))?;
        Ok(())
    }

    fn wait_for_pipeline_state(
        &self,
        target: gst::State,
        timeout: gst::ClockTime,
    ) -> PlayerResult<()> {
        let context = self.pipeline.as_ref().ok_or_else(|| {
            PlayerError::new(
                PlayerErrorKind::InvalidState,
                "player pipeline is not available",
            )
        })?;
        let (result, current, pending) = context.pipeline.state(timeout);
        result.map_err(|error| {
            backend_failure(
                &format!(
                    "pipeline did not reach {target:?} (current: {current:?}, pending: {pending:?})"
                ),
                error,
            )
        })?;
        if current != target {
            return Err(PlayerError::new(
                PlayerErrorKind::BackendFailure,
                format!(
                    "pipeline did not reach {target:?} (current: {current:?}, pending: {pending:?})"
                ),
            ));
        }
        Ok(())
    }

    fn shutdown_pipeline(&mut self) -> PlayerResult<()> {
        let state_result = self
            .pipeline
            .as_ref()
            .map(|context| context.pipeline.set_state(gst::State::Null));
        self.pipeline = None;
        self.position_tracker = Arc::new(Mutex::new(PlaybackPositionTracker::default()));
        self.pending_seek = None;

        if let Some(Err(error)) = state_result {
            return Err(backend_failure("failed to stop player pipeline", error));
        }
        Ok(())
    }

    fn start_seek(&mut self, position: Duration) -> PlayerResult<()> {
        self.start_seek_with_recovery(position, 0)
    }

    fn start_seek_with_recovery(
        &mut self,
        position: Duration,
        recovery_attempts: u8,
    ) -> PlayerResult<()> {
        let context = self.pipeline.as_ref().ok_or_else(|| {
            PlayerError::new(
                PlayerErrorKind::InvalidState,
                "player pipeline is not available",
            )
        })?;
        let seek_point = self
            .timeline
            .as_ref()
            .and_then(|timeline| timeline.seek_point(position))
            .ok_or_else(|| {
                PlayerError::new(
                    PlayerErrorKind::BackendFailure,
                    "transport stream does not contain an indexed video seek point",
                )
            })?;
        let seqnum = gst::Seqnum::next();
        let seek_event = gst::event::Seek::builder(
            1.0,
            gst::SeekFlags::FLUSH,
            gst::SeekType::Set,
            gst::format::Bytes::from_bytes(seek_point.byte_offset()),
            gst::SeekType::None,
            gst::format::Bytes::NONE,
        )
        .seqnum(seqnum)
        .build();
        let previous = self
            .position_tracker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .begin_seek(
                seqnum,
                position,
                seek_point.position(),
                seek_point.byte_offset(),
                recovery_attempts,
            );
        if !context.pipeline.send_event(seek_event) {
            self.position_tracker
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .rollback_seek(seqnum, previous);
            return Err(PlayerError::new(
                PlayerErrorKind::BackendFailure,
                "player pipeline rejected the seek event",
            ));
        }
        Ok(())
    }

    fn apply_pending_seek_if_ready(&mut self) -> PlayerResult<()> {
        let can_start = self
            .position_tracker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .can_start_pending_seek();
        if !can_start {
            return Ok(());
        }
        let Some(position) = self.pending_seek.take() else {
            return Ok(());
        };
        self.start_seek(position)
    }

    fn invalid_state(&self, operation: &str) -> PlayerError {
        PlayerError::new(
            PlayerErrorKind::InvalidState,
            format!("cannot {operation} while player is {:?}", self.state),
        )
    }

    fn take_source_error(&self) -> Option<PlayerError> {
        self.pipeline
            .as_ref()?
            .source_error_receiver
            .try_recv()
            .ok()
    }

    fn sync_dynamic_decoder_identity(&mut self) -> PlayerResult<()> {
        let Some(context) = self.pipeline.as_ref() else {
            return Ok(());
        };
        let identity = context.dynamic_decoder_identity.lock().map_err(|_| {
            PlayerError::new(
                PlayerErrorKind::BackendFailure,
                "decoder identity lock was poisoned",
            )
        })?;
        if let Some(identity) = identity.as_ref() {
            self.decoder_identity = Some(identity.clone());
        }
        Ok(())
    }

    fn handle_end_of_stream(&mut self) -> PlayerResult<Option<PlayerEvent>> {
        self.shutdown_pipeline()?;
        self.state = PlayerState::Stopped;
        Ok(Some(PlayerEvent::EndOfStream))
    }

    fn handle_pipeline_error(
        &mut self,
        diagnostic: PlayerDiagnostic,
    ) -> PlayerResult<Option<PlayerEvent>> {
        let _shutdown_result = self.shutdown_pipeline();
        self.state = PlayerState::Failed;
        Ok(Some(PlayerEvent::Error(diagnostic)))
    }
}

impl PlayerBackend for GstreamerPlayerBackend {
    fn state(&self) -> PlayerState {
        self.state
    }

    fn program_selection(&self) -> ProgramSelection {
        self.selection
    }

    fn load(&mut self, input: &Path) -> PlayerResult<()> {
        if input.as_os_str().is_empty() || !input.is_file() {
            return Err(PlayerError::new(
                PlayerErrorKind::InvalidInput,
                format!("input is not a file: {}", input.display()),
            ));
        }

        if let Err(error) = self.shutdown_pipeline() {
            self.state = PlayerState::Failed;
            return Err(error);
        }
        self.input = None;
        self.decoder_identity = None;
        self.transport_probe = None;
        self.timeline = None;

        match self.build_pipeline(input, self.selection) {
            Ok(pipeline) => {
                self.timeline = File::open(input)
                    .ok()
                    .and_then(|file| index_transport_stream(file, pipeline.transport_probe).ok())
                    .flatten();
                self.input = Some(input.to_path_buf());
                self.decoder_identity = None;
                self.transport_probe = Some(pipeline.transport_probe);
                self.pipeline = Some(pipeline);
                self.state = PlayerState::Stopped;
                Ok(())
            }
            Err(error) => {
                self.state = PlayerState::Failed;
                Err(error)
            }
        }
    }

    fn play(&mut self) -> PlayerResult<()> {
        match self.state {
            PlayerState::Stopped => {
                if let Err(error) = self.ensure_stopped_pipeline() {
                    self.state = PlayerState::Failed;
                    return Err(error);
                }
                if let Err(error) = self.set_pipeline_state(gst::State::Paused) {
                    self.state = PlayerState::Failed;
                    return Err(error);
                }
                if let Err(error) = self
                    .wait_for_pipeline_state(gst::State::Paused, gst::ClockTime::from_seconds(5))
                {
                    self.state = PlayerState::Failed;
                    return Err(error);
                }
                if let Err(error) = self.set_pipeline_state(gst::State::Playing) {
                    self.state = PlayerState::Failed;
                    return Err(error);
                }
            }
            PlayerState::Paused => {
                if let Err(error) = self.set_pipeline_state(gst::State::Playing) {
                    self.state = PlayerState::Failed;
                    return Err(error);
                }
            }
            _ => return Err(self.invalid_state("play")),
        }

        self.state = PlayerState::Playing;
        Ok(())
    }

    fn pause(&mut self) -> PlayerResult<()> {
        if self.state != PlayerState::Playing {
            return Err(self.invalid_state("pause"));
        }

        if let Err(error) = self.set_pipeline_state(gst::State::Paused) {
            self.state = PlayerState::Failed;
            return Err(error);
        }
        self.state = PlayerState::Paused;
        Ok(())
    }

    fn stop(&mut self) -> PlayerResult<()> {
        if !matches!(
            self.state,
            PlayerState::Stopped | PlayerState::Playing | PlayerState::Paused | PlayerState::Failed
        ) {
            return Err(self.invalid_state("stop"));
        }

        if let Err(error) = self.shutdown_pipeline() {
            self.state = PlayerState::Failed;
            return Err(error);
        }
        self.state = if self.input.is_some() {
            PlayerState::Stopped
        } else {
            PlayerState::Idle
        };
        Ok(())
    }

    fn position(&self) -> Option<Duration> {
        self.pipeline.as_ref()?;
        let tracker = self
            .position_tracker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Some(
            tracker
                .frame_position(self.duration())
                .unwrap_or(Duration::ZERO),
        )
    }

    fn duration(&self) -> Option<Duration> {
        self.timeline
            .as_ref()
            .map(TransportStreamTimeline::duration)
    }

    fn seek(&mut self, position: Duration) -> PlayerResult<()> {
        if !matches!(self.state, PlayerState::Playing | PlayerState::Paused) {
            return Err(self.invalid_state("seek"));
        }
        let position = self
            .duration()
            .map_or(position, |duration| position.min(duration));
        self.pending_seek = Some(position);
        self.apply_pending_seek_if_ready()
    }

    fn select_program(&mut self, selection: ProgramSelection) -> PlayerResult<()> {
        if matches!(self.state, PlayerState::Playing | PlayerState::Paused) {
            return Err(self.invalid_state("select a program"));
        }
        if selection == self.selection {
            return Ok(());
        }

        if let Err(error) = self.shutdown_pipeline() {
            self.state = PlayerState::Failed;
            return Err(error);
        }

        let Some(input) = self.input.clone() else {
            self.selection = selection;
            return Ok(());
        };

        match self.build_pipeline(&input, selection) {
            Ok(pipeline) => {
                self.selection = selection;
                self.decoder_identity = None;
                self.transport_probe = Some(pipeline.transport_probe);
                self.pipeline = Some(pipeline);
                self.state = PlayerState::Stopped;
                Ok(())
            }
            Err(error) => {
                self.state = PlayerState::Failed;
                Err(error)
            }
        }
    }

    fn set_video_surface(&mut self, surface: Option<VideoSurface>) -> PlayerResult<()> {
        if let Some(context) = self.pipeline.as_ref() {
            context
                .video_target
                .lock()
                .map_err(|_| video_target_lock_error())?
                .surface = surface.clone();
            if let Some(overlay) = context
                .video_overlay
                .lock()
                .map_err(|_| video_overlay_lock_error())?
                .as_ref()
            {
                set_overlay_surface(overlay, surface.as_ref())?;
            }
        }
        self.surface = surface;
        Ok(())
    }

    fn set_video_rectangle(&mut self, rectangle: Option<VideoRectangle>) -> PlayerResult<()> {
        if let Some(context) = self.pipeline.as_ref() {
            context
                .video_target
                .lock()
                .map_err(|_| video_target_lock_error())?
                .rectangle = rectangle;
            if let (Some(overlay), Some(rectangle)) = (
                context
                    .video_overlay
                    .lock()
                    .map_err(|_| video_overlay_lock_error())?
                    .as_ref(),
                rectangle,
            ) {
                set_overlay_rectangle(overlay, rectangle)?;
            }
        }
        self.video_rectangle = rectangle;
        Ok(())
    }

    fn poll_event(&mut self) -> PlayerResult<Option<PlayerEvent>> {
        self.apply_pending_seek_if_ready()?;
        self.sync_dynamic_decoder_identity()?;
        if let Some(error) = self.take_source_error() {
            let diagnostic =
                PlayerDiagnostic::new(Some("dynamic pipeline".to_owned()), error.message(), None);
            return self.handle_pipeline_error(diagnostic);
        }

        let Some(bus) = self
            .pipeline
            .as_ref()
            .and_then(|context| context.pipeline.bus())
        else {
            return Ok(None);
        };

        while let Some(message) = bus.pop() {
            match message.view() {
                gst::MessageView::AsyncDone(_) => {
                    let completed = self
                        .position_tracker
                        .lock()
                        .map(|tracker| tracker.observe_async_done(message.seqnum()))
                        .unwrap_or(false);
                    if completed {
                        self.apply_pending_seek_if_ready()?;
                    }
                }
                gst::MessageView::Eos(_) => {
                    if let Some(position) = self.pending_seek.take() {
                        if let Err(error) = self.start_seek(position) {
                            self.pending_seek = Some(position);
                            return Err(error);
                        }
                        continue;
                    }

                    let duration = self.duration();
                    let recovery = self
                        .position_tracker
                        .lock()
                        .map(|tracker| tracker.eos_recovery(message.seqnum(), duration))
                        .unwrap_or(None);
                    if let Some((position, recovery_attempts)) = recovery {
                        self.start_seek_with_recovery(position, recovery_attempts)?;
                        continue;
                    }

                    let stale = self
                        .position_tracker
                        .lock()
                        .map(|tracker| {
                            tracker.is_stale_eos(message.seqnum(), self.pending_seek.is_some())
                        })
                        .unwrap_or(false);
                    if stale {
                        continue;
                    }
                    return self.handle_end_of_stream();
                }
                gst::MessageView::Warning(warning) => {
                    return Ok(Some(PlayerEvent::Warning(PlayerDiagnostic::new(
                        message_source(message.as_ref()),
                        warning.error().to_string(),
                        warning.debug().map(|debug| debug.to_string()),
                    ))));
                }
                gst::MessageView::Qos(_) => {
                    return Ok(Some(PlayerEvent::QualityOfService {
                        source: message_source(message.as_ref()),
                    }));
                }
                gst::MessageView::Error(error) => {
                    let diagnostic = PlayerDiagnostic::new(
                        message_source(message.as_ref()),
                        error.error().to_string(),
                        error.debug().map(|debug| debug.to_string()),
                    );
                    return self.handle_pipeline_error(diagnostic);
                }
                _ => {}
            }
        }

        Ok(None)
    }
}

impl Drop for GstreamerPlayerBackend {
    fn drop(&mut self) {
        let _shutdown_result = self.shutdown_pipeline();
    }
}

fn create_element(factory_name: &str, element_name: &str) -> PlayerResult<gst::Element> {
    gst::ElementFactory::make(factory_name)
        .name(element_name)
        .build()
        .map_err(|error| element_creation_error(factory_name, error))
}

fn create_playback_queue(element_name: &str) -> PlayerResult<gst::Element> {
    gst::ElementFactory::make("queue")
        .name(element_name)
        .property("silent", true)
        .build()
        .map_err(|error| element_creation_error("queue", error))
}

fn create_video_sink(
    kind: PlayerBackendKind,
    adapter: GpuAdapterSelection,
    factory_name: &str,
) -> PlayerResult<gst::Element> {
    let adapter_index = match adapter {
        GpuAdapterSelection::Default => -1,
        GpuAdapterSelection::Index(index) => i32::try_from(index).map_err(|error| {
            PlayerError::new(
                PlayerErrorKind::InvalidInput,
                format!("GPU adapter index {index} is too large: {error}"),
            )
        })?,
    };
    let builder = gst::ElementFactory::make(factory_name)
        .name("video-sink")
        .property("force-aspect-ratio", true)
        .property("adapter", adapter_index);
    let result = match kind {
        PlayerBackendKind::D3d12 => builder
            .property("fullscreen-on-alt-enter", false)
            .property("external-window-only", true)
            .property("direct-swapchain", true)
            .build(),
        PlayerBackendKind::D3d11 => builder
            .property_from_str("fullscreen-toggle-mode", "none")
            .build(),
    };

    let sink = result.map_err(|error| element_creation_error(factory_name, error))?;
    if sink.find_property("error-on-closed").is_some() {
        sink.set_property("error-on-closed", false);
    }
    Ok(sink)
}

fn decoder_factory_name(prefix: &str, adapter: GpuAdapterSelection) -> String {
    match adapter {
        GpuAdapterSelection::Default | GpuAdapterSelection::Index(0) => format!("{prefix}dec"),
        GpuAdapterSelection::Index(index) => format!("{prefix}device{index}dec"),
    }
}

fn decoder_factory_adapter_index(kind: PlayerBackendKind, factory_name: &str) -> Option<u32> {
    let prefix = match kind {
        PlayerBackendKind::D3d12 => "d3d12h265",
        PlayerBackendKind::D3d11 => "d3d11h265",
    };
    let adapter = factory_name.strip_prefix(prefix)?.strip_suffix("dec")?;
    if adapter.is_empty() {
        return Some(0);
    }

    adapter.strip_prefix("device")?.parse().ok()
}

fn decoder_identity(decoder: &gst::Element, factory_name: &str) -> GstreamerDecoderIdentity {
    let name = decoder
        .factory()
        .and_then(|factory| factory.metadata("long-name").map(ToString::to_string))
        .unwrap_or_else(|| factory_name.to_owned());

    GstreamerDecoderIdentity {
        factory: factory_name.to_owned(),
        name,
        vendor_id: optional_u32_property(decoder, "vendor-id"),
        device_id: optional_u32_property(decoder, "device-id"),
        adapter_luid: optional_i64_property(decoder, "adapter-luid"),
    }
}

fn optional_u32_property(element: &gst::Element, name: &str) -> Option<u32> {
    element
        .find_property(name)
        .map(|_| element.property::<u32>(name))
}

fn optional_i64_property(element: &gst::Element, name: &str) -> Option<i64> {
    element
        .find_property(name)
        .map(|_| element.property::<i64>(name))
}

fn set_overlay_rectangle(
    overlay: &gst_video::VideoOverlay,
    rectangle: VideoRectangle,
) -> PlayerResult<()> {
    overlay
        .set_render_rectangle(
            rectangle.x(),
            rectangle.y(),
            rectangle.width(),
            rectangle.height(),
        )
        .map_err(|error| backend_failure("failed to set video render rectangle", error))
}

#[allow(unsafe_code)]
fn set_overlay_surface(
    overlay: &gst_video::VideoOverlay,
    surface: Option<&VideoSurface>,
) -> PlayerResult<()> {
    let native_handle = surface.map(native_video_handle).transpose()?.unwrap_or(0);

    // SAFETY: VideoSurface retains shared ownership of the window for as long as the
    // handle can be used by the backend. A zero handle explicitly detaches the overlay.
    unsafe {
        overlay.set_window_handle(native_handle);
    }

    Ok(())
}

fn connect_video_pad(demux: &gst::Element, context: VideoBranchContext) -> Arc<AtomicBool> {
    let video_pad_linked = Arc::new(AtomicBool::new(false));
    let linked_for_callback = Arc::clone(&video_pad_linked);

    demux.connect_pad_added(move |_demux, source_pad| {
        let Some(codec) = video_codec_for_pad(source_pad) else {
            return;
        };
        if linked_for_callback
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        if let Ok(mut media_info) = context.media_info.lock() {
            media_info.video_codec = Some(codec.display_name().to_owned());
        }
        if let Err(error) = create_and_link_video_branch(
            &context.pipeline,
            source_pad,
            context.kind,
            context.adapter,
            codec,
            &context.elements,
            &context.video_overlay,
            &context.decoder_identity,
            &context.video_info_pad,
            &context.video_target,
            Arc::clone(&context.position_tracker),
        ) {
            linked_for_callback.store(false, Ordering::Release);
            report_source_error(&context.error_sender, error);
        }
    });

    video_pad_linked
}

fn connect_audio_pad(
    pipeline: &gst::Pipeline,
    demux: &gst::Element,
    media_info: Arc<Mutex<GstreamerMediaInfo>>,
    error_sender: SyncSender<PlayerError>,
) {
    let audio_pad_linked = Arc::new(AtomicBool::new(false));
    let pipeline = pipeline.clone();

    demux.connect_pad_added(move |_demux, source_pad| {
        if !is_audio_pad(source_pad) {
            return;
        }
        if let Ok(mut media_info) = media_info.lock() {
            media_info.audio_tracks = media_info.audio_tracks.saturating_add(1);
            if let Some(codec) = audio_codec_for_pad(source_pad)
                && !media_info.audio_codecs.iter().any(|value| value == codec)
            {
                media_info.audio_codecs.push(codec.to_owned());
            }
        }
        if audio_pad_linked
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        if let Err(error) =
            create_and_link_audio_branch(&pipeline, source_pad, error_sender.clone())
        {
            audio_pad_linked.store(false, Ordering::Release);
            report_source_error(&error_sender, error);
        }
    });
}

fn is_audio_pad(pad: &gst::Pad) -> bool {
    let caps = pad.current_caps().unwrap_or_else(|| pad.query_caps(None));
    caps.structure(0)
        .is_some_and(|structure| structure.name().starts_with("audio/"))
}

fn audio_codec_for_pad(pad: &gst::Pad) -> Option<&'static str> {
    let caps = pad.current_caps().unwrap_or_else(|| pad.query_caps(None));
    let structure = caps.structure(0)?;
    match structure.name().as_str() {
        "audio/x-ac3" => Some("AC-3"),
        "audio/x-eac3" => Some("E-AC-3"),
        "audio/x-dts" => Some("DTS"),
        "audio/x-opus" => Some("Opus"),
        "audio/mpeg" => match structure.get::<i32>("mpegversion").ok() {
            Some(4) => Some("AAC"),
            Some(1) if structure.get::<i32>("layer").ok() == Some(3) => Some("MP3"),
            Some(1) => Some("MPEG Audio"),
            _ => Some("MPEG Audio"),
        },
        _ => None,
    }
}

fn create_and_link_audio_branch(
    pipeline: &gst::Pipeline,
    source_pad: &gst::Pad,
    error_sender: SyncSender<PlayerError>,
) -> PlayerResult<()> {
    let audio_queue = create_playback_queue("audio-queue")?;
    let audio_decoder = create_element("decodebin3", "audio-decoder")?;
    let audio_convert = create_element("audioconvert", "audio-convert")?;
    let audio_resample = create_element("audioresample", "audio-resample")?;
    let audio_sink = create_element("wasapi2sink", "audio-sink")?;
    let convert_sink_pad = audio_convert.static_pad("sink").ok_or_else(|| {
        PlayerError::new(
            PlayerErrorKind::BackendFailure,
            "audio converter does not expose a static sink pad",
        )
    })?;
    let decoded_audio_linked = Arc::new(AtomicBool::new(false));

    audio_decoder.connect_pad_added(move |_decoder, decoded_pad| {
        if !is_raw_audio_pad(decoded_pad) {
            return;
        }
        if decoded_audio_linked
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        if let Err(error) = decoded_pad.link(&convert_sink_pad) {
            decoded_audio_linked.store(false, Ordering::Release);
            report_source_error(
                &error_sender,
                backend_failure("failed to link decoded audio", error),
            );
        }
    });

    pipeline
        .add_many([
            &audio_queue,
            &audio_decoder,
            &audio_convert,
            &audio_resample,
            &audio_sink,
        ])
        .map_err(|error| backend_failure("failed to add audio branch to pipeline", error))?;
    audio_queue
        .link(&audio_decoder)
        .map_err(|error| backend_failure("failed to link audio queue and decoder", error))?;
    gst::Element::link_many([&audio_convert, &audio_resample, &audio_sink])
        .map_err(|error| backend_failure("failed to link audio output elements", error))?;
    let queue_sink_pad = audio_queue.static_pad("sink").ok_or_else(|| {
        PlayerError::new(
            PlayerErrorKind::BackendFailure,
            "audio queue does not expose a static sink pad",
        )
    })?;
    source_pad
        .link(&queue_sink_pad)
        .map_err(|error| backend_failure("failed to link demuxed audio", error))?;
    for element in [
        &audio_queue,
        &audio_decoder,
        &audio_convert,
        &audio_resample,
        &audio_sink,
    ] {
        element
            .sync_state_with_parent()
            .map_err(|error| backend_failure("failed to activate audio element", error))?;
    }
    Ok(())
}

fn is_raw_audio_pad(pad: &gst::Pad) -> bool {
    let caps = pad.current_caps().unwrap_or_else(|| pad.query_caps(None));
    caps.structure(0)
        .is_some_and(|structure| structure.name() == "audio/x-raw")
}

fn video_codec_for_pad(pad: &gst::Pad) -> Option<VideoCodec> {
    let caps = pad.current_caps().unwrap_or_else(|| pad.query_caps(None));
    match caps.structure(0)?.name().as_str() {
        "video/x-h264" => Some(VideoCodec::H264),
        "video/x-h265" => Some(VideoCodec::H265),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn create_and_link_video_branch(
    pipeline: &gst::Pipeline,
    source_pad: &gst::Pad,
    kind: PlayerBackendKind,
    adapter: GpuAdapterSelection,
    codec: VideoCodec,
    elements: &BackendElements,
    video_overlay: &Mutex<Option<gst_video::VideoOverlay>>,
    dynamic_decoder_identity: &Mutex<Option<GstreamerDecoderIdentity>>,
    video_info_pad: &Mutex<Option<gst::Pad>>,
    video_target: &Mutex<VideoTarget>,
    position_tracker: Arc<Mutex<PlaybackPositionTracker>>,
) -> PlayerResult<()> {
    let video_queue = create_playback_queue("video-queue")?;
    let video_parser = create_element(codec.parser_factory(), "video-parser")?;
    let decoder_factory = BackendElements::decoder_factory(kind, codec, adapter);
    let decoder = create_element(&decoder_factory, "video-decoder")?;
    let decoder_source_pad = decoder.static_pad("src").ok_or_else(|| {
        PlayerError::new(
            PlayerErrorKind::BackendFailure,
            "video decoder does not expose a static source pad",
        )
    })?;
    let identity = decoder_identity(&decoder, &decoder_factory);
    let video_sink = create_video_sink(kind, adapter, elements.sink)?;
    let overlay = video_sink
        .clone()
        .dynamic_cast::<gst_video::VideoOverlay>()
        .map_err(|_| {
            PlayerError::new(
                PlayerErrorKind::Unavailable,
                format!(
                    "GStreamer element '{}' does not support VideoOverlay",
                    elements.sink
                ),
            )
        })?;
    let target = video_target
        .lock()
        .map_err(|_| video_target_lock_error())?
        .clone();
    set_overlay_surface(&overlay, target.surface.as_ref())?;
    if let Some(rectangle) = target.rectangle {
        set_overlay_rectangle(&overlay, rectangle)?;
    }

    pipeline
        .add_many([&video_queue, &video_parser, &decoder, &video_sink])
        .map_err(|error| backend_failure("failed to add video branch to pipeline", error))?;
    gst::Element::link_many([&video_queue, &video_parser, &decoder, &video_sink])
        .map_err(|error| backend_failure("failed to link video elements", error))?;
    let queue_sink_pad = video_queue.static_pad("sink").ok_or_else(|| {
        PlayerError::new(
            PlayerErrorKind::BackendFailure,
            "video queue does not expose a static sink pad",
        )
    })?;
    source_pad.link(&queue_sink_pad).map_err(|error| {
        backend_failure(
            &format!(
                "failed to link {} pad '{}' to video queue",
                codec.media_type(),
                source_pad.name()
            ),
            error,
        )
    })?;
    for element in [&video_queue, &video_parser, &decoder, &video_sink] {
        element
            .sync_state_with_parent()
            .map_err(|error| backend_failure("failed to activate video element", error))?;
    }

    let sink_pad = video_sink.static_pad("sink").ok_or_else(|| {
        PlayerError::new(
            PlayerErrorKind::BackendFailure,
            "video sink does not expose a static sink pad",
        )
    })?;
    let _probe_id = sink_pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
        if let Some(buffer) = info.buffer()
            && let Ok(mut tracker) = position_tracker.lock()
        {
            let _target_reached = tracker.observe_frame(buffer.pts());
        }
        // The real D3D sink must receive the preroll frames between the indexed
        // random-access point and the requested presentation time. Dropping them
        // here can leave its post-flush preroll incomplete, so the pipeline stays
        // nominally Playing while the displayed frame never advances.
        gst::PadProbeReturn::Ok
    });

    *video_overlay
        .lock()
        .map_err(|_| video_overlay_lock_error())? = Some(overlay);
    *dynamic_decoder_identity.lock().map_err(|_| {
        PlayerError::new(
            PlayerErrorKind::BackendFailure,
            "decoder identity lock was poisoned",
        )
    })? = Some(identity);
    *video_info_pad.lock().map_err(|_| {
        PlayerError::new(
            PlayerErrorKind::BackendFailure,
            "video information pad lock was poisoned",
        )
    })? = Some(decoder_source_pad);
    Ok(())
}

fn update_video_info_from_caps(media_info: &mut GstreamerMediaInfo, caps: &gst::CapsRef) {
    let Some(structure) = caps.structure(0) else {
        return;
    };
    media_info.video_width = structure
        .get::<i32>("width")
        .ok()
        .and_then(|value| u32::try_from(value).ok());
    media_info.video_height = structure
        .get::<i32>("height")
        .ok()
        .and_then(|value| u32::try_from(value).ok());
    media_info.video_frame_rate =
        structure
            .get::<gst::Fraction>("framerate")
            .ok()
            .and_then(|rate| {
                let numerator = u32::try_from(rate.numer()).ok()?;
                let denominator = u32::try_from(rate.denom()).ok()?;
                (denominator != 0).then_some((numerator, denominator))
            });
}

fn configure_app_source(
    app_src: &gst_app::AppSrc,
    input_file: File,
    input_name: String,
    packet_size: usize,
    stream_start_offset: u64,
    error_sender: SyncSender<PlayerError>,
    position_tracker: Arc<Mutex<PlaybackPositionTracker>>,
) {
    let input_file = Arc::new(Mutex::new(input_file));
    let read_file = Arc::clone(&input_file);
    let seek_file = input_file;
    let read_error_sender = error_sender.clone();
    let read_input_name = input_name.clone();

    app_src.set_callbacks(
        gst_app::AppSrcCallbacks::builder()
            .need_data(move |source, requested_bytes| {
                match read_source_buffer(&read_file, requested_bytes, packet_size, &read_input_name)
                {
                    Ok(Some(buffer)) => {
                        if let Err(error) = source.push_buffer(buffer) {
                            report_source_error(
                                &read_error_sender,
                                backend_failure("appsrc failed to push a TS buffer", error),
                            );
                        }
                    }
                    Ok(None) => {
                        if let Err(error) = source.end_of_stream() {
                            report_source_error(
                                &read_error_sender,
                                backend_failure("appsrc failed to signal end of stream", error),
                            );
                        }
                    }
                    Err(error) => {
                        report_source_error(&read_error_sender, error);
                        let _end_of_stream_result = source.end_of_stream();
                    }
                }
            })
            .seek_data(move |_source, offset| {
                let result = seek_source_file(
                    &seek_file,
                    offset,
                    packet_size,
                    stream_start_offset,
                    &input_name,
                );
                if let Err(error) = result {
                    report_source_error(&error_sender, error);
                    return false;
                }
                if let Ok(mut tracker) = position_tracker.lock() {
                    tracker.observe_source_seek(offset);
                }
                true
            })
            .build(),
    );
}

fn read_source_buffer(
    input_file: &Mutex<File>,
    requested_bytes: u32,
    packet_size: usize,
    input_name: &str,
) -> PlayerResult<Option<gst::Buffer>> {
    let maximum_bytes = packet_size * FEED_PACKETS_PER_BUFFER;
    let requested_bytes = usize::try_from(requested_bytes).unwrap_or(maximum_bytes);
    let requested_bytes = requested_bytes.clamp(packet_size, maximum_bytes);
    let requested_bytes = requested_bytes - requested_bytes % packet_size;
    let mut bytes = vec![0_u8; requested_bytes];
    let mut file = input_file.lock().map_err(|_| {
        PlayerError::new(
            PlayerErrorKind::BackendFailure,
            format!("file lock for '{input_name}' was poisoned"),
        )
    })?;
    let offset = file.stream_position().map_err(|error| {
        PlayerError::new(
            PlayerErrorKind::BackendFailure,
            format!("failed to query position in '{input_name}': {error}"),
        )
    })?;
    let bytes_read = file.read(&mut bytes).map_err(|error| {
        PlayerError::new(
            PlayerErrorKind::BackendFailure,
            format!("failed to read '{input_name}': {error}"),
        )
    })?;

    if bytes_read == 0 {
        return Ok(None);
    }

    bytes.truncate(bytes_read);
    let mut buffer = gst::Buffer::from_mut_slice(bytes);
    if let Some(buffer) = buffer.get_mut() {
        buffer.set_offset(offset);
        buffer.set_offset_end(offset + bytes_read as u64);
    }
    Ok(Some(buffer))
}

fn seek_source_file(
    input_file: &Mutex<File>,
    offset: u64,
    packet_size: usize,
    stream_start_offset: u64,
    input_name: &str,
) -> PlayerResult<()> {
    let packet_size = packet_size as u64;
    let aligned_offset = if offset <= stream_start_offset {
        offset
    } else {
        stream_start_offset + (offset - stream_start_offset) / packet_size * packet_size
    };
    let mut file = input_file.lock().map_err(|_| {
        PlayerError::new(
            PlayerErrorKind::BackendFailure,
            format!("file lock for '{input_name}' was poisoned"),
        )
    })?;
    file.seek(SeekFrom::Start(aligned_offset))
        .map_err(|error| {
            PlayerError::new(
                PlayerErrorKind::BackendFailure,
                format!("failed to seek '{input_name}': {error}"),
            )
        })?;
    Ok(())
}

fn probe_input(input_file: &mut File, input: &Path) -> PlayerResult<TransportStreamProbe> {
    let mut sample = Vec::with_capacity(TRANSPORT_PROBE_BYTES);
    input_file
        .by_ref()
        .take(TRANSPORT_PROBE_BYTES as u64)
        .read_to_end(&mut sample)
        .map_err(|error| {
            PlayerError::new(
                PlayerErrorKind::InvalidInput,
                format!("failed to inspect '{}': {error}", input.display()),
            )
        })?;
    input_file.seek(SeekFrom::Start(0)).map_err(|error| {
        PlayerError::new(
            PlayerErrorKind::InvalidInput,
            format!("failed to rewind '{}': {error}", input.display()),
        )
    })?;

    probe_file_source(&sample)
        .map(|source| source.transport_stream())
        .map_err(|error| {
            PlayerError::new(
                PlayerErrorKind::InvalidInput,
                format!(
                    "'{}' is not a playable MPEG transport stream: {error}",
                    input.display()
                ),
            )
        })
}

fn report_source_error(error_sender: &SyncSender<PlayerError>, error: PlayerError) {
    let _send_result = error_sender.try_send(error);
}

fn element_creation_error(error_element: &str, error: impl std::fmt::Display) -> PlayerError {
    PlayerError::new(
        PlayerErrorKind::Unavailable,
        format!("failed to create GStreamer element '{error_element}': {error}"),
    )
}

fn backend_failure(context: &str, error: impl std::fmt::Display) -> PlayerError {
    PlayerError::new(
        PlayerErrorKind::BackendFailure,
        format!("{context}: {error}"),
    )
}

fn video_target_lock_error() -> PlayerError {
    PlayerError::new(
        PlayerErrorKind::BackendFailure,
        "video target lock was poisoned",
    )
}

fn video_overlay_lock_error() -> PlayerError {
    PlayerError::new(
        PlayerErrorKind::BackendFailure,
        "video overlay lock was poisoned",
    )
}

fn message_source(message: &gst::MessageRef) -> Option<String> {
    message.src().map(|source| source.path_string().to_string())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::super::{GpuAdapterSelection, PlayerBackendKind};
    use super::{
        BackendElements, GstreamerPlayerBackend, PlaybackPositionTracker, VideoCodec,
        decoder_factory_adapter_index, decoder_factory_name, gst,
    };

    #[test]
    fn imports_external_ts_files_without_playback() -> Result<(), Box<dyn std::error::Error>> {
        use std::path::PathBuf;

        use crate::PlayerBackend;

        let Ok(path) = std::env::var("TSAN_TS_SAMPLE") else {
            return Ok(());
        };
        let path = PathBuf::from(path);
        let mut paths = if path.is_dir() {
            std::fs::read_dir(path)?
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<std::io::Result<Vec<_>>>()?
        } else {
            vec![path]
        };
        paths.retain(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("ts"))
        });
        paths.sort();
        assert!(!paths.is_empty(), "no TS files were found");

        let mut player = GstreamerPlayerBackend::new(PlayerBackendKind::D3d12)?;
        for path in paths {
            player.load(&path)?;
            let duration = player.duration();
            eprintln!("imported {}: {duration:?}", path.display());
            assert!(
                duration.is_some(),
                "{} has no indexed duration",
                path.display()
            );
        }
        Ok(())
    }

    fn assert_send<T: Send>() {}

    #[test]
    fn backend_is_send() {
        assert_send::<GstreamerPlayerBackend>();
    }

    #[test]
    fn seek_waits_for_the_indexed_source_reset_and_target_frame() {
        let mut tracker = PlaybackPositionTracker::default();
        tracker.observe_frame(Some(gst::ClockTime::from_seconds(2)));
        assert_eq!(tracker.frame_position(None), Some(Duration::ZERO));

        let seqnum = gst::Seqnum::next();
        tracker.begin_seek(
            seqnum,
            Duration::from_millis(46_500),
            Duration::from_secs(45),
            1_880,
            0,
        );
        assert!(tracker.is_seek_in_flight());
        assert!(tracker.is_stale_eos(seqnum, false));
        tracker.observe_source_seek(3_760);
        assert!(!tracker.observe_frame(Some(gst::ClockTime::from_seconds(3))));
        tracker.observe_source_seek(1_880);
        assert!(tracker.is_seek_in_flight());
        assert!(!tracker.observe_frame(Some(gst::ClockTime::from_mseconds(48_500))));
        assert!(tracker.is_seek_in_flight());
        assert!(tracker.is_stale_eos(seqnum, false));
        assert_eq!(
            tracker.frame_position(Some(Duration::from_secs(100))),
            Some(Duration::from_millis(46_500))
        );
        assert!(!tracker.observe_frame(Some(gst::ClockTime::from_mseconds(48_600))));
        assert_eq!(
            tracker.frame_position(None),
            Some(Duration::from_millis(46_500))
        );
        assert!(tracker.observe_frame(Some(gst::ClockTime::from_mseconds(50_000))));
        assert!(!tracker.is_seek_in_flight());
        assert!(!tracker.can_start_pending_seek());
        assert!(!tracker.is_stale_eos(seqnum, false));
        assert_eq!(
            tracker.frame_position(None),
            Some(Duration::from_millis(46_500))
        );
    }

    #[test]
    fn frame_timeline_ignores_old_source_resets_and_clamps_duration() {
        let mut tracker = PlaybackPositionTracker::default();
        tracker.observe_frame(Some(gst::ClockTime::from_seconds(2)));

        let first_seqnum = gst::Seqnum::next();
        tracker.begin_seek(
            first_seqnum,
            Duration::from_secs(80),
            Duration::from_secs(79),
            1_880,
            0,
        );
        assert!(tracker.observe_async_done(first_seqnum));
        assert!(!tracker.observe_frame(Some(gst::ClockTime::from_seconds(82))));
        tracker.observe_source_seek(1_880);
        assert!(!tracker.observe_frame(Some(gst::ClockTime::from_seconds(82))));
        assert_eq!(
            tracker.frame_position(Some(Duration::from_secs(90))),
            Some(Duration::from_secs(80))
        );
        assert!(tracker.observe_frame(Some(gst::ClockTime::from_seconds(83))));

        let second_seqnum = gst::Seqnum::next();
        tracker.begin_seek(
            second_seqnum,
            Duration::from_secs(20),
            Duration::from_secs(19),
            3_760,
            0,
        );
        tracker.observe_source_seek(1_880);
        assert!(tracker.is_seek_in_flight());
        assert!(tracker.observe_async_done(second_seqnum));
        assert!(!tracker.observe_frame(Some(gst::ClockTime::from_seconds(22))));
        tracker.observe_source_seek(3_760);
        assert!(!tracker.observe_frame(Some(gst::ClockTime::from_seconds(22))));
        assert_eq!(
            tracker.frame_position(Some(Duration::from_secs(90))),
            Some(Duration::from_secs(20))
        );
        assert!(tracker.observe_frame(Some(gst::ClockTime::from_seconds(23))));
        tracker.observe_frame(Some(gst::ClockTime::from_seconds(24)));
        assert_eq!(
            tracker.frame_position(Some(Duration::from_secs(20))),
            Some(Duration::from_secs(20))
        );
        assert!(tracker.is_stale_eos(first_seqnum, false));
        assert!(!tracker.is_stale_eos(second_seqnum, false));
    }

    #[test]
    fn stuck_seek_can_be_superseded_after_timeout() {
        let mut tracker = PlaybackPositionTracker::default();
        let seqnum = gst::Seqnum::next();
        tracker.begin_seek(
            seqnum,
            Duration::from_secs(20),
            Duration::from_secs(19),
            0,
            0,
        );
        assert!(!tracker.can_start_pending_seek());

        if let Some(active) = tracker.active_seek.as_mut() {
            active.issued_at = Instant::now() - super::SEEK_SUPERSEDE_TIMEOUT;
        }
        assert!(tracker.can_start_pending_seek());
    }

    #[test]
    fn backend_elements_are_explicit() {
        let d3d12 = BackendElements::for_selection(PlayerBackendKind::D3d12);
        assert_eq!(
            BackendElements::decoder_factory(
                PlayerBackendKind::D3d12,
                VideoCodec::H264,
                GpuAdapterSelection::Default,
            ),
            "d3d12h264dec"
        );
        assert_eq!(
            BackendElements::decoder_factory(
                PlayerBackendKind::D3d12,
                VideoCodec::H265,
                GpuAdapterSelection::Default,
            ),
            "d3d12h265dec"
        );
        assert_eq!(d3d12.sink, "d3d12videosink");

        let d3d11 = BackendElements::for_selection(PlayerBackendKind::D3d11);
        assert_eq!(
            BackendElements::decoder_factory(
                PlayerBackendKind::D3d11,
                VideoCodec::H264,
                GpuAdapterSelection::Default,
            ),
            "d3d11h264dec"
        );
        assert_eq!(
            BackendElements::decoder_factory(
                PlayerBackendKind::D3d11,
                VideoCodec::H265,
                GpuAdapterSelection::Default,
            ),
            "d3d11h265dec"
        );
        assert_eq!(d3d11.sink, "d3d11videosink");
    }

    #[test]
    fn adapter_decoder_factory_names_match_gstreamer_registration() {
        assert_eq!(
            decoder_factory_name("d3d12h265", GpuAdapterSelection::Index(0)),
            "d3d12h265dec"
        );
        assert_eq!(
            decoder_factory_name("d3d12h265", GpuAdapterSelection::Index(1)),
            "d3d12h265device1dec"
        );
        assert_eq!(
            decoder_factory_name("d3d11h265", GpuAdapterSelection::Index(1)),
            "d3d11h265device1dec"
        );
    }

    #[test]
    fn adapter_indices_are_parsed_from_registered_factory_names() {
        assert_eq!(
            decoder_factory_adapter_index(PlayerBackendKind::D3d12, "d3d12h265dec"),
            Some(0)
        );
        assert_eq!(
            decoder_factory_adapter_index(PlayerBackendKind::D3d12, "d3d12h265device27dec"),
            Some(27)
        );
        assert_eq!(
            decoder_factory_adapter_index(PlayerBackendKind::D3d11, "d3d11h265device3dec"),
            Some(3)
        );
        assert_eq!(
            decoder_factory_adapter_index(PlayerBackendKind::D3d12, "d3d11h265dec"),
            None
        );
        assert_eq!(
            decoder_factory_adapter_index(PlayerBackendKind::D3d12, "d3d12h264dec"),
            None
        );
    }
}
