//! End document work when input closes, before the protocol response drain.
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, ReadBuf};
use tokio_util::sync::CancellationToken;

pub(super) struct Input<R> {
    inner: R,
    closed: CancellationToken,
}
impl<R> Input<R> {
    pub fn new(inner: R, closed: CancellationToken) -> Self {
        Self { inner, closed }
    }
}
impl<R: AsyncRead + Unpin> AsyncRead for Input<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let has_room = buf.remaining() > 0;
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        if matches!(&result, Poll::Ready(Err(_)))
            || matches!(&result, Poll::Ready(Ok(()))) && has_room && buf.filled().len() == before
        {
            self.closed.cancel();
        }
        result
    }
}
impl<R> Drop for Input<R> {
    fn drop(&mut self) {
        self.closed.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test]
    async fn bytes_and_empty_buffers_do_not_cancel_but_eof_does() {
        let (mut writer, reader) = tokio::io::duplex(32);
        let closed = CancellationToken::new();
        let mut input = Input::new(reader, closed.clone());
        input.read_exact(&mut []).await.unwrap();
        assert!(!closed.is_cancelled());
        writer.write_all(b"input").await.unwrap();
        let mut bytes = [0; 5];
        input.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"input");
        assert!(!closed.is_cancelled());
        drop(writer);
        assert_eq!(input.read(&mut bytes).await.unwrap(), 0);
        assert!(closed.is_cancelled());
    }
}
