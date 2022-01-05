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

use loom::thread;
use loom::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};


/// Returns a [`Trigger`] and [`Listener`] pair bound to each other.
///
/// The [`Listener`] is used to wait for the trigger to fire. It can be waited on both sync
/// and async.
pub fn trigger() -> (Trigger, Listener) {
    let inner = Arc::new(Inner {
        complete: AtomicBool::new(false),
        listeners: Mutex::new(Vec::new()),
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
    listeners: Mutex<Vec<thread::Thread>>,
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
        let mut listeners_guard = self
            .inner
            .listeners
            .lock()
            .expect("Some Trigger/Listener has panicked");
        let listeners = mem::take(&mut *listeners_guard);
        mem::drop(listeners_guard);
        for thread in listeners {
            thread.unpark();
        }
    }

    /// Returns true if this trigger has been triggered.
    pub fn is_triggered(&self) -> bool {
        self.inner.complete.load(Ordering::SeqCst)
    }
}

impl Listener {
    /// Wait for this trigger synchronously.
    ///
    /// Blocks the current thread until the corresponding [`Trigger`] is triggered.
    /// If the trigger has already been triggered at least once, this returns immediately.
    pub fn wait(&self) {
        if self.is_triggered() {
            return;
        }

        let mut listeners_guard = self
            .inner
            .listeners
            .lock()
            .expect("Some Trigger/Listener has panicked");

        // If the trigger completed while we waited for the lock, skip adding our waker to the list
        // of listeners.
        if self.is_triggered() {
            return;
        } else {
            listeners_guard.push(thread::current());
            drop(listeners_guard);
        }

        while !self.is_triggered() {
            thread::park();
        }
    }

    /// Returns true if this trigger has been triggered.
    pub fn is_triggered(&self) -> bool {
        self.inner.complete.load(Ordering::SeqCst)
    }
}

#[test]
fn wait_and_trigger() {
    loom::model(|| {
        let (trigger, listener) = crate::trigger();

        let thread_handle = thread::spawn(move || {
            //thread::yield_now();
            trigger.trigger();
            assert!(trigger.is_triggered());
        });

        //thread::yield_now();
        listener.wait();

        assert!(listener.is_triggered());

        // Uncomment any of the `yield_now` calls or remove this join
        // to make the test pass.
        thread_handle.join().expect("Trigger thread panic");
    })
}
