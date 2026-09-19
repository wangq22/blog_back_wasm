-- Live schedule / todo data. The original class timetable image is not stored.
CREATE TABLE schedule_tasks (
    _id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    notes TEXT NOT NULL DEFAULT '',
    scheduled_start TEXT NOT NULL,
    scheduled_end TEXT NOT NULL,
    estimated_minutes INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'planned',
    ai_reason TEXT NOT NULL DEFAULT '',
    ai_model TEXT NOT NULL DEFAULT '',
    actual_minutes INTEGER,
    completion_summary TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX schedule_tasks_status_start ON schedule_tasks(status, scheduled_start);

CREATE TABLE schedule_task_reviews (
    _id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    outcome TEXT,
    prompted_at TEXT NOT NULL,
    reviewed_at TEXT,
    spent_minutes INTEGER,
    summary TEXT NOT NULL DEFAULT '',
    FOREIGN KEY(task_id) REFERENCES schedule_tasks(_id)
);

CREATE INDEX schedule_task_reviews_status ON schedule_task_reviews(status, prompted_at);
CREATE UNIQUE INDEX schedule_pending_review_task ON schedule_task_reviews(task_id) WHERE status = 'pending';

CREATE TABLE schedule_reactions (
    task_id INTEGER NOT NULL,
    emoji TEXT NOT NULL,
    count INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(task_id, emoji),
    FOREIGN KEY(task_id) REFERENCES schedule_tasks(_id)
);

CREATE TABLE schedule_learning (
    _id INTEGER PRIMARY KEY CHECK (_id = 1),
    content TEXT NOT NULL,
    ai_model TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL
);
