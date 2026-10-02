//! A Bluetooth channel as an async byte stream.
//!
//! The shim is blocking and run-loop driven, so each channel gets two helper
//! threads: one parked in `bt_recv`, one draining writes. That keeps every
//! blocking call off the async executor, and lets the channel present the
//! plain `AsyncRead + AsyncWrite` that `maestro`'s codec wants.

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc;

use crate::ffi;

/// How long a reader thread parks in `bt_recv` before looping to re-check
/// whether the channel has been dropped.
const RECV_POLL_MS: i32 = 250;

/// Owns the shim handle. The shim guards its own state, so this is safe to
/// use from the reader and writer threads at once.
struct Handle(*mut ffi::bt_chan);

// SAFETY: every shim entry point either marshals onto the main thread or takes
// the channel's own lock, so the handle may be used from any thread.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle is only dropped once, when the last Arc goes.
        unsafe { ffi::bt_close(self.0) };
    }
}

pub struct Channel {
    handle: Arc<Handle>,
    closed: Arc<AtomicBool>,
    inbound: mpsc::UnboundedReceiver<Vec<u8>>,
    outbound: mpsc::UnboundedSender<Vec<u8>>,
    /// Bytes from a frame the caller's buffer was too small to take whole.
    pending: Vec<u8>,
    mtu: usize,
}

impl Channel {
    /// Wraps an opened shim handle, starting its reader and writer threads.
    pub(crate) fn new(raw: *mut ffi::bt_chan) -> Self {
        // SAFETY: `raw` comes from a successful bt_*_open.
        let mtu = unsafe { ffi::bt_mtu(raw) } as usize;
        let handle = Arc::new(Handle(raw));
        let closed = Arc::new(AtomicBool::new(false));

        let (in_tx, inbound) = mpsc::unbounded_channel();
        let (outbound, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();

        let reader = (Arc::clone(&handle), Arc::clone(&closed));
        std::thread::Builder::new()
            .name("btmac-recv".into())
            .spawn(move || {
                let (handle, closed) = reader;
                // One MTU is enough to take any frame whole.
                let mut buf = vec![0u8; mtu.max(4096)];
                while !closed.load(Ordering::Acquire) {
                    // SAFETY: buf outlives the call; the shim copies into it.
                    let n = unsafe {
                        ffi::bt_recv(handle.0, buf.as_mut_ptr(), buf.len(), RECV_POLL_MS)
                    };
                    match n {
                        0 => continue,       // nothing yet
                        n if n < 0 => break, // closed and drained
                        n => {
                            if in_tx.send(buf[..n as usize].to_vec()).is_err() {
                                break; // receiver dropped
                            }
                        }
                    }
                }
            })
            .expect("spawn btmac-recv");

        let writer = (Arc::clone(&handle), Arc::clone(&closed));
        std::thread::Builder::new()
            .name("btmac-send".into())
            .spawn(move || {
                let (handle, closed) = writer;
                while let Some(frame) = out_rx.blocking_recv() {
                    if closed.load(Ordering::Acquire) {
                        break;
                    }
                    // SAFETY: frame outlives the call; the shim copies out of it.
                    if unsafe { ffi::bt_send(handle.0, frame.as_ptr(), frame.len()) } != 0 {
                        break;
                    }
                }
            })
            .expect("spawn btmac-send");

        Self {
            handle,
            closed,
            inbound,
            outbound,
            pending: Vec::new(),
            mtu,
        }
    }

    /// Largest payload the channel carries in one write.
    pub fn mtu(&self) -> usize {
        self.mtu
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
    }
}

impl AsyncRead for Channel {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if !self.pending.is_empty() {
            let n = self.pending.len().min(buf.remaining());
            let rest = self.pending.split_off(n);
            buf.put_slice(&self.pending);
            self.pending = rest;
            return Poll::Ready(Ok(()));
        }
        match self.inbound.poll_recv(cx) {
            Poll::Ready(Some(frame)) => {
                let n = frame.len().min(buf.remaining());
                buf.put_slice(&frame[..n]);
                if n < frame.len() {
                    self.pending = frame[n..].to_vec();
                }
                Poll::Ready(Ok(()))
            }
            // Reader thread ended: the channel is gone. EOF, not an error, so
            // the link layer treats it as a disconnect and reconnects.
            Poll::Ready(None) => Poll::Ready(Ok(())),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncWrite for Channel {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        // Writes are queued rather than pushed through the shim here, so the
        // executor never blocks on the main thread's run loop.
        let n = buf.len().min(self.mtu.max(1));
        match self.outbound.send(buf[..n].to_vec()) {
            Ok(()) => Poll::Ready(Ok(n)),
            Err(_) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "bluetooth channel closed",
            ))),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.closed.store(true, Ordering::Release);
        Poll::Ready(Ok(()))
    }
}

// Keeps the handle field read, and documents that the channel owns it.
impl Channel {
    #[allow(dead_code)]
    fn raw(&self) -> *mut ffi::bt_chan {
        self.handle.0
    }
}
