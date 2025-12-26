use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};

use crate::error::{ExecutionError, KyroError, KyroResult};

use super::dispatcher::ControlMsg;
use super::triggers::{MonitorEvent, SubscriptionId};

/// A subscription stream for monitor events.
///
/// Dropping this stream attempts best-effort unregistration.
///

#[derive(Debug)]
pub struct MonitorStream {
    subscription_id: SubscriptionId,
    rx: Receiver<MonitorEvent>,
    control_tx: Sender<ControlMsg>,
    unregistered: AtomicBool,
}

impl MonitorStream {
    pub(crate) fn new(
        subscription_id: SubscriptionId,
        rx: Receiver<MonitorEvent>,
        control_tx: Sender<ControlMsg>,
    ) -> Self {
        Self {
            subscription_id,
            rx,
            control_tx,
            unregistered: AtomicBool::new(false),
        }
    }

    /// The subscription id backing this stream.
    #[must_use]
    pub const fn subscription_id(&self) -> SubscriptionId {
        self.subscription_id
    }

    /// Best-effort explicit unregistration.
    ///
    /// This is non-blocking and idempotent. After the subscription is removed on the
    /// dispatcher side, the stream will eventually become disconnected.
    pub fn unsubscribe(&self) {
        if self.unregistered.swap(true, Ordering::AcqRel) {
            return;
        }

        let _ = self.control_tx.try_send(ControlMsg::Unregister {
            subscription_id: self.subscription_id,
        });
    }

    /// Receive the next event (blocking).
    pub fn recv(&self) -> KyroResult<MonitorEvent> {
        self.rx.recv().map_err(|_| {
            KyroError::Execution(ExecutionError::Disconnected {
                path: "monitor_stream".to_string(),
            })
        })
    }

    /// Receive the next event with a timeout.
    pub fn recv_timeout(&self, timeout: Duration) -> KyroResult<MonitorEvent> {
        self.rx.recv_timeout(timeout).map_err(|err| match err {
            RecvTimeoutError::Timeout => KyroError::Execution(ExecutionError::Timeout {
                duration_ms: timeout.as_millis().min(u128::from(u64::MAX)) as u64,
            }),
            RecvTimeoutError::Disconnected => KyroError::Execution(ExecutionError::Disconnected {
                path: "monitor_stream".to_string(),
            }),
        })
    }
}

impl Drop for MonitorStream {
    fn drop(&mut self) {
        // Best-effort: do not block on shutdown.
        if !self.unregistered.swap(true, Ordering::AcqRel) {
            let _ = self.control_tx.try_send(ControlMsg::Unregister {
                subscription_id: self.subscription_id,
            });
        }
    }
}
