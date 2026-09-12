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
    bilibili_url TEXT
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
VALUES (1, 'Front End', 0)