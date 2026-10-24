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

pub(super) struct Transport {
    stream: TcpStream,
    end: Instant,
    cancel: Option<CancellationToken>,
}

impl Transport {
    pub(super) fn set_budget(&mut self, budget: &Budget<'_>) {
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

impl Read for Transport {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        loop {
            self.stream.set_read_timeout(Some(self.remaining()?))?;
            match self.stream.read(bytes) {
                Err(error) if pending(&error) => continue,
                result => return result,
            }
        }
    }
}

impl Write for Transport {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        loop {
            self.stream.set_write_timeout(Some(self.remaining()?))?;
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
