#![deny(unsafe_code)]
#![deny(rust_2018_idioms)]

use std::mem;
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Condvar, Mutex,
};
use std::task::{Context, Poll, Waker};

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

#[derive(Clone)]
pub struct Trigger {
    inner: Arc<Inner>,
}

#[derive(Clone)]
pub struct Listener {
    inner: Arc<Inner>,
}

struct Inner {
    complete: AtomicBool,
    tasks: Mutex<Vec<Waker>>,
    condvar: Condvar,
}

impl Unpin for Trigger {}
impl Unpin for Listener {}

impl Trigger {
    pub fn trigger(&self) {
        if self.inner.complete.swap(true, Ordering::SeqCst) {
            return;
        }
        // This code will only be executed once. No matter the amount of `Trigger` clones or calls
        // to `trigger()`, thanks to the atomic swap above.
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
    /// Wait for this trigger synchronously. Blocks the current thread until the corresponding
    /// [`Trigger`] is triggered.
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
