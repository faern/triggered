# triggered

Simple triggers that allows triggering a one time event in another task/thread.

The mechanism consists of two types, the [`Trigger`] and the [`Listener`]. They come together
as a pair. Much like the sender/receiver pair of a channel. The trigger half has a
[`Trigger::trigger`] method that will make all tasks/threads waiting on
a listener continue executing.
The listener both has a sync [`Listener::wait`] method, and it also implements
`Future<Output = ()>` for async support.

Both the [`Trigger`] and [`Listener`] can be cloned. So any number of trigger instances can
trigger any number of waiting listeners. When any one trigger instance belonging to the pair is
triggered, all the waiting listeners will be unblocked. Waiting on a listener whose
trigger already went off will return instantly. So each trigger/listener pair can only be fired
once.

This crate does not use any `unsafe` code.

## Examples

A trivial example showing the basic usage:

```rust
#[tokio::main]
async fn main() {
    let (trigger, listener) = triggered::trigger();
    tokio::spawn(async {
        listener.await;
        println!("Trigger went off!");
    });
    trigger.trigger();
}
```

An example showing a trigger/listener pair being used to gracefully shut down some async
server instances on a Ctrl+C event, where only an immutable `Fn` closure is accepted:

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error> {
    let (shutdown_trigger, shutdown_signal) = triggered::trigger();

    ctrlc::set_handler(move || {
        shutdown_trigger.trigger();
    }).expect("Error setting Ctrl-C handler");

    let server1_task = tokio::spawn(async {
        SomeServer::new().serve_with_shutdown_signal(shutdown_signal.clone());
    });
    let server2_task = tokio::spawn(async {
        SomeOServer::new().serve_with_shutdown_signal(shutdown_signal);
    });

    server1_task.await?;
    server2_task.await?;
}
```

## Comparison to channels

The listener is somewhat similar to a `futures::channel::oneshot::Receiver<()>`. But it:
 * Is not fallible - Implements `Future<()>` instead of `Future<(), Canceled>`
 * Implements `Clone` - Any number of listeners can wait for the same oneshot event
 * Has a sync [`Listener::wait`] - Both synchronous threads, and asynchronous tasks can wait
   at the same time.

The trigger, when compared to a `futures::channel::oneshot::Sender<()>` has the differences
that it:
 * Is not fallible - The trigger does not care if there are any listeners left
 * Does not consume itself on send, instead takes `&self` - So can be used
   in situations where it is not owned or not mutable. For example in `Drop` implementations
   or callback closures that are limited to `Fn` or `FnMut`

License: MIT OR Apache-2.0
