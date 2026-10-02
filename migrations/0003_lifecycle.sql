-- Source-specific hysteresis and replay watermarks survive controller restarts.
CREATE TABLE state_streams (
    environment TEXT PRIMARY KEY REFERENCES environments(name) ON DELETE CASCADE,
    payload TEXT NOT NULL
) STRICT;

-- Current notification candidates are committed together with their incident.
CREATE TABLE notification_candidates (
    deduplication_key TEXT PRIMARY KEY,
    incident_id TEXT NOT NULL REFERENCES incidents(id) ON DELETE CASCADE,
    payload TEXT NOT NULL
) STRICT;
