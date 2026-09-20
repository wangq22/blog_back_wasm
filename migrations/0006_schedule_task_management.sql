-- Persist an optional deadline so owner edits can keep and change it.
ALTER TABLE schedule_tasks ADD COLUMN deadline TEXT NOT NULL DEFAULT '';
