use std::io::{self, Result};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use std::net::SocketAddr;

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
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize>> {
        let this = self.get_mut();
        Pin::new(&mut this.stream).poll_write(cx, buf)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<()>> {
        let this = self.get_mut();
        Pin::new(&mut this.stream).poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<()>> {
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
