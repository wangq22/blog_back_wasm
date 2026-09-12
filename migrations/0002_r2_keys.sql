-- R2 迁移:D1 不再存 markdown 全文/封面二进制,只存 R2 key。
-- 已有 content/cover_image 保留(老文章回退可读),新文章写 content_key/cover_key。
-- 执行(任选其一):
--   npx wrangler d1 execute blog --remote --file=./migrations/0002_r2_keys.sql
--   npx wrangler d1 execute blog --local  --file=./migrations/0002_r2_keys.sql
ALTER TABLE posts ADD COLUMN content_key TEXT;
ALTER TABLE posts ADD COLUMN cover_key TEXT;
