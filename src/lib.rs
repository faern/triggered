//! Triggers for one time events between tasks and threads.
//!
//! The mechanism consists of two types, the [`Trigger`] and the [`Listener`]. They come together
//! as a pair. Much like the sender/receiver pair of a channel. The trigger half has a
//! [`Trigger::trigger`] method that will make all tasks/threads waiting on
//! a listener continue executing.
//! The listener both has a sync [`Listener::wait`] method, and it also implements
//! `Future<Output = ()>` for async support.
//!
//! Both the [`Trigger`] and [`Listener`] can be cloned. So any number of trigger instances can
//! trigger any number of waiting listeners. When any one trigger instance belonging to the pair is
//! triggered, all the waiting listeners will be unblocked. Waiting on a listener whose
//! trigger already went off will return instantly. So each trigger/listener pair can only be fired
//! once.
//!
//! This crate does not use any `unsafe` code.
//!
//! # Examples
//!
//! A trivial example showing the basic usage:
//!
//! ```
//! #[tokio::main]
//! async fn main() {
//!     let (trigger, listener) = triggered::trigger();
//!
//!     let task = tokio::spawn(async {
//!         // Blocks until `trigger.trigger()` below
//!         listener.await;
//!
//!         println!("Triggered async task");
//!     });
//!
//!     // This will make any thread blocked in `Listener::wait()` or async task awaiting the
//!     // listener continue execution again.
//!     trigger.trigger();
//!
//!     let _ = task.await;
//! }
//! ```
//!
//! An example showing a trigger/listener pair being used to gracefully shut down some async
//! server instances on a Ctrl-C event, where only an immutable `Fn` closure is accepted:
//!
//! ```
//! # use std::future::Future;
//! # type Error = Box<dyn std::error::Error>;
//! # struct SomeServer;
//! # impl SomeServer {
//! #    fn new() -> Self { SomeServer }
//! #    async fn serve_with_shutdown_signal(self, s: impl Future<Output = ()>) -> Result<(), Error> {Ok(())}
//! #    async fn serve(self) -> Result<(), Error> {Ok(())}
//! # }
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Error> {
//!     let (shutdown_trigger, shutdown_signal1) = triggered::trigger();
//!
//!     // A sync `Fn` closure will trigger the trigger when the user hits Ctrl-C
//!     ctrlc::set_handler(move || {
//!         shutdown_trigger.trigger();
//!     }).expect("Error setting Ctrl-C handler");
//!
//!     // If the server library has support for something like a shutdown signal:
//!     let shutdown_signal2 = shutdown_signal1.clone();
//!     let server1_task = tokio::spawn(async move {
//!         SomeServer::new().serve_with_shutdown_signal(shutdown_signal1).await;
//!     });
//!
//!     // Or just select between the long running future and the signal to abort it
//!     tokio::select! {
//!         server_result = SomeServer::new().serve() => {
//!             eprintln!("Server error: {:?}", server_result);
//!         }
//!         _ = shutdown_signal2 => {}
//!     }
//!
//!     let _ = server1_task.await;
//!     Ok(())
//! }
//! ```
//!
//! # Rust Compatibility
//!
//! Will work with at least the two latest stable Rust releases. This gives users at least six
//! weeks to upgrade their Rust toolchain after a new stable is released.
//!
//! The current MSRV can be seen in `travis.yml`. Any change to the MSRV will be considered a
//! breaking change and listed in the changelog.
//!
//! # Comparison with similar primitives
//!
//! ## Channels
//!
//! The event triggering primitives in this library is somewhat similar to channels. The main
//! difference and why I developed this library is that
//!
//! The listener is somewhat similar to a `futures::channel::oneshot::Receiver<()>`. But it:
//!  * Is not fallible - Implements `Future<Output = ()>` instead of
//!    `Future<Output = Result<T, Canceled>>`
//!  * Implements `Clone` - Any number of listeners can wait for the same event
//!  * Has a sync [`Listener::wait`] - Both synchronous threads, and asynchronous tasks can wait
//!    at the same time.
//!
//! The trigger, when compared to a `futures::channel::oneshot::Sender<()>` has the differences
//! that it:
//!  * Is not fallible - The trigger does not care if there are any listeners left
//!  * Does not consume itself on send, instead takes `&self` - So can be used
//!    in situations where it is not owned or not mutable. For example in `Drop` implementations
//!    or callback closures that are limited to `Fn` or `FnMut`.
//!
//! ## `futures::future::Abortable`
//!
//! One use case of these triggers is to abort futures when some event happens. See examples above.
//! The differences include:
//!  * A single handle can abort any number of futures
//!  * Some futures are not properly cleaned up when just dropped the way `Abortable` does it.
//!    These libraries sometimes allows creating their futures with a shutdown signal that triggers
//!    a clean abort. Something like `serve_with_shutdown(signal: impl Future<Output = ()>)`.

#![deny(unsafe_code)]
#![deny(rust_2018_idioms)]

use std::mem;
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Condvar, Mutex,
};
use std::task::{Context, Poll, Waker};

/// Returns a [`Trigger`] and [`Listener`] pair bound to each other.
///
/// The [`Listener`] is used to wait for the trigger to fire. It can be waited on both sync
/// and async.
pub fn trigger() -> (Trigger, Listener) {
    let inner = Arc::new(Inner {
        complete: AtomicBool::new(false),
        tasks: Mutex::new(Vec::new()),
        condvar: Condvar::new(),
    });
    let trigger = Trigger {
        inner: inner.clone(),
    };
    let listener = Listener { inner };
    (trigger, listener)
}

/// A struct used to trigger [`Listener`]s it is paired with.
///
/// Can be cloned to create multiple instances that all trigger the same listeners.
#[derive(Clone, Debug)]
pub struct Trigger {
    inner: Arc<Inner>,
}

/// A struct used to wait for a trigger event from a [`Trigger`].
///
/// Can be waited on synchronously via [`Listener::wait`] or asynchronously thanks to the struct
/// implementing `Future`.
///
/// The listener can be cloned and any amount of threads and tasks can wait for the same trigger
/// at the same time.
#[derive(Clone, Debug)]
pub struct Listener {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    complete: AtomicBool,
    tasks: Mutex<Vec<Waker>>,
    condvar: Condvar,
}

impl Unpin for Trigger {}
impl Unpin for Listener {}

impl Trigger {
    /// Trigger all [`Listener`]s paired with this trigger.
    ///
    /// Makes all listeners currently blocked in [`Listener::wait`] return,
    /// and all that is being `await`ed finish.
    ///
    /// Calling this method only does anything the first time. Any subsequent trigger call to
    /// the same instance or a clone thereof does nothing, it has already been triggered.
    /// Any listener waiting on the trigger after it has been triggered will just return
    /// instantly.
    ///
    /// This method is safe to call from both async and sync code. It's not an async function,
    /// but it always finishes very fast.
    pub fn trigger(&self) {
        if self.inner.complete.swap(true, Ordering::SeqCst) {
            return;
        }
        // This code will only be executed once per trigger instance. No matter the amount of
        // `Trigger` clones or calls to `trigger()`, thanks to the atomic swap above.
        let mut tasks_guard = self
            .inner
            .tasks
            .lock()
            .expect("Some Trigger/Listener has panicked");
        let tasks = mem::replace(&mut *tasks_guard, Vec::new());
        mem::drop(tasks_guard);
        for task in tasks {
            task.wake();
        }
        self.inner.condvar.notify_all();
    }

    /// Returns true if this trigger has been triggered.
    pub fn is_triggered(&self) -> bool {
        self.inner.complete.load(Ordering::SeqCst)
    }
}

impl std::future::Future for Listener {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.inner.complete.load(Ordering::SeqCst) {
            return Poll::Ready(());
        }

        let mut tasks = self
            .inner
            .tasks
            .lock()
            .expect("Some Trigger/Listener has panicked");

        // If the trigger completed while we waited for the lock, skip adding our waker to the list
        // of tasks.
        if self.inner.complete.load(Ordering::SeqCst) {
            Poll::Ready(())
        } else {
            tasks.push(cx.waker().clone());
            Poll::Pending
        }
    }
}

impl Listener {
    /// Wait for this trigger synchronously.
    ///
    /// Blocks the current thread until the corresponding [`Trigger`] is triggered.
    /// If the trigger has already been triggered at least once, this returns immediately.
    pub fn wait(&self) {
        if self.inner.complete.load(Ordering::SeqCst) {
            return;
        }

        let mut guard = self
            .inner
            .tasks
            .lock()
            .expect("Some Trigger/Listener has panicked");

        while !self.inner.complete.load(Ordering::SeqCst) {
            guard = self
                .inner
                .condvar
                .wait(guard)
                .expect("Some Trigger/Listener has panicked");
        }
    }

    /// Returns true if this trigger has been triggered.
    pub fn is_triggered(&self) -> bool {
        self.inner.complete.load(Ordering::SeqCst)
    }
}

#[allow(unsafe_code)]
#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::sync::atomic::AtomicU8;
    use std::task::{RawWaker, RawWakerVTable};

    #[test]
    fn polling_listener_keeps_only_last_waker() {
        let (_trigger, mut listener) = trigger();

        let (waker1, waker_handle1) = create_waker();
        {
            let mut context = Context::from_waker(&waker1);
            let listener = Pin::new(&mut listener);
            assert_eq!(listener.poll(&mut context), Poll::Pending);
        }
        assert!(waker_handle1.data.load(Ordering::SeqCst) & CLONED != 0);
        assert!(waker_handle1.data.load(Ordering::SeqCst) & DROPPED == 0);

        let (waker2, waker_handle2) = create_waker();
        {
            let mut context = Context::from_waker(&waker2);
            let listener = Pin::new(&mut listener);
            assert_eq!(listener.poll(&mut context), Poll::Pending);
        }
        assert!(waker_handle2.data.load(Ordering::SeqCst) & CLONED != 0);
        assert!(waker_handle2.data.load(Ordering::SeqCst) & DROPPED == 0);
        assert!(waker_handle1.data.load(Ordering::SeqCst) & DROPPED != 0);
    }

    const CLONED: u8 = 0b0001;
    const WOKE: u8 = 0b0010;
    const DROPPED: u8 = 0b0100;

    fn create_waker() -> (Waker, Arc<WakerHandle>) {
        let waker_handle = Arc::new(WakerHandle {
            data: AtomicU8::new(0),
        });
        let data = Arc::into_raw(waker_handle.clone()) as *const _;
        let raw_waker = RawWaker::new(data, &VTABLE);
        (unsafe { Waker::from_raw(raw_waker) }, waker_handle)
    }

    struct WakerHandle {
        data: AtomicU8,
    }

    impl Drop for WakerHandle {
        fn drop(&mut self) {
            println!("WakerHandle dropped");
        }
    }

    const VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop);

    unsafe fn clone(data: *const ()) -> RawWaker {
        let waker_handle = &*(data as *const WakerHandle);
        waker_handle.data.fetch_or(CLONED, Ordering::SeqCst);
        Arc::increment_strong_count(waker_handle);
        RawWaker::new(data, &VTABLE)
    }

    unsafe fn wake(data: *const ()) {
        let waker_handle = &*(data as *const WakerHandle);
        waker_handle.data.fetch_or(WOKE, Ordering::SeqCst);
    }

    unsafe fn wake_by_ref(_data: *const ()) {
        todo!();
    }

    unsafe fn drop(data: *const ()) {
        let waker_handle = &*(data as *const WakerHandle);
        waker_handle.data.fetch_or(DROPPED, Ordering::SeqCst);
        Arc::decrement_strong_count(waker_handle);
    }
}
