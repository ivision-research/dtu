use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crossbeam::channel::{bounded, Receiver, Sender};

/// Allows cancelling active tasks
#[derive(Clone)]
pub struct TaskCanceller {
    cancel: Arc<AtomicBool>,
}

impl Drop for TaskCanceller {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed)
    }
}

impl TaskCanceller {
    pub fn new() -> (Self, TaskCancelCheck) {
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel = Arc::clone(&cancelled);
        (
            Self { cancel },
            TaskCancelCheck {
                cancelled: Arc::clone(&cancelled),
            },
        )
    }

    /// Cancel the task. This can be invoked more than once, but only the first
    /// invocation matters.
    pub fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub struct TaskCancelCheck {
    cancelled: Arc<AtomicBool>,
}

impl TaskCancelCheck {
    /// Check to see if the task has been cancelled
    pub fn was_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
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
