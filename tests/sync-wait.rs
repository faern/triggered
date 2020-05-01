use std::thread;
use std::time::{Duration, Instant};

#[test]
fn single_trigger_first() {
    let (trigger, listener) = triggered::trigger();
    trigger.trigger();
    listener.wait();
}

#[test]
fn single_listen_first() {
    let (trigger, listener) = triggered::trigger();
    let thread_handle = thread::spawn(move || {
        // This is a thread race. But hopefully the trigger will happen after the main thread
        // started waiting.
        thread::sleep(Duration::from_secs(1));
        trigger.trigger();
    });
    listener.wait();
    thread_handle.join().expect("Trigger thread panic");
}

#[test]
fn many_listeners_trigger_first() {
    let (trigger, listener) = triggered::trigger();
    let listener2 = listener.clone();

    trigger.trigger();

    let listener3 = listener2.clone();
    listener.wait();
    listener2.wait();
    listener3.wait();
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
