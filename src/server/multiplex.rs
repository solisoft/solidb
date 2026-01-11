use std::io::{self, Result};
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

/// A stream that has had some bytes peeked from it.
pub struct PeekedStream {
    stream: TcpStream,
    peeked: Option<Vec<u8>>,
    peek_cursor: usize,
}

impl PeekedStream {
    pub fn new(stream: TcpStream, peeked_bytes: Vec<u8>) -> Self {
        Self {
            stream,
            peeked: Some(peeked_bytes),
            peek_cursor: 0,
        }
    }
}

impl AsyncRead for PeekedStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<Result<()>> {
        let this = self.get_mut();

        if let Some(peeked) = this.peeked.take() {
            // tracing::info!("PeekedStream: serving from peeked buffer (len={}, remaining={})", peeked.len(), buf.remaining());
            if this.peek_cursor < peeked.len() {
                let available = &peeked[this.peek_cursor..];
                let to_copy = std::cmp::min(available.len(), buf.remaining());
                buf.put_slice(&available[..to_copy]);
                this.peek_cursor += to_copy;

                if this.peek_cursor < peeked.len() {
                    this.peeked = Some(peeked);
                } else {
                    // tracing::info!("PeekedStream: peeked buffer exhausted");
                    this.peeked = None;
                }

                return Poll::Ready(Ok(()));
            } else {
                this.peeked = None;
            }
        }

        // tracing::info!("PeekedStream: delegating to stream");
        let poll = Pin::new(&mut this.stream).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &poll {
            // tracing::info!("PeekedStream: stream returned data");
        }
        poll
    }
}

impl AsyncWrite for PeekedStream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
        let this = self.get_mut();
        Pin::new(&mut this.stream).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        let this = self.get_mut();
        Pin::new(&mut this.stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        let this = self.get_mut();
        Pin::new(&mut this.stream).poll_shutdown(cx)
    }
}

/// A listener that accepts connections from a channel.
pub struct ChannelListener {
    rx: mpsc::Receiver<(PeekedStream, SocketAddr)>,
    local_addr: SocketAddr,
}

impl ChannelListener {
    pub fn new(rx: mpsc::Receiver<(PeekedStream, SocketAddr)>, local_addr: SocketAddr) -> Self {
        Self { rx, local_addr }
    }
}

// Implement axum::serve::Listener for ChannelListener
impl axum::serve::Listener for ChannelListener {
    type Io = PeekedStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.rx.recv().await {
            Some((s, a)) => (s, a),
            None => {
                // Channel closed, wait forever to allow graceful shutdown to complete
                std::future::pending().await
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        Ok(self.local_addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn test_peeked_stream_reconstruction() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
            stream.write_all(b"1234567890").await.unwrap();
        });

        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 5];
        socket.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"12345");

        let mut peeked_stream = PeekedStream::new(socket, buf.to_vec());
        let mut content = Vec::new();
        peeked_stream.read_to_end(&mut content).await.unwrap();

        assert_eq!(content, b"1234567890");
    }

    #[tokio::test]
    async fn test_peeked_stream_partial_read() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
            stream.write_all(b"ABCDEFGHIJ").await.unwrap();
        });

        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 3];
        socket.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ABC"); // Left: DEFGHIJ

        let mut peeked_stream = PeekedStream::new(socket, buf.to_vec());

        let mut small_buf = [0u8; 2];
        peeked_stream.read_exact(&mut small_buf).await.unwrap();
        assert_eq!(&small_buf, b"AB");

        let mut med_buf = [0u8; 3];
        peeked_stream.read_exact(&mut med_buf).await.unwrap();
        assert_eq!(&med_buf, b"CDE");

        let mut content = String::new();
        peeked_stream.read_to_string(&mut content).await.unwrap();
        assert_eq!(content, "FGHIJ");
    }
}
