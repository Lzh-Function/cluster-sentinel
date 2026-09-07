//! Time model (IMPLEMENTATION.md §46).
//!
//! * Persisted timestamps are **UTC wall clock** ([`Timestamp`]).
//! * Probe latency and timeouts use a **monotonic** clock ([`std::time::Instant`]).
//! * Incident timelines use wall clock.

use chrono::{DateTime, SubsecRound, Utc};

/// A UTC wall-clock instant. This is the only timestamp type persisted.
pub type Timestamp = DateTime<Utc>;

/// Current UTC wall clock, truncated to microseconds.
///
/// The truncation matters: [`to_rfc3339`] persists microseconds, so a timestamp
/// that kept nanoseconds in memory would not equal itself after a round trip
/// through the database. Sub-microsecond resolution is of no use to a monitoring
/// system, and latency is measured on a monotonic clock regardless.
pub fn now() -> Timestamp {
    Utc::now().trunc_subsecs(6)
}

/// Serialize a timestamp to the canonical persisted form (RFC3339, UTC).
pub fn to_rfc3339(ts: Timestamp) -> String {
    ts.to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}

/// Parse a canonical persisted timestamp.
pub fn parse_rfc3339(s: &str) -> Result<Timestamp, chrono::ParseError> {
    Ok(DateTime::parse_from_rfc3339(s)?.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timestamp_survives_a_database_round_trip_exactly() {
        // Equality after persistence is relied on by the observation tests and
        // by idempotent ingestion.
        let now = now();
        assert_eq!(parse_rfc3339(&to_rfc3339(now)).expect("parse"), now);
    }

    #[test]
    fn rfc3339_roundtrip_is_stable() {
        let now = now();
        let text = to_rfc3339(now);
        let parsed = parse_rfc3339(&text).expect("parse");
        assert_eq!(to_rfc3339(parsed), text);
        assert!(text.ends_with('Z'), "timestamps must be persisted as UTC");
    }
}
