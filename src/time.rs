//! Temporal types for bitemporal data management.
//!
//! KyroQL uses bitemporal semantics:
//! - **Valid Time**: When is this belief true in reality?
//! - **Transaction Time**: When did the system learn this?

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::error::ValidationError;

/// A range of time (half-open interval: [from, to)).
///
/// Used to represent the valid time of a belief—when it is true in reality.
///
/// # Examples
///
/// ```
/// use kyroql::TimeRange;
/// use chrono::Utc;
///
/// // Create an open-ended range starting now
/// let range = TimeRange::from_now();
/// assert!(range.is_open_ended());
///
/// // Check if a time falls within the range
/// assert!(range.contains(Utc::now()));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TimeRange {
    /// Start of the range (inclusive).
    pub from: DateTime<Utc>,

    /// End of the range (exclusive). None means open-ended.
    pub to: Option<DateTime<Utc>>,
}

impl TimeRange {
    /// Creates a time range from two timestamps.
    ///
    /// # Errors
    ///
    /// Returns `ValidationError::InvalidTimeRange` if `from >= to`.
    ///
    /// # Examples
    ///
    /// ```
    /// use kyroql::TimeRange;
    /// use chrono::{Utc, Duration};
    ///
    /// let now = Utc::now();
    /// let later = now + Duration::hours(1);
    /// let range = TimeRange::new(now, later).unwrap();
    /// ```
    pub fn new(from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Self, ValidationError> {
        if from >= to {
            return Err(ValidationError::InvalidTimeRange { from, to });
        }
        Ok(Self { from, to: Some(to) })
    }

    /// Creates an open-ended time range starting at the given time.
    #[must_use]
    pub const fn starting_at(from: DateTime<Utc>) -> Self {
        Self { from, to: None }
    }

    /// Creates an open-ended time range starting now.
    #[must_use]
    pub fn from_now() -> Self {
        Self {
            from: Utc::now(),
            to: None,
        }
    }

    /// Creates a time range starting now with a specified duration.
    ///
    /// # Panics
    ///
    /// Panics if `duration` is zero or negative.
    #[must_use]
    pub fn from_now_for(duration: Duration) -> Self {
        assert!(duration > Duration::zero(), "duration must be positive");
        let from = Utc::now();
        Self {
            from,
            to: Some(from + duration),
        }
    }

    /// Creates a time range for a specific instant (1 microsecond duration).
    #[must_use]
    pub fn instant(at: DateTime<Utc>) -> Self {
        Self {
            from: at,
            to: Some(at + Duration::microseconds(1)),
        }
    }

    /// Creates a time range representing "forever" (from epoch to open-ended).
    #[must_use]
    pub fn forever() -> Self {
        Self {
            from: DateTime::UNIX_EPOCH,
            to: None,
        }
    }

    pub const fn is_open_ended(&self) -> bool {
        self.to.is_none()
    }

    pub fn has_ended(&self) -> bool {
        match self.to {
            Some(to) => to <= Utc::now(),
            None => false,
        }
    }

    pub fn is_active(&self) -> bool {
        self.contains(Utc::now())
    }

    /// Check if a timestamp falls within this range [from, to).
    #[must_use]
    pub fn contains(&self, time: DateTime<Utc>) -> bool {
        time >= self.from && self.to.map_or(true, |to| time < to)
    }

    pub fn overlaps(&self, other: &Self) -> bool {
        let self_end = self.to.unwrap_or(DateTime::<Utc>::MAX_UTC);
        let other_end = other.to.unwrap_or(DateTime::<Utc>::MAX_UTC);
        self.from < other_end && other.from < self_end
    }

    /// Returns the intersection of two ranges, if any.
    #[must_use]
    pub fn intersection(&self, other: &Self) -> Option<Self> {
        if !self.overlaps(other) {
            return None;
        }

        let from = self.from.max(other.from);
        let to = match (self.to, other.to) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };

        Some(Self { from, to })
    }

    pub fn duration(&self) -> Option<Duration> {
        self.to.map(|to| to - self.from)
    }

    pub fn extend_by(&mut self, duration: Duration) {
        if let Some(to) = self.to.as_mut() {
            *to = *to + duration;
        }
    }

    /// Closes an open-ended range at the current time.
    /// Ensures the end never precedes the start by clamping to max(now, from).
    pub fn close_now(&mut self) {
        if self.to.is_none() {
            let now = Utc::now();
            let end = std::cmp::max(now, self.from);
            self.to = Some(end);
        }
    }

    /// Closes an open-ended range at the specified time.
    ///
    /// # Errors
    ///
    /// Returns `ValidationError::InvalidTimeRange` if the close time is before the start.
    pub fn close_at(&mut self, at: DateTime<Utc>) -> Result<(), ValidationError> {
        if at < self.from {
            return Err(ValidationError::InvalidTimeRange {
                from: self.from,
                to: at,
            });
        }
        self.to = Some(at);
        Ok(())
    }
}

impl Default for TimeRange {
    fn default() -> Self {
        Self::from_now()
    }
}

impl std::fmt::Display for TimeRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.to {
            Some(to) => write!(f, "[{} → {})", self.from, to),
            None => write!(f, "[{} → ∞)", self.from),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_time_range_new_valid() {
        let now = Utc::now();
        let later = now + Duration::hours(1);
        let range = TimeRange::new(now, later).unwrap();

        assert_eq!(range.from, now);
        assert_eq!(range.to, Some(later));
        assert!(!range.is_open_ended());
    }

    #[test]
    fn test_time_range_new_invalid() {
        let now = Utc::now();
        let earlier = now - Duration::hours(1);

        assert!(TimeRange::new(now, earlier).is_err());
        assert!(TimeRange::new(now, now).is_err()); // Same time is invalid
    }

    #[test]
    fn test_time_range_from_now() {
        let range = TimeRange::from_now();
        assert!(range.is_open_ended());
        assert!(range.is_active());
    }

    #[test]
    fn test_time_range_from_now_for() {
        let range = TimeRange::from_now_for(Duration::hours(1));
        assert!(!range.is_open_ended());
        assert!(range.is_active());
    }

    #[test]
    fn test_time_range_starting_at() {
        let time = Utc::now();
        let range = TimeRange::starting_at(time);
        assert!(range.is_open_ended());
        assert_eq!(range.from, time);
    }

    #[test]
    fn test_time_range_forever() {
        let range = TimeRange::forever();
        assert!(range.is_open_ended());
        assert!(range.contains(Utc::now()));
        assert!(range.contains(DateTime::UNIX_EPOCH));
    }

    #[test]
    fn test_time_range_contains() {
        let start = Utc::now();
        let end = start + Duration::hours(1);
        let range = TimeRange::new(start, end).unwrap();

        assert!(range.contains(start)); // Inclusive start
        assert!(range.contains(start + Duration::minutes(30)));
        assert!(!range.contains(end)); // Exclusive end
        assert!(!range.contains(start - Duration::hours(1)));
    }

    #[test]
    fn test_time_range_contains_open_ended() {
        let start = Utc::now() - Duration::hours(1);
        let range = TimeRange::starting_at(start);

        assert!(range.contains(start));
        assert!(range.contains(Utc::now()));
        assert!(range.contains(Utc::now() + Duration::days(365)));
    }

    #[test]
    fn test_time_range_overlaps() {
        let now = Utc::now();

        let range1 = TimeRange::new(now, now + Duration::hours(2)).unwrap();
        let range2 = TimeRange::new(now + Duration::hours(1), now + Duration::hours(3)).unwrap();
        let range3 = TimeRange::new(now + Duration::hours(3), now + Duration::hours(4)).unwrap();

        assert!(range1.overlaps(&range2));
        assert!(range2.overlaps(&range1));
        assert!(!range1.overlaps(&range3));
        assert!(!range3.overlaps(&range1));
    }

    #[test]
    fn test_time_range_overlaps_open_ended() {
        let now = Utc::now();

        let range1 = TimeRange::starting_at(now);
        let range2 = TimeRange::new(now + Duration::hours(1), now + Duration::hours(2)).unwrap();

        assert!(range1.overlaps(&range2));
        assert!(range2.overlaps(&range1));
    }

    #[test]
    fn test_time_range_intersection() {
        let now = Utc::now();

        let range1 = TimeRange::new(now, now + Duration::hours(3)).unwrap();
        let range2 = TimeRange::new(now + Duration::hours(1), now + Duration::hours(4)).unwrap();

        let intersection = range1.intersection(&range2).unwrap();
        assert_eq!(intersection.from, now + Duration::hours(1));
        assert_eq!(intersection.to, Some(now + Duration::hours(3)));
    }

    #[test]
    fn test_time_range_no_intersection() {
        let now = Utc::now();

        let range1 = TimeRange::new(now, now + Duration::hours(1)).unwrap();
        let range2 =
            TimeRange::new(now + Duration::hours(2), now + Duration::hours(3)).unwrap();

        assert!(range1.intersection(&range2).is_none());
    }

    #[test]
    fn test_time_range_duration() {
        let now = Utc::now();
        let range = TimeRange::new(now, now + Duration::hours(2)).unwrap();

        assert_eq!(range.duration(), Some(Duration::hours(2)));

        let open = TimeRange::starting_at(now);
        assert!(open.duration().is_none());
    }

    #[test]
    fn test_time_range_has_ended() {
        let past = Utc::now() - Duration::hours(2);
        let range = TimeRange::new(past, past + Duration::hours(1)).unwrap();
        assert!(range.has_ended());

        let open = TimeRange::from_now();
        assert!(!open.has_ended());
    }

    #[test]
    fn test_time_range_extend_by() {
        let now = Utc::now();
        let mut range = TimeRange::new(now, now + Duration::hours(1)).unwrap();

        range.extend_by(Duration::hours(1));
        assert_eq!(range.to, Some(now + Duration::hours(2)));
    }

    #[test]
    fn test_time_range_close_now() {
        let mut range = TimeRange::from_now();
        assert!(range.is_open_ended());

        range.close_now();
        assert!(!range.is_open_ended());
    }

    #[test]
    fn test_time_range_close_at() {
        let now = Utc::now();
        let mut range = TimeRange::starting_at(now);

        let close_time = now + Duration::hours(1);
        range.close_at(close_time).unwrap();
        assert_eq!(range.to, Some(close_time));
    }

    #[test]
    fn test_time_range_close_at_invalid() {
        let now = Utc::now();
        let mut range = TimeRange::starting_at(now);

        let before = now - Duration::hours(1);
        assert!(range.close_at(before).is_err());
    }

    #[test]
    fn test_time_range_display() {
        let range = TimeRange::from_now();
        let display = format!("{range}");
        assert!(display.contains("→"));
        assert!(display.contains("∞"));
    }

    #[test]
    fn test_time_range_serialization() {
        let range = TimeRange::from_now();
        let json = serde_json::to_string(&range).unwrap();
        let deserialized: TimeRange = serde_json::from_str(&json).unwrap();
        assert_eq!(range.from, deserialized.from);
        assert_eq!(range.to, deserialized.to);
    }
}
