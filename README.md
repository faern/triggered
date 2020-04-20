# triggered

Triggers for one time events between tasks and threads.

The mechanism consists of two types, the `Trigger` and the `Listener`. They come together
as a pair. Much like the sender/receiver pair of a channel. The trigger half has a
`Trigger::trigger` method that will make all tasks/threads waiting on
a listener continue executing.
The listener both has a sync `Listener::wait` method, and it also implements
`Future<Output = ()>` for async support.

Both the `Trigger` and `Listener` can be cloned. So any number of trigger instances can
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

    let task = tokio::spawn(async {
        // Blocks until `trigger.trigger()` below
        listener.await;

        println!("Triggered async task");
    });

    // This will make any thread blocked in `Listener::wait()` or async task awaiting the
    // listener continue execution again.
    trigger.trigger();

    let _ = task.await;
}
```

An example showing a trigger/listener pair being used to gracefully shut down some async
server instances on a Ctrl-C event, where only an immutable `Fn` closure is accepted:

```rust

#[tokio::main]
async fn main() -> Result<(), Error> {
    let (shutdown_trigger, shutdown_signal1) = triggered::trigger();

    // A sync `Fn` closure will trigger the trigger when the user hits Ctrl-C
    ctrlc::set_handler(move || {
        shutdown_trigger.trigger();
    }).expect("Error setting Ctrl-C handler");

    // If the server library has support for something like a shutdown signal:
    let shutdown_signal2 = shutdown_signal1.clone();
    let server1_task = tokio::spawn(async move {
        SomeServer::new().serve_with_shutdown_signal(shutdown_signal1).await;
    });

    // Or just select between the long running future and the signal to abort it
    tokio::select! {
        server_result = SomeServer::new().serve() => {
            eprintln!("Server error: {:?}", server_result);
        }
        _ = shutdown_signal2 => {}
    }

    let _ = server1_task.await;
    Ok(())
}
```

## Rust Compatibility

Will work with at least the two latest stable Rust releases. This gives users at least six
weeks to upgrade their Rust toolchain after a new stable is released.

The current MSRV can be seen in `travis.yml`. Any change to the MSRV will be considered a
breaking change and listed in the [changelog](CHANGELOG.md).

## Comparison with similar primitives

### Channels

The event triggering primitives in this library is somewhat similar to channels. The main
difference and why I developed this library is that

The listener is somewhat similar to a `futures::channel::oneshot::Receiver<()>`. But it:
 * Is not fallible - Implements `Future<Output = ()>` instead of
   `Future<Output = Result<T, Canceled>>`
 * Implements `Clone` - Any number of listeners can wait for the same event
 * Has a sync [`Listener::wait`] - Both synchronous threads, and asynchronous tasks can wait
   at the same time.

The trigger, when compared to a `futures::channel::oneshot::Sender<()>` has the differences
that it:
 * Is not fallible - The trigger does not care if there are any listeners left
 * Does not consume itself on send, instead takes `&self` - So can be used
   in situations where it is not owned or not mutable. For example in `Drop` implementations
   or callback closures that are limited to `Fn` or `FnMut`.

### `futures::future::Abortable`

One use case of these triggers is to abort futures when some event happens. See examples above.
The differences include:
 * A single handle can abort any number of futures
 * Some futures are not properly cleaned up when just dropped the way `Abortable` does it.
   These libraries sometimes allows creating their futures with a shutdown signal that triggers
   a clean abort. Something like `serve_with_shutdown(signal: impl Future<Output = ()>)`.
