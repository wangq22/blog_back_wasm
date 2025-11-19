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
CREATE TABLE posts (
    _id INTEGER PRIMARY KEY,
    title TEXT,
    excerpt TEXT,
    date TEXT,
    category TEXT,
    cover_image TEXT,
    content TEXT,
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
        'https://github.com/WangQiHao-Charlie',
        'https://space.bilibili.com/660065958'
    );
INSERT INTO posts (
        title,
        excerpt,
        date,
        category,
        content,
        cover_image,
        word_count,
        read_time
    )
VALUES (
        'Build a modern Blog with React and Cloudflare Worker-rs',
        'This blog described the architecture of my blog',
        '2025-10-09T13:45:00Z',
        'front end',
        '# Build a blog 
        This is an example of blog content',
        'https://fuwari.vercel.app/_astro/cover.CgGywNHJ_9MQNr.webp',
        1024,
        5
    );
INSERT INTO tags (name, color_class)
VALUES ('Rust', 'info_badge');
INSERT INTO tags (name, color_class)
VALUES ('Cloudflare workers', 'info_badge');
INSERT INTO tags (name, color_class)
VALUES ('React', 'info_badge');
INSERT INTO tags (name, color_class)
VALUES ('Vite', 'info_badge');
INSERT INTO post_tags (post_id, tag_id)
VALUES (1, 1);
INSERT INTO post_tags (post_id, tag_id)
VALUES (1, 2);
INSERT INTO post_tags (post_id, tag_id)
VALUES (1, 3);
INSERT INTO post_tags (post_id, tag_id)
VALUES (1, 4);
INSERT INTO category (_id, name, post_count)
VALUES (1, 'Front End', 1)