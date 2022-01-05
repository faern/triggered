#[cfg(loom)]
use loom::thread;
#[cfg(not(loom))]
use std::thread;

use std::time::{Duration, Instant};

fn maybe_loom_model(test: impl Fn() + Sync + Send + 'static) {
    #[cfg(loom)]
    loom::model(test);
    #[cfg(not(loom))]
    test();
}

#[test]
fn join_threads() {
    loom::model(|| {
        let join_handle = thread::spawn(|| {
            "hej"
        });
        let result = join_handle.join().unwrap();
        assert_eq!("hej", result);
    })
}

#[test]
fn single_trigger_first() {
    maybe_loom_model(|| {
        let (trigger, listener) = triggered::trigger();
        assert!(!trigger.is_triggered());
        assert!(!listener.is_triggered());
        trigger.trigger();
        assert!(trigger.is_triggered());
        assert!(listener.is_triggered());
        listener.wait();
        assert!(trigger.is_triggered());
        assert!(listener.is_triggered());
    })
}

#[test]
fn single_listen_first() {
    maybe_loom_model(|| {
        let (trigger, listener) = triggered::trigger();
        let thread_handle = thread::spawn(move || {
            trigger.trigger();
        });
        listener.wait();
        thread_handle.join().expect("Trigger thread panic");
    })
}

#[test]
fn buggy_loom() {
    maybe_loom_model(|| {
        let (sender, receiver) = loom::sync::mpsc::channel();
        let thread_handle = thread::spawn(move || {
            sender.send(()).unwrap();
        });
        receiver.recv().unwrap();
        thread_handle.join().expect("Trigger thread panic");
    })
}

#[test]
fn many_listeners_trigger_first() {
    maybe_loom_model(|| {
        let (trigger, listener) = triggered::trigger();
        let listener2 = listener.clone();

        trigger.trigger();

        let listener3 = listener2.clone();
        listener.wait();
        listener2.wait();
        listener3.wait();
    })
}

#[test]
fn many_listeners_listen_first() {
    let (trigger, listener) = triggered::trigger();

    // Spawn a bunch of tasks that all await their own clone of the trigger's listener
    let mut listeners = Vec::new();
    for _ in 0..103 {
        let listener = listener.clone();
        listeners.push(thread::spawn(move || {
            listener.wait();
            Instant::now()
        }));
    }

    // Pause a while to let most/all listeners end up in wait(), then trigger.
    #[cfg(not(loom))]
    thread::sleep(Duration::from_secs(1));
    let trigger_instant = Instant::now();
    trigger.trigger();

    // Make sure all tasks finish
    for listener in listeners {
        let wakeup_instant = listener.join().expect("Listener task panicked");
        assert!(trigger_instant < wakeup_instant);
    }
}

#[test]
fn wait_timeout() {
    let (_trigger, listener) = triggered::trigger();

    let start = Instant::now();
    let triggered = listener.wait_timeout(Duration::from_millis(100));
    let elapsed = start.elapsed();
    assert!(!triggered);
    assert!(
        elapsed > Duration::from_millis(99),
        "Timeout not within acceptable range: {:?}",
        elapsed
    );
    assert!(
        elapsed < Duration::from_millis(300),
        "Timeout not within acceptable range: {:?}",
        elapsed
    );
    assert!(!listener.is_triggered())
}

#[test]
fn wait_timeout_trigger_first() {
    let (trigger, listener) = triggered::trigger();

    trigger.trigger();

    let start = Instant::now();
    let triggered = listener.wait_timeout(Duration::from_millis(1000));
    assert!(triggered);
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_millis(10));
}

#[test]
fn wait_timeout_triggered_while_waiting() {
    let (trigger, listener) = triggered::trigger();

    // Spawn a bunch of tasks that all await their own clone of the trigger's listener
    let mut listeners = Vec::new();
    for _ in 0..103 {
        let listener = listener.clone();
        listeners.push(thread::spawn(move || {
            let triggered = listener.wait_timeout(Duration::from_millis(1000));
            assert!(triggered);
            Instant::now()
        }));
    }

    // Pause a while to let most/all listeners end up in wait(), then trigger.
    #[cfg(not(loom))]
    thread::sleep(Duration::from_millis(100));

    let trigger_instant = Instant::now();
    trigger.trigger();

    // Make sure all tasks finish
    for listener in listeners {
        let wakeup_instant = listener.join().expect("Listener task panicked");
        assert!(trigger_instant < wakeup_instant);
    }
}

#[test]
fn is_triggered() {
    let (trigger, listener) = triggered::trigger();
    assert!(!trigger.is_triggered());
    assert!(!listener.is_triggered());
    trigger.trigger();
    assert!(trigger.is_triggered());
    assert!(listener.is_triggered());

    // Check so a second trigger does not do anything unexpected.
    trigger.trigger();
    assert!(trigger.is_triggered());
    assert!(listener.is_triggered());
}
