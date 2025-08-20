use crossbeam::channel::{bounded, unbounded, Receiver, Sender, TryRecvError};

/// Allows cancelling active tasks
pub struct TaskCanceller {
    tx: Option<Sender<()>>,
}

impl TaskCanceller {
    pub fn new() -> (Self, TaskCancelCheck) {
        let (tx, rx) = unbounded();
        (Self { tx: Some(tx) }, TaskCancelCheck { rx })
    }

    /// Cancel the task. This can be invoked more than once, but it is
    /// essentially a noop after the first invocation.
    pub fn cancel(&mut self) {
        self.tx.take().map(|tx| drop(tx));
    }
}

pub struct TaskCancelCheck {
    rx: Receiver<()>,
}

impl TaskCancelCheck {
    /// Check to see if the task has been cancelled
    pub fn was_cancelled(&self) -> bool {
        match self.rx.try_recv() {
            Err(TryRecvError::Disconnected) => true,
            _ => false,
        }
    }

    /// Convenience wrapper to check [was_cancelled] and return an error if
    /// true
    pub fn check<T>(&self, val: T) -> Result<(), T> {
        if self.was_cancelled() {
            Err(val)
        } else {
            Ok(())
        }
    }
}

pub trait EventMonitor<T>: Send + Sync {
    fn on_event(&self, evt: T);
}

impl<U> EventMonitor<U> for Box<dyn EventMonitor<U>> {
    fn on_event(&self, evt: U) {
        self.as_ref().on_event(evt)
    }
}

impl<T, U> EventMonitor<U> for Box<T>
where
    T: EventMonitor<U>,
{
    fn on_event(&self, evt: U) {
        self.as_ref().on_event(evt)
    }
}

/// An [EventMonitor] that is just a noop
pub struct NoopMonitor;

impl<T> EventMonitor<T> for NoopMonitor {
    fn on_event(&self, _evt: T) {
        // noop
    }
}

impl NoopMonitor {
    pub fn new() -> Self {
        Self {}
    }
}

/// An [EventMonitor] that just dumps the events onto a channel.
pub struct ChannelEventMonitor<T>
where
    T: Sync + Send,
{
    chan: Sender<T>,
}

impl<T> ChannelEventMonitor<T>
where
    T: Sync + Send,
{
    pub fn create() -> (Self, Receiver<T>) {
        Self::create_with_bound(16)
    }

    pub fn create_with_bound(bound: usize) -> (Self, Receiver<T>) {
        let (tx, rx) = bounded(bound);
        (Self::new(tx), rx)
    }

    pub fn new(chan: Sender<T>) -> Self {
        Self { chan }
    }
}

impl<T> EventMonitor<T> for ChannelEventMonitor<T>
where
    T: Sync + Send,
{
    fn on_event(&self, evt: T) {
        let _ = self.chan.send(evt);
    }
}
