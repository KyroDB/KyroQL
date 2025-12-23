use crate::belief::Belief;
use crate::confidence::BeliefId;
use crate::inference::ConflictResolutionPolicy;

/// Decision produced by applying a policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    /// A winning belief was selected.
    Selected(BeliefId),

    /// Policy refused to pick a winner.
    Unresolved,
}

/// Apply a conflict-resolution policy to a non-empty belief set.
///
/// `beliefs` should already be filtered to the relevant scope (e.g. same
/// `(entity, predicate)` and correct `as_of`).
#[must_use]
pub fn apply_conflict_policy(policy: &ConflictResolutionPolicy, beliefs: &[Belief]) -> PolicyDecision {
    if beliefs.is_empty() {
        return PolicyDecision::Unresolved;
    }

    match policy {
        ConflictResolutionPolicy::ExplicitConflict => PolicyDecision::Unresolved,
        ConflictResolutionPolicy::HighestConfidence => {
            let mut best = &beliefs[0];
            for b in &beliefs[1..] {
                if b.confidence.value() > best.confidence.value() {
                    best = b;
                } else if b.confidence.value() == best.confidence.value() {
                    // Deterministic tie-breaker: newest tx_time, then BeliefId.
                    if b.tx_time > best.tx_time {
                        best = b;
                    } else if b.tx_time == best.tx_time && b.id.to_string() < best.id.to_string() {
                        best = b;
                    }
                }
            }
            PolicyDecision::Selected(best.id)
        }
        ConflictResolutionPolicy::LatestWins => {
            let mut best = &beliefs[0];
            for b in &beliefs[1..] {
                if b.tx_time > best.tx_time {
                    best = b;
                } else if b.tx_time == best.tx_time && b.id.to_string() < best.id.to_string() {
                    best = b;
                }
            }
            PolicyDecision::Selected(best.id)
        }
        ConflictResolutionPolicy::SourcePriority { priority } => {
            let priority = priority.as_slice();
            let rank = |b: &Belief| {
                let sid = b.source.source_id();
                priority
                    .iter()
                    .position(|p| *p == sid)
                    .unwrap_or(usize::MAX)
            };

            let mut best = &beliefs[0];
            let mut best_rank = rank(best);

            for b in &beliefs[1..] {
                let r = rank(b);
                if r < best_rank {
                    best = b;
                    best_rank = r;
                } else if r == best_rank {
                    // Tie-breaker: higher confidence, then newest tx_time, then BeliefId.
                    if b.confidence.value() > best.confidence.value() {
                        best = b;
                    } else if b.confidence.value() == best.confidence.value() {
                        if b.tx_time > best.tx_time {
                            best = b;
                        } else if b.tx_time == best.tx_time && b.id.to_string() < best.id.to_string() {
                            best = b;
                        }
                    }
                }
            }

            PolicyDecision::Selected(best.id)
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use crate::confidence::Confidence;
    use crate::entity::EntityId;
    use crate::source::Source;
    use crate::time::TimeRange;
    use crate::value::Value;

    use super::*;

    fn belief_with(conf: f32, tx_time: chrono::DateTime<chrono::Utc>, source: Source) -> Belief {
        Belief {
            id: BeliefId::new(),
            subject: EntityId::new(),
            predicate: "p".to_string(),
            value: Value::String("v".to_string()),
            confidence: Confidence::from_agent(conf, "t").unwrap(),
            source,
            valid_time: TimeRange::forever(),
            tx_time,
            reason: None,
            consistency_status: crate::belief::ConsistencyStatus::Verified,
            supersedes: None,
            superseded_by: None,
            embedding: None,
        }
    }

    #[test]
    fn highest_confidence_picks_max() {
        let b1 = belief_with(0.5, Utc::now(), Source::agent("a", None::<String>));
        let b2 = belief_with(0.9, Utc::now(), Source::agent("b", None::<String>));
        let decision = apply_conflict_policy(&ConflictResolutionPolicy::HighestConfidence, &[b1.clone(), b2.clone()]);
        assert_eq!(decision, PolicyDecision::Selected(b2.id));
    }

    #[test]
    fn latest_wins_picks_newest() {
        let now = Utc::now();
        let b1 = belief_with(0.9, now, Source::agent("a", None::<String>));
        let b2 = belief_with(0.1, now + chrono::Duration::seconds(5), Source::agent("b", None::<String>));
        let decision = apply_conflict_policy(&ConflictResolutionPolicy::LatestWins, &[b1.clone(), b2.clone()]);
        assert_eq!(decision, PolicyDecision::Selected(b2.id));
    }

    #[test]
    fn explicit_conflict_never_picks() {
        let b = belief_with(0.9, Utc::now(), Source::agent("a", None::<String>));
        let decision = apply_conflict_policy(&ConflictResolutionPolicy::ExplicitConflict, &[b]);
        assert_eq!(decision, PolicyDecision::Unresolved);
    }
}
