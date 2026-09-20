DROP TABLE IF EXISTS post_tags;
DROP TABLE IF EXISTS user;
DROP TABLE IF EXISTS posts;
DROP TABLE IF EXISTS tags;
DROP TABLE IF EXISTS category;
CREATE TABLE user (
    _id INTEGER PRIMARY KEY,
    name TEXT,
    bio TEXT,
    avatar_url TEXT,
    github_url TEXT,
    bilibili_url TEXT,
    timezone TEXT,
    city TEXT,
    email TEXT,
    affiliation TEXT
);
CREATE TABLE category(
    _id INTEGER PRIMARY KEY,
    name TEXT UNIQUE,
    post_count INTEGER
);
-- D1 只存 R2 key,正文 markdown 与封面图全在 R2(不兼容老 content/cover_image 列)
CREATE TABLE posts (
    _id INTEGER PRIMARY KEY,
    title TEXT,
    excerpt TEXT,
    date TEXT,
    category TEXT,
    content_key TEXT,
    cover_key TEXT,
    word_count INTEGER,
    read_time INTEGER
);
CREATE TABLE tags (
    _id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT UNIQUE,
    color_class TEXT
);
CREATE TABLE post_tags (
    post_id TEXT,
    tag_id INTEGER,
    FOREIGN KEY(post_id) REFERENCES posts(_id),
    FOREIGN KEY(tag_id) REFERENCES tags(_id)
);
INSERT into user (
        name,
        bio,
        avatar_url,
        github_url,
        bilibili_url
    )
VALUES (
        'Charlie Wang',
        'Full-stack developer, RISC-V processor enthusiast, self-hosting enthusiast, DevOps enthusiast',
        'https://avatars.githubusercontent.com/u/180044640?v=4',
        'https://github.com/wangq22',
        'https://space.bilibili.com/660065958'
    );
INSERT INTO tags (name, color_class)
VALUES ('Rust', 'info_badge');
INSERT INTO tags (name, color_class)
VALUES ('Cloudflare workers', 'info_badge');
INSERT INTO tags (name, color_class)
VALUES ('React', 'info_badge');
INSERT INTO tags (name, color_class)
VALUES ('Vite', 'info_badge');
INSERT INTO category (_id, name, post_count)
VALUES (1, 'Front End', 0);

CREATE TABLE schedule_tasks (
    _id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    notes TEXT NOT NULL DEFAULT '',
    deadline TEXT NOT NULL DEFAULT '',
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
CREATE TABLE mcp_oauth_codes (
    code_hash TEXT PRIMARY KEY,
    client_id TEXT NOT NULL,
    redirect_uri TEXT NOT NULL,
    code_challenge TEXT NOT NULL,
    resource TEXT NOT NULL,
    scope TEXT NOT NULL,
    subject TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);
CREATE INDEX mcp_oauth_codes_expiry ON mcp_oauth_codes(expires_at);
CREATE TABLE mcp_oauth_tokens (
    token_hash TEXT PRIMARY KEY,
    token_type TEXT NOT NULL,
    resource TEXT NOT NULL,
    scope TEXT NOT NULL,
    subject TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    revoked_at TEXT NOT NULL DEFAULT ''
);
CREATE INDEX mcp_oauth_tokens_lookup
    ON mcp_oauth_tokens(token_type, expires_at, revoked_at);
