mod backend;
pub mod platform;
pub mod source;

pub use backend::{
    PlayerBackend, PlayerDiagnostic, PlayerError, PlayerErrorKind, PlayerEvent, PlayerResult,
    PlayerState, ProgramNumber, ProgramSelection, VideoRectangle, VideoSurface,
};
