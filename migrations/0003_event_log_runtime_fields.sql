ALTER TABLE event_log ADD COLUMN provider_kind TEXT;
ALTER TABLE event_log ADD COLUMN runtime_mode TEXT;
ALTER TABLE event_log ADD COLUMN provider_latency_ms INTEGER;
ALTER TABLE event_log ADD COLUMN provider_status TEXT;
