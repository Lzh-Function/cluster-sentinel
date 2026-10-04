-- Find each probe/observer stream without loading historical payloads, then
-- seek its newest valid observation. Ascending finished_at also leaves the
-- implicit rowid ascending, so a reverse scan resolves equal times by rowid.
CREATE INDEX IF NOT EXISTS idx_observations_stream_time
    ON observations(target_entity_id, probe_id, COALESCE(observer_entity_id, ''), finished_at);
