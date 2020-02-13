use std::thread;
use std::time::Duration;

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
        }));
    }

    // Pause a while to let most/all listener tasks run their first poll. Then trigger.
    thread::sleep(Duration::from_secs(1));
    trigger.trigger();

    // Make sure all tasks finish
    for listener in listeners {
        listener.await.expect("Listener task panicked");
    }
}
