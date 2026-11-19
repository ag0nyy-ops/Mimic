//! The windows named pipe transport. This is the only platform specific piece;
//! everything else works on any `AsyncRead + AsyncWrite`.

/// `\\.\pipe\discord-ipc-N`
pub fn pipe_path(index: u8) -> String {
    format!(r"\\.\pipe\discord-ipc-{index}")
}

#[cfg(windows)]
pub use windows_impl::PipeConnector;

#[cfg(windows)]
mod windows_impl {
    use std::io;
    use std::time::Duration;

    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

    use super::pipe_path;
    use crate::manager::Connector;

    /// `ERROR_PIPE_BUSY`: every instance of the pipe is momentarily taken.
    const ERROR_PIPE_BUSY: i32 = 231;

    /// Connects to discord's named pipes.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct PipeConnector;

    impl Connector for PipeConnector {
        type Stream = NamedPipeClient;

        async fn connect(&self, index: u8) -> io::Result<Self::Stream> {
            let name = pipe_path(index);
            let mut attempts = 0;
            loop {
                match ClientOptions::new().open(&name) {
                    Ok(client) => return Ok(client),
                    Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && attempts < 5 => {
                        attempts += 1;
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    Err(e) => return Err(e),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_names() {
        assert_eq!(pipe_path(0), r"\\.\pipe\discord-ipc-0");
        assert_eq!(pipe_path(9), r"\\.\pipe\discord-ipc-9");
    }
}
