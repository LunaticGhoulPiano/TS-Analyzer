use std::any::Any;
use std::error::Error;
use std::fmt;
use std::num::NonZeroU16;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PlayerState {
    #[default]
    Idle,
    Stopped,
    Playing,
    Paused,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProgramNumber(NonZeroU16);

impl ProgramNumber {
    pub const fn new(value: u16) -> Option<Self> {
        match NonZeroU16::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn get(self) -> u16 {
        self.0.get()
    }
}

impl fmt::Display for ProgramNumber {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProgramSelection {
    #[default]
    Automatic,
    Program(ProgramNumber),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlayerErrorKind {
    Unavailable,
    InvalidInput,
    InvalidState,
    BackendFailure,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlayerDiagnostic {
    source: Option<String>,
    message: String,
    debug: Option<String>,
}

impl PlayerDiagnostic {
    pub fn new(source: Option<String>, message: impl Into<String>, debug: Option<String>) -> Self {
        Self {
            source,
            message: message.into(),
            debug,
        }
    }

    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn debug(&self) -> Option<&str> {
        self.debug.as_deref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlayerEvent {
    EndOfStream,
    Warning(PlayerDiagnostic),
    QualityOfService { source: Option<String> },
    Error(PlayerDiagnostic),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VideoRectangle {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl VideoRectangle {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Option<Self> {
        if width <= 0 || height <= 0 {
            return None;
        }

        Some(Self {
            x,
            y,
            width,
            height,
        })
    }

    pub const fn x(self) -> i32 {
        self.x
    }

    pub const fn y(self) -> i32 {
        self.y
    }

    pub const fn width(self) -> i32 {
        self.width
    }

    pub const fn height(self) -> i32 {
        self.height
    }
}

#[derive(Clone)]
pub struct VideoSurface {
    platform_surface: Arc<dyn Any + Send + Sync>,
}

impl VideoSurface {
    pub(crate) fn from_platform<T>(platform_surface: T) -> Self
    where
        T: Any + Send + Sync,
    {
        Self {
            platform_surface: Arc::new(platform_surface),
        }
    }

    pub(crate) fn platform_ref<T: Any>(&self) -> Option<&T> {
        self.platform_surface.downcast_ref()
    }

    pub fn identity(&self) -> usize {
        Arc::as_ptr(&self.platform_surface) as *const () as usize
    }
}

impl fmt::Debug for VideoSurface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VideoSurface")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlayerError {
    kind: PlayerErrorKind,
    message: String,
}

impl PlayerError {
    pub fn new(kind: PlayerErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub const fn kind(&self) -> PlayerErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for PlayerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for PlayerError {}

pub type PlayerResult<T> = Result<T, PlayerError>;

pub trait PlayerBackend: Send {
    fn state(&self) -> PlayerState;

    fn program_selection(&self) -> ProgramSelection;

    fn load(&mut self, input: &Path) -> PlayerResult<()>;

    fn play(&mut self) -> PlayerResult<()>;

    fn pause(&mut self) -> PlayerResult<()>;

    fn stop(&mut self) -> PlayerResult<()>;

    fn position(&self) -> Option<Duration>;

    fn duration(&self) -> Option<Duration>;

    fn seek(&mut self, position: Duration) -> PlayerResult<()>;

    fn select_program(&mut self, selection: ProgramSelection) -> PlayerResult<()>;

    fn set_video_surface(&mut self, surface: Option<VideoSurface>) -> PlayerResult<()>;

    fn set_video_rectangle(&mut self, rectangle: Option<VideoRectangle>) -> PlayerResult<()>;

    fn poll_event(&mut self) -> PlayerResult<Option<PlayerEvent>>;
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use super::{
        PlayerBackend, PlayerDiagnostic, PlayerError, PlayerErrorKind, PlayerEvent, PlayerResult,
        PlayerState, ProgramNumber, ProgramSelection, VideoRectangle, VideoSurface,
    };

    #[derive(Debug)]
    struct MockPlayerBackend {
        input: Option<PathBuf>,
        selection: ProgramSelection,
        state: PlayerState,
        events: VecDeque<PlayerEvent>,
        rectangle: Option<VideoRectangle>,
        surface: Option<VideoSurface>,
        position: Option<Duration>,
        duration: Option<Duration>,
    }

    impl Default for MockPlayerBackend {
        fn default() -> Self {
            Self {
                input: None,
                selection: ProgramSelection::Automatic,
                state: PlayerState::Idle,
                events: VecDeque::new(),
                rectangle: None,
                surface: None,
                position: None,
                duration: None,
            }
        }
    }

    impl MockPlayerBackend {
        fn require_state(&self, allowed: &[PlayerState], operation: &str) -> PlayerResult<()> {
            if allowed.contains(&self.state) {
                return Ok(());
            }

            Err(PlayerError::new(
                PlayerErrorKind::InvalidState,
                format!("cannot {operation} while player is {:?}", self.state),
            ))
        }
    }

    impl PlayerBackend for MockPlayerBackend {
        fn state(&self) -> PlayerState {
            self.state
        }

        fn program_selection(&self) -> ProgramSelection {
            self.selection
        }

        fn load(&mut self, input: &Path) -> PlayerResult<()> {
            if input.as_os_str().is_empty() {
                return Err(PlayerError::new(
                    PlayerErrorKind::InvalidInput,
                    "input path is empty",
                ));
            }

            self.input = Some(input.to_path_buf());
            self.state = PlayerState::Stopped;
            self.position = Some(Duration::ZERO);
            self.duration = Some(Duration::from_secs(60));
            Ok(())
        }

        fn play(&mut self) -> PlayerResult<()> {
            self.require_state(&[PlayerState::Stopped, PlayerState::Paused], "play")?;
            self.state = PlayerState::Playing;
            Ok(())
        }

        fn pause(&mut self) -> PlayerResult<()> {
            self.require_state(&[PlayerState::Playing], "pause")?;
            self.state = PlayerState::Paused;
            Ok(())
        }

        fn stop(&mut self) -> PlayerResult<()> {
            self.require_state(
                &[
                    PlayerState::Stopped,
                    PlayerState::Playing,
                    PlayerState::Paused,
                ],
                "stop",
            )?;
            self.state = PlayerState::Stopped;
            self.position = Some(Duration::ZERO);
            Ok(())
        }

        fn position(&self) -> Option<Duration> {
            self.position
        }

        fn duration(&self) -> Option<Duration> {
            self.duration
        }

        fn seek(&mut self, position: Duration) -> PlayerResult<()> {
            self.require_state(&[PlayerState::Playing, PlayerState::Paused], "seek")?;
            let duration = self.duration.ok_or_else(|| {
                PlayerError::new(PlayerErrorKind::InvalidState, "duration is unavailable")
            })?;
            self.position = Some(position.min(duration));
            Ok(())
        }

        fn select_program(&mut self, selection: ProgramSelection) -> PlayerResult<()> {
            self.selection = selection;
            Ok(())
        }

        fn set_video_surface(&mut self, surface: Option<VideoSurface>) -> PlayerResult<()> {
            self.surface = surface;
            Ok(())
        }

        fn set_video_rectangle(&mut self, rectangle: Option<VideoRectangle>) -> PlayerResult<()> {
            self.rectangle = rectangle;
            Ok(())
        }

        fn poll_event(&mut self) -> PlayerResult<Option<PlayerEvent>> {
            Ok(self.events.pop_front())
        }
    }

    fn run_lifecycle(backend: &mut dyn PlayerBackend) -> PlayerResult<()> {
        backend.load(Path::new("sample.ts"))?;
        backend.play()?;
        backend.pause()?;
        backend.stop()
    }

    #[test]
    fn backend_contract_supports_expected_lifecycle() -> PlayerResult<()> {
        let mut backend = MockPlayerBackend::default();

        assert_eq!(backend.state(), PlayerState::Idle);

        run_lifecycle(&mut backend)?;

        assert_eq!(backend.state(), PlayerState::Stopped);
        assert_eq!(backend.input.as_deref(), Some(Path::new("sample.ts")));
        Ok(())
    }

    #[test]
    fn backend_contract_reports_invalid_state() {
        let mut backend = MockPlayerBackend::default();
        let result = backend.pause();

        assert!(matches!(
            result,
            Err(ref error) if error.kind() == PlayerErrorKind::InvalidState
        ));
        assert_eq!(backend.state(), PlayerState::Idle);
    }

    #[test]
    fn backend_contract_supports_timeline_queries_and_seek() -> PlayerResult<()> {
        let mut backend = MockPlayerBackend::default();
        backend.load(Path::new("sample.ts"))?;
        backend.play()?;
        backend.seek(Duration::from_secs(15))?;

        assert_eq!(backend.position(), Some(Duration::from_secs(15)));
        assert_eq!(backend.duration(), Some(Duration::from_secs(60)));
        Ok(())
    }

    #[test]
    fn program_selection_rejects_reserved_zero() -> PlayerResult<()> {
        assert_eq!(ProgramNumber::new(0), None);

        let number = match ProgramNumber::new(42) {
            Some(number) => number,
            None => {
                return Err(PlayerError::new(
                    PlayerErrorKind::InvalidInput,
                    "program number 42 was rejected",
                ));
            }
        };
        let mut backend = MockPlayerBackend::default();

        backend.select_program(ProgramSelection::Program(number))?;

        assert_eq!(number.get(), 42);
        assert_eq!(
            backend.program_selection(),
            ProgramSelection::Program(number)
        );
        Ok(())
    }

    #[test]
    fn event_contract_does_not_expose_backend_types() -> PlayerResult<()> {
        let diagnostic = PlayerDiagnostic::new(
            Some("video-decoder".to_owned()),
            "decoder warning",
            Some("diagnostic details".to_owned()),
        );
        let event = PlayerEvent::Warning(diagnostic.clone());
        let mut backend = MockPlayerBackend::default();
        backend.events.push_back(event.clone());

        assert_eq!(backend.poll_event()?, Some(event));
        assert_eq!(diagnostic.source(), Some("video-decoder"));
        assert_eq!(diagnostic.message(), "decoder warning");
        assert_eq!(diagnostic.debug(), Some("diagnostic details"));
        Ok(())
    }

    #[test]
    fn video_rectangle_rejects_empty_surfaces() {
        assert_eq!(VideoRectangle::new(0, 0, 0, 1080), None);
        assert_eq!(VideoRectangle::new(0, 0, 1920, 0), None);
        assert_eq!(
            VideoRectangle::new(10, 20, 1920, 1080),
            Some(VideoRectangle {
                x: 10,
                y: 20,
                width: 1920,
                height: 1080,
            })
        );
    }
}
