//! Only for connecting to a websocket via a local socket

use std::{
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use pin_project::pin_project;
use tokio::io::{AsyncRead, AsyncWrite};
#[cfg(unix)]
use tokio::net::UnixStream;
#[cfg(windows)]
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
#[cfg(windows)]
use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;

use crate::{Error, Result};

#[pin_project(project = WrapStreamProj)]
pub enum WrapStream {
    #[cfg(unix)]
    Unix(#[pin] UnixStream),
    #[cfg(windows)]
    NamedPipe(#[pin] NamedPipeClient),
}

// impl WrapStream {
//     pub async fn readable(&self) -> std::io::Result<()> {
//         match self {
//             #[cfg(unix)]
//             WrapStream::Unix(s) => s.readable().await,
//             #[cfg(windows)]
//             WrapStream::NamedPipe(s) => s.readable().await,
//         }
//     }
//     pub async fn writable(&self) -> std::io::Result<()> {
//         match self {
//             #[cfg(unix)]
//             WrapStream::Unix(s) => s.writable().await,
//             #[cfg(windows)]
//             WrapStream::NamedPipe(s) => s.writable().await,
//         }
//     }
// }

impl AsyncRead for WrapStream {
    #[inline]
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.project() {
            #[cfg(unix)]
            WrapStreamProj::Unix(s) => s.poll_read(cx, buf),
            #[cfg(windows)]
            WrapStreamProj::NamedPipe(s) => s.poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for WrapStream {
    #[inline]
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        match self.project() {
            #[cfg(unix)]
            WrapStreamProj::Unix(s) => s.poll_write(cx, buf),
            #[cfg(windows)]
            WrapStreamProj::NamedPipe(s) => s.poll_write(cx, buf),
        }
    }

    #[inline]
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.project() {
            #[cfg(unix)]
            WrapStreamProj::Unix(s) => s.poll_flush(cx),
            #[cfg(windows)]
            WrapStreamProj::NamedPipe(s) => s.poll_flush(cx),
        }
    }

    #[inline]
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.project() {
            #[cfg(unix)]
            WrapStreamProj::Unix(s) => s.poll_shutdown(cx),
            #[cfg(windows)]
            WrapStreamProj::NamedPipe(s) => s.poll_shutdown(cx),
        }
    }
}

pub async fn connect_to_socket(socket_path: &str) -> Result<WrapStream> {
    #[cfg(unix)]
    {
        use std::io::ErrorKind;
        const MAX_RETRIES: u32 = 3;
        const BASE_DELAY: Duration = Duration::from_millis(25);

        let mut last_err: Option<std::io::Error> = None;

        for attempt in 0..=MAX_RETRIES {
            let connect_fut = tokio::time::timeout(Duration::from_millis(200), UnixStream::connect(socket_path));
            let connection_result = match connect_fut.await {
                Ok(result) => result,
                Err(_) => {
                    log::warn!("Socket connect attempt {attempt} timed out");
                    last_err = Some(std::io::Error::new(ErrorKind::TimedOut, "connect timeout"));
                    continue;
                }
            };

            match connection_result {
                Ok(stream) => return Ok(WrapStream::Unix(stream)),
                Err(e) => match e.kind() {
                    ErrorKind::PermissionDenied => {
                        log::error!("Permission denied for socket: {socket_path}");
                        return Err(Error::Io(e));
                    }
                    _ => {
                        log::warn!("Socket connect attempt {attempt} failed: {e}");
                        last_err = Some(e);
                    }
                },
            }

            if attempt < MAX_RETRIES {
                let delay = BASE_DELAY * 2u32.pow(attempt);
                let jitter = Duration::from_millis(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| (d.as_nanos() % 25) as u64)
                        .unwrap_or(0),
                );

                tokio::time::sleep(delay + jitter).await;
            }
        }

        Err(Error::Io(
            last_err.unwrap_or_else(|| std::io::Error::other("Retries exhausted")),
        ))
    }

    #[cfg(windows)]
    {
        let mut max_retry_count = 3;
        const RETRY_DELAY: Duration = Duration::from_millis(125);

        let client = loop {
            match ClientOptions::new().open(socket_path) {
                Ok(client) => break client,
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => (),
                Err(e) => {
                    log::warn!("failed to connect to named pipe: {socket_path}, {e}");
                    if max_retry_count == 0 {
                        return Err(Error::Io(std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            format!("Failed to connect to named pipe: {socket_path}, {e}"),
                        )));
                    }
                    max_retry_count -= 1;
                }
            }
            tokio::time::sleep(RETRY_DELAY).await;
        };
        Ok(WrapStream::NamedPipe(client))
    }
}
