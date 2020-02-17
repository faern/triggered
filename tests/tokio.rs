use std::thread;
use std::time::{Duration, Instant};

#[tokio::test]
async fn single_trigger_first() {
    let (trigger, listener) = triggered::trigger();
    trigger.trigger();
    listener.await;
}

#[tokio::test]
async fn single_listen_first() {
    let (trigger, listener) = triggered::trigger();
    let thread_handle = thread::spawn(move || {
        // This is a thread race. But hopefully the trigger will happen after the main thread
        // started waiting.
        thread::sleep(Duration::from_secs(1));
        trigger.trigger();
    });
    listener.await;
    thread_handle.join().expect("Trigger thread panic");
}

#[tokio::test]
async fn many_listeners_trigger_first() {
    let (trigger, listener) = triggered::trigger();
    let listener2 = listener.clone();

    trigger.trigger();

    let listener3 = listener2.clone();
    listener.await;
    listener2.await;
    listener3.await;
}

#[tokio::test]
async fn many_listeners_listen_first() {
    let (trigger, listener) = triggered::trigger();

    // Spawn a bunch of tasks that all await their own clone of the trigger's listener
    let mut listeners = Vec::new();
    for _ in 0..103 {
        let listener = listener.clone();
        listeners.push(tokio::spawn(async {
            listener.await;
            Instant::now()
        }));
    }

    // Pause a while to let most/all listener tasks run their first poll. Then trigger.
    thread::sleep(Duration::from_secs(1));
    let trigger_instant = Instant::now();
    trigger.trigger();

    // Make sure all tasks finish
    for listener in listeners {
        let wakeup_instant = listener.await.expect("Listener task panicked");
        assert!(trigger_instant < wakeup_instant);
    }
}

#[tokio::test]
async fn many_sync_and_async_listeners_listen_first() {
    let (trigger, listener) = triggered::trigger();

    // Spawn a bunch of tasks that all await their own clone of the trigger's listener
    let mut sync_listeners = Vec::new();
    let mut async_listeners = Vec::new();
    for _ in 0..103 {
        let listener1 = listener.clone();
        sync_listeners.push(thread::spawn(move || {
            listener1.wait();
            Instant::now()
        }));

        let listener2 = listener.clone();
        async_listeners.push(tokio::spawn(async {
            listener2.await;
            Instant::now()
        }));
    }

    // Pause a while to let most/all listener tasks start waiting. Then trigger.
    thread::sleep(Duration::from_secs(1));
    let trigger_instant = Instant::now();
    trigger.trigger();

    // Make sure all tasks finish
    for listener in sync_listeners {
        let wakeup_instant = listener.join().expect("Listener task panicked");
        assert!(trigger_instant < wakeup_instant);
    }
    for listener in async_listeners {
        let wakeup_instant = listener.await.expect("Listener task panicked");
        assert!(trigger_instant < wakeup_instant);
    }
}
