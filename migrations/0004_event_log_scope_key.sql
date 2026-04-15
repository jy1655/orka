ALTER TABLE event_log ADD COLUMN scope_key TEXT;

UPDATE event_log
SET scope_key = CASE
  WHEN user_id IS NOT NULL AND TRIM(user_id) <> '' THEN channel || ':' || TRIM(chat_id) || ':' || TRIM(user_id)
  ELSE channel || ':' || TRIM(chat_id)
END
WHERE scope_key IS NULL;

CREATE INDEX IF NOT EXISTS idx_event_log_scope_key_id
ON event_log(scope_key, id);
