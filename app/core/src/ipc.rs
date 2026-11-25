//! The platform transports for discord's local ipc. This is the only platform
//! specific piece; everything else works on any `AsyncRead + AsyncWrite`.
//!
//! | platform | endpoint |
//! | --- | --- |
//! | windows | named pipe `\\.\pipe\discord-ipc-N` |
//! | macos | unix socket `$TMPDIR/discord-ipc-N` |
//! | linux | unix socket `$XDG_RUNTIME_DIR/discord-ipc-N`, plus the flatpak and snap spots |
//!
//! [`PlatformConnector`] is whichever one fits the os being built for.

#[cfg(any(windows, test))]
mod windows;

#[cfg(any(unix, test))]
mod unix;

#[cfg(any(windows, test))]
pub use windows::pipe_path;

#[cfg(windows)]
pub use windows::PipeConnector;

#[cfg(any(unix, test))]
pub use unix::{candidate_dirs, socket_name};

#[cfg(unix)]
pub use unix::UnixConnector;

/// The connector for the os mimic was built for.
#[cfg(windows)]
pub type PlatformConnector = PipeConnector;

/// The connector for the os mimic was built for.
#[cfg(unix)]
pub type PlatformConnector = UnixConnector;
