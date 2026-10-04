-- The reset boundary prevents observations from before an update from
-- rebuilding current state. Observation and incident history are retained.
CREATE TABLE state_resets (
    environment TEXT PRIMARY KEY REFERENCES environments(name) ON DELETE CASCADE,
    reset_at TEXT NOT NULL
) STRICT;
