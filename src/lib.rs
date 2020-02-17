//! Simple triggers that allows triggering a one time event in another task/thread.
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
//!     tokio::spawn(async {
//!         listener.await;
//!         println!("Trigger went off!");
//!     });
//!     trigger.trigger();
//! }
//! ```
//!
//! An example showing a trigger/listener pair being used to gracefully shut down some async
//! server instances on a Ctrl+C event, where only an immutable `Fn` closure is accepted:
//!
//! ```ignore
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error> {
//!     let (shutdown_trigger, shutdown_signal) = triggered::trigger();
//!
//!     ctrlc::set_handler(move || {
//!         shutdown_trigger.trigger();
//!     }).expect("Error setting Ctrl-C handler");
//!
//!     let server1_task = tokio::spawn(async {
//!         SomeServer::new().serve_with_shutdown_signal(shutdown_signal.clone());
//!     });
//!     let server2_task = tokio::spawn(async {
//!         SomeOServer::new().serve_with_shutdown_signal(shutdown_signal);
//!     });
//!
//!     server1_task.await?;
//!     server2_task.await?;
//! }
//! ```
//!
//! # Comparison to channels
//!
//! The listener is somewhat similar to a `futures::channel::oneshot::Receiver<()>`. But it:
//!  * Is not fallible - Implements `Future<()>` instead of `Future<(), Canceled>`
//!  * Implements `Clone` - Any number of listeners can wait for the same oneshot event
//!  * Has a sync [`Listener::wait`] - Both synchronous threads, and asynchronous tasks can wait
//!    at the same time.
//!
//! The trigger, when compared to a `futures::channel::oneshot::Sender<()>` has the differences
//! that it:
//!  * Is not fallible - The trigger does not care if there are any listeners left
//!  * Does not consume itself on send, instead takes `&self` - So can be used
//!    in situations where it is not owned or not mutable. For example in `Drop` implementations
//!    or callback closures that are limited to `Fn` or `FnMut`

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
}
