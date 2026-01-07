pub mod fuzz;
pub mod pull;
pub mod selinux;
pub mod smalisa;

pub mod task;
pub use task::{ChannelEventMonitor, EventMonitor, NoopMonitor, TaskCancelCheck, TaskCanceller};

use crossbeam::channel::{Receiver, RecvTimeoutError, SendTimeoutError, Sender};
use std::time::Duration;

use crate::{Error, Result};

/// Used to send something on a channel and check the TaskCancelCheck.
///
/// Returns false if the channel was disconnected.
pub fn cancelable_send<T>(cancel: &TaskCancelCheck, mut value: T, tx: &Sender<T>) -> Result<bool> {
    while !cancel.was_cancelled() {
        match tx.send_timeout(value, Duration::from_millis(200)) {
            Err(SendTimeoutError::Timeout(ret)) => value = ret,
            Err(SendTimeoutError::Disconnected(_)) => return Ok(false),
            _ => return Ok(true),
        };
    }
    Err(Error::Cancelled)
}

/// Used to receive something from a channel and check the TaskCancelCheck.
///
/// Returns Ok(None) if the channel was disconnected
pub fn cancelable_recv<T>(cancel: &TaskCancelCheck, rx: &Receiver<T>) -> Result<Option<T>> {
    while !cancel.was_cancelled() {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return Ok(None),
            Ok(v) => return Ok(Some(v)),
        };
    }
    Err(Error::Cancelled)
}
