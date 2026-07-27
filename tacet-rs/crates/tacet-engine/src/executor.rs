//! Minimal executor — for TESTS and EVAL.
//!
//! Why not tokio: this crate MUST NOT PICK a runtime. `EngineProvider` only
//! imposes the `Send` bound; which runtime drives it is the layer above's
//! decision (the app may use tokio, the CLI may block). Core has no tokio for
//! the same reason.
//!
//! BUSY-WAIT WARNING: `wait` does not support wakers, it spins polling the
//! future until it is Ready. `FakeEngine` and pure-computation futures return
//! Ready on the first poll, so the loop turns once. On a future that waits on
//! REAL I/O this burns CPU — use tokio there.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

fn noop(_: *const ()) {}
fn clone(p: *const ()) -> RawWaker {
    RawWaker::new(p, &VT)
}
static VT: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);

/// Polls the future until it completes and returns its output.
pub fn wait<F: Future>(future: F) -> F::Output {
    // The waker does nothing; the loop itself takes over the "wake up" job.
    let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VT)) };
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let Poll::Ready(v) = Pin::as_mut(&mut future).poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}
