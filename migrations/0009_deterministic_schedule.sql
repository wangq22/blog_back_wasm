-- Keep the old task columns as compatibility summaries; blocks are authoritative.
ALTER TABLE schedule_tasks ADD COLUMN planning_json TEXT NOT NULL DEFAULT '{}';
CREATE TABLE schedule_blocks (
    id TEXT PRIMARY KEY,
    task_id INTEGER NOT NULL REFERENCES schedule_tasks(_id),
    scheduled_start TEXT NOT NULL,
    scheduled_end TEXT NOT NULL,
    status TEXT NOT NULL,
    data_json TEXT NOT NULL
);
CREATE INDEX schedule_blocks_task ON schedule_blocks(task_id, scheduled_start);
CREATE INDEX schedule_blocks_due ON schedule_blocks(status, scheduled_end);
ALTER TABLE schedule_task_reviews ADD COLUMN block_id TEXT;
ALTER TABLE schedule_task_reviews ADD COLUMN data_json TEXT NOT NULL DEFAULT '{}';
DROP INDEX schedule_pending_review_task;
CREATE UNIQUE INDEX schedule_review_block ON schedule_task_reviews(block_id) WHERE block_id IS NOT NULL;
CREATE TABLE schedule_state (
    _id INTEGER PRIMARY KEY CHECK (_id = 1),
    revision INTEGER NOT NULL DEFAULT 0,
    commit_token TEXT NOT NULL DEFAULT '',
    data_json TEXT NOT NULL DEFAULT '{}'
);
INSERT INTO schedule_state (_id) VALUES (1);
UPDATE schedule_state SET data_json = json_object('learning', json_object(
    'content', COALESCE((SELECT content FROM schedule_learning WHERE _id=1), ''),
    'ai_model', COALESCE((SELECT ai_model FROM schedule_learning WHERE _id=1), ''),
    'updated_at', COALESCE((SELECT updated_at FROM schedule_learning WHERE _id=1), '')
)) WHERE _id=1;
-- Existing intervals remain intact, with conservative assessment metadata.
INSERT INTO schedule_blocks
SELECT 'legacy:' || _id, _id, scheduled_start, scheduled_end,
       CASE status WHEN 'done' THEN 'completed' ELSE status END,
       json_object('id','legacy:' || _id,'task_id',_id,
         'start',unixepoch(scheduled_start),'end',unixepoch(scheduled_end),
         'minutes',estimated_minutes,'status',CASE status WHEN 'done' THEN 'completed' ELSE status END,
         'locked',json('false'),'assessment',json_object('estimated_minutes',estimated_minutes,
           'source','legacy','reason','Migrated single block; historical estimates are not training evidence.'),
         'settings_version',0,'legacy',json('true'),
         'actual_minutes',CASE WHEN status IN ('done','skipped') THEN actual_minutes ELSE NULL END)
FROM schedule_tasks WHERE scheduled_start != '' AND scheduled_end != '';
UPDATE schedule_task_reviews SET block_id = 'legacy:' || task_id
WHERE status = 'pending';
-- Complete before/after profiles make automated and manual calibration auditable.
CREATE TABLE schedule_profile_history (
    revision INTEGER PRIMARY KEY,
    created_at TEXT NOT NULL,
    before_json TEXT NOT NULL,
    after_json TEXT NOT NULL
);
