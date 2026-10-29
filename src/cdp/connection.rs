//! Check the operation budget for every socket read/write, including partial
//! HTTP headers and WebSocket frames consumed inside the protocol library.

use super::{Budget, failed, pending};
use crate::Result;
use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use tungstenite::{WebSocket, protocol::WebSocketConfig};

pub(crate) trait Socket: Read + Write {
    fn read_timeout(&self, duration: Duration) -> io::Result<()>;
    fn write_timeout(&self, duration: Duration) -> io::Result<()>;
}

impl Socket for TcpStream {
    fn read_timeout(&self, duration: Duration) -> io::Result<()> {
        self.set_read_timeout(Some(duration))
    }
    fn write_timeout(&self, duration: Duration) -> io::Result<()> {
        self.set_write_timeout(Some(duration))
    }
}

#[cfg(all(unix, feature = "rendering"))]
impl Socket for std::os::unix::net::UnixStream {
    fn read_timeout(&self, duration: Duration) -> io::Result<()> {
        self.set_read_timeout(Some(duration))
    }
    fn write_timeout(&self, duration: Duration) -> io::Result<()> {
        self.set_write_timeout(Some(duration))
    }
}

pub(crate) struct Transport<S: Socket = TcpStream> {
    stream: S,
    end: Instant,
    cancel: Option<CancellationToken>,
}

impl<S: Socket> Transport<S> {
    #[cfg(all(unix, feature = "rendering"))]
    pub(crate) fn new(stream: S, budget: &Budget<'_>) -> Self {
        Self {
            stream,
            end: budget.end,
            cancel: budget.cancel.cloned(),
        }
    }

    pub(crate) fn set_budget(&mut self, budget: &Budget<'_>) {
        self.end = budget.end;
        self.cancel = budget.cancel.cloned();
    }

    fn remaining(&self) -> io::Result<Duration> {
        if self
            .cancel
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "CDP operation cancelled",
            ));
        }
        self.end
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .map(|left| left.min(Duration::from_millis(50)))
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::TimedOut, "CDP operation deadline exceeded")
            })
    }
}

impl<S: Socket> Read for Transport<S> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        loop {
            self.stream.read_timeout(self.remaining()?)?;
            match self.stream.read(bytes) {
                Err(error) if pending(&error) => continue,
                result => return result,
            }
        }
    }
}

impl<S: Socket> Write for Transport<S> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        loop {
            self.stream.write_timeout(self.remaining()?)?;
            match self.stream.write(bytes) {
                Err(error) if pending(&error) => continue,
                result => return result,
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.remaining()?;
        self.stream.flush()
    }
}

pub(super) fn connect(
    address: SocketAddr,
    path: &str,
    budget: &Budget<'_>,
) -> Result<WebSocket<Transport>> {
    let result = (|| {
        let stream =
            TcpStream::connect_timeout(&address, budget.remaining()?.min(Duration::from_secs(1)))
                .map_err(|error| failed(format!("Cannot connect to local CDP: {error}")))?;
        let transport = Transport {
            stream,
            end: budget.end,
            cancel: budget.cancel.cloned(),
        };
        let config = WebSocketConfig::default()
            .max_message_size(Some(96 * 1024 * 1024))
            .max_frame_size(Some(96 * 1024 * 1024));
        let (socket, _) = tungstenite::client::client_with_config(
            format!("ws://{address}{path}"),
            transport,
            Some(config),
        )
        .map_err(|error| failed(format!("CDP handshake failed: {error}")))?;
        budget.check()?;
        Ok(socket)
    })();
    result.map_err(|error| budget.check().err().unwrap_or(error))
}

#[cfg(test)]
mod tests;
