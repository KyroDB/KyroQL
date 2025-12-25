//! Monitor dispatcher worker.
//!
//! This module owns trigger registrations and dispatches `MonitorEvent`s to
//! per-subscription streams. ASSERT commits enqueue observations using a bounded
//! channel and never block the caller.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use chrono::{DateTime, Utc};
use crossbeam_channel::{bounded, select, Receiver, Sender, TrySendError};
use serde_json;

use crate::error::{ExecutionError, KyroError, KyroResult, ValidationError};
use crate::storage::BeliefStore;
use crate::value::Value;

use super::matcher::{AssertObservation, MatchOutput, TriggerMatcher};
use super::stream::MonitorStream;
use super::triggers::{MonitorEvent, SubscriptionId, Trigger, TriggerId};

#[allow(missing_docs)]
#[derive(Debug, Clone)]
pub struct MonitorSystemConfig {
    /// Max queued observation events before backpressure applies.
    pub observation_queue_capacity: usize,
    /// Max queued control messages (register/unregister).
    pub control_queue_capacity: usize,
    /// Per-subscription stream buffer capacity.
    pub stream_capacity: usize,
}

impl Default for MonitorSystemConfig {
    fn default() -> Self {
        Self {
            observation_queue_capacity: 4096,
            control_queue_capacity: 1024,
            stream_capacity: 1024,
        }
    }
}

#[allow(missing_docs)]
#[derive(Debug)]
pub struct MonitorRegistration {
    pub subscription_id: SubscriptionId,
    pub trigger_ids: Vec<TriggerId>,
    pub stream: MonitorStream,
}

#[derive(Debug)]
pub(crate) enum ControlMsg {
    Register {
        subscription_id: SubscriptionId,
        triggers: Vec<(TriggerId, Trigger)>,
        expires_at: Option<DateTime<Utc>>,
        stream_tx: Sender<MonitorEvent>,
        reply: Sender<KyroResult<()>>,
    },
    Unregister {
        subscription_id: SubscriptionId,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct ObserveMsg {
    pub obs: AssertObservation,
}

#[derive(Debug)]
struct TriggerEntry {
    id: TriggerId,
    trigger: Trigger,
    expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug)]
struct SubscriptionEntry {
    tx: Sender<MonitorEvent>,
    triggers: Vec<TriggerEntry>,
}

/// Monitor system: owns trigger registrations and dispatches events.
///
/// This system runs a dedicated worker thread. ASSERT commits enqueue an
/// `AssertObservation` using non-blocking `try_send` to avoid stalling callers.
#[allow(missing_docs)]
#[derive(Debug)]
pub struct MonitorSystem {
    cfg: MonitorSystemConfig,
    control_tx: Sender<ControlMsg>,
    observe_tx: Sender<ObserveMsg>,
    dropped_observations: AtomicU64,
    dropped_events: Arc<AtomicU64>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl MonitorSystem {
    pub fn new(cfg: MonitorSystemConfig, beliefs: Arc<dyn BeliefStore>) -> Self {
        let observation_queue_capacity = cfg.observation_queue_capacity.max(1);
        let control_queue_capacity = cfg.control_queue_capacity.max(1);

        let (control_tx, control_rx) = bounded::<ControlMsg>(control_queue_capacity);
        let (observe_tx, observe_rx) = bounded::<ObserveMsg>(observation_queue_capacity);

        let dropped_observations = AtomicU64::new(0);
        let dropped_events = Arc::new(AtomicU64::new(0));

        let matcher = TriggerMatcher::new(Arc::clone(&beliefs));

        let thread_cfg = cfg.clone();
        let thread_dropped_events = Arc::clone(&dropped_events);
        let join = thread::Builder::new()
            .name("kyroql-monitor".to_string())
            .spawn(move || worker_loop(thread_cfg, matcher, thread_dropped_events, control_rx, observe_rx))
            .expect("failed to spawn kyroql monitor worker");

        Self {
            cfg,
            control_tx,
            observe_tx,
            dropped_observations,
            dropped_events,
            join: Mutex::new(Some(join)),
        }
    }

    /// Register triggers and obtain a stream for matching events.
    pub fn register(&self, triggers: Vec<Trigger>, expires_at: Option<DateTime<Utc>>) -> KyroResult<MonitorRegistration> {
        if triggers.is_empty() {
            return Err(KyroError::Validation(ValidationError::MissingField {
                field: "trigger".to_string(),
            }));
        }

        if let Some(exp) = expires_at {
            if exp <= Utc::now() {
                return Err(KyroError::Validation(ValidationError::InvalidSimulationConstraints {
                    reason: "monitor expires_at must be in the future".to_string(),
                }));
            }
        }

        let subscription_id = SubscriptionId::new();

        let (stream_tx, stream_rx) = bounded::<MonitorEvent>(self.cfg.stream_capacity.max(1));

        let mut trigger_ids = Vec::with_capacity(triggers.len());
        let mut trigger_pairs = Vec::with_capacity(triggers.len());
        for t in triggers {
            let id = TriggerId::new();
            trigger_ids.push(id);
            trigger_pairs.push((id, t));
        }

        let stream = MonitorStream::new(subscription_id, stream_rx, self.control_tx.clone());
        let reg = MonitorRegistration {
            subscription_id,
            trigger_ids,
            stream,
        };

        let (reply_tx, reply_rx) = bounded::<KyroResult<()>>(1);
        self.control_tx
            .send(ControlMsg::Register {
                subscription_id,
                triggers: trigger_pairs,
                expires_at,
                stream_tx,
                reply: reply_tx,
            })
            .map_err(|_| {
                KyroError::Execution(ExecutionError::Disconnected {
                    path: "monitor_control".to_string(),
                })
            })?;

        // Wait for ack (or error) and return the stream registration.
        reply_rx.recv().map_err(|_| {
            KyroError::Execution(ExecutionError::Disconnected {
                path: "monitor_control".to_string(),
            })
        })??;

        Ok(reg)
    }

    /// Non-blocking observation enqueue.
    pub fn observe_assert(&self, obs: AssertObservation) {
        match self.observe_tx.try_send(ObserveMsg { obs }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                self.dropped_observations.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    #[must_use]
    pub fn dropped_observations(&self) -> u64 {
        self.dropped_observations.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn dropped_events(&self) -> u64 {
        self.dropped_events.load(Ordering::Relaxed)
    }

    /// Translate a `Value::Structured` trigger specification to concrete triggers.
    pub fn triggers_from_threshold_value(
        &self,
        threshold: &Value,
        entity_filters: Option<&[crate::entity::EntityId]>,
        predicate_filters: Option<&[String]>,
        pattern_filters: Option<&[crate::pattern::PatternId]>,
    ) -> KyroResult<Vec<Trigger>> {
        let mut out = Vec::new();

        if let Some(pats) = pattern_filters {
            out.extend(pats.iter().copied().map(|pattern_id| Trigger::PatternViolation { pattern_id }));
        }

        match threshold {
            Value::Null => {}
            Value::Float(v) => {
                let thr = (*v as f32).abs();
                out.extend(build_confidence_shift_triggers(thr, entity_filters, predicate_filters));
            }
            Value::Int(v) => {
                let thr = (*v as f32).abs();
                out.extend(build_confidence_shift_triggers(thr, entity_filters, predicate_filters));
            }
            Value::Structured(v) => {
                // Accept either a single trigger object or an array of triggers.
                if v.is_array() {
                    let triggers: Vec<Trigger> = serde_json::from_value(v.clone()).map_err(|e| {
                        KyroError::Validation(ValidationError::InvalidSimulationConstraints {
                            reason: format!("invalid trigger array: {e}"),
                        })
                    })?;
                    out.extend(triggers);
                } else {
                    let trigger: Trigger = serde_json::from_value(v.clone()).map_err(|e| {
                        KyroError::Validation(ValidationError::InvalidSimulationConstraints {
                            reason: format!("invalid trigger object: {e}"),
                        })
                    })?;
                    out.push(trigger);
                }
            }
            other => {
                return Err(KyroError::Validation(ValidationError::InvalidSimulationConstraints {
                    reason: format!("monitor threshold must be float/int/structured/null, got {other:?}"),
                }));
            }
        }

        Ok(out)
    }
}

impl Drop for MonitorSystem {
    fn drop(&mut self) {
        // Close channels first so the worker can terminate, then join.
        // This avoids deadlocking by waiting on a worker that's blocked on recv.
        let (dummy_control_tx, _) = bounded::<ControlMsg>(1);
        let old_control = std::mem::replace(&mut self.control_tx, dummy_control_tx);
        drop(old_control);

        let (dummy_observe_tx, _) = bounded::<ObserveMsg>(1);
        let old_observe = std::mem::replace(&mut self.observe_tx, dummy_observe_tx);
        drop(old_observe);

        if let Ok(mut guard) = self.join.lock() {
            if let Some(handle) = guard.take() {
                // Do not join here.
                //
                // Callers may keep `MonitorStream` alive beyond the engine/system lifetime,
                // and `MonitorStream` holds a clone of `control_tx`. If we join here, the
                // worker can stay alive (channel remains open) and Drop would deadlock.
                //
                // Detaching is safe: the worker exits once the last sender is dropped.
                drop(handle);
            }
        }
    }
}

fn build_confidence_shift_triggers(
    threshold: f32,
    entity_filters: Option<&[crate::entity::EntityId]>,
    predicate_filters: Option<&[String]>,
) -> Vec<Trigger> {
    let mut out = Vec::new();

    let entities: Vec<Option<crate::entity::EntityId>> = match entity_filters {
        None => vec![None],
        Some(v) if v.is_empty() => vec![None],
        Some(v) => v.iter().copied().map(Some).collect(),
    };

    let predicates: Vec<Option<String>> = match predicate_filters {
        None => vec![None],
        Some(v) if v.is_empty() => vec![None],
        Some(v) => v
            .iter()
            .map(|s| {
                let t = s.trim().to_string();
                if t.is_empty() {
                    None
                } else {
                    Some(t)
                }
            })
            .collect(),
    };

    // Safety: keep worst-case registration bounded.
    const MAX_TRIGGERS: usize = 4096;

    for e in &entities {
        for p in &predicates {
            if out.len() >= MAX_TRIGGERS {
                return out;
            }
            out.push(Trigger::ConfidenceShift {
                entity_id: *e,
                predicate: p.clone(),
                threshold,
            });
        }
    }

    out
}

fn worker_loop(
    _cfg: MonitorSystemConfig,
    matcher: TriggerMatcher,
    dropped_events: Arc<AtomicU64>,
    control_rx: Receiver<ControlMsg>,
    observe_rx: Receiver<ObserveMsg>,
) {
    let mut subs: HashMap<SubscriptionId, SubscriptionEntry> = HashMap::new();

    let mut control_closed = false;
    let mut observe_closed = false;

    loop {
        select! {
            recv(control_rx) -> msg => {
                match msg {
                    Ok(ControlMsg::Register { subscription_id, triggers, expires_at, stream_tx, reply }) => {
                        let trigger_entries: Vec<TriggerEntry> = triggers
                            .into_iter()
                            .map(|(id, trigger)| TriggerEntry { id, trigger, expires_at })
                            .collect();

                        subs.insert(subscription_id, SubscriptionEntry { tx: stream_tx, triggers: trigger_entries });

                        let _ = reply.send(Ok(()));
                    }
                    Ok(ControlMsg::Unregister { subscription_id }) => {
                        subs.remove(&subscription_id);
                    }
                    Err(_) => {
                        control_closed = true;
                    }
                }
            }
            recv(observe_rx) -> msg => {
                match msg {
                    Ok(ObserveMsg { obs }) => {
                        // Dispatch observation to matching triggers.
                        let now = Utc::now();
                        for sub in subs.values_mut() {
                            // Filter expired triggers in-place.
                            sub.triggers.retain(|t| t.expires_at.map(|e| e > now).unwrap_or(true));

                            for t in &sub.triggers {
                                match matcher.evaluate(&t.trigger, &obs) {
                                    Ok(MatchOutput::NoMatch) => {}
                                    Ok(MatchOutput::Match(payload)) => {
                                        let Ok(event) = MonitorEvent::new(t.id, t.trigger.clone(), payload) else {
                                            // Internal invariant violation (matcher produced payload inconsistent with trigger).
                                            // Fail closed by dropping the event.
                                            dropped_events.fetch_add(1, Ordering::Relaxed);
                                            continue;
                                        };

                                        // Never block monitor thread: drop if subscriber is slow.
                                        match sub.tx.try_send(event) {
                                            Ok(()) => {}
                                            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                                                dropped_events.fetch_add(1, Ordering::Relaxed);
                                            }
                                        }
                                    }
                                    Err(_) => {
                                        // Storage/matcher error: fail closed (no event).
                                    }
                                }
                            }
                        }
                    }
                    Err(_) => {
                        observe_closed = true;
                    }
                }
            }
            default(Duration::from_millis(50)) => {
                // Periodic cleanup: drop fully expired subscriptions.
                let now = Utc::now();
                subs.retain(|_, sub| {
                    sub.triggers.retain(|t| t.expires_at.map(|e| e > now).unwrap_or(true));
                    !sub.triggers.is_empty()
                });
            }
        }

        if control_closed && observe_closed {
            break;
        }
    }
}
