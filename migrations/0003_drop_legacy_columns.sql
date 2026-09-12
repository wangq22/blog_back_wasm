-- 去 legacy:删掉 D1 里存全文/外链的 content/cover_image 列,只留 R2 key。
-- 老测试文章不要了,先清数据再删列(按顺序 0002 已加 content_key/cover_key)。
-- 执行:
--   npx wrangler d1 execute blog --remote --file=./migrations/0003_drop_legacy_columns.sql
--   npx wrangler d1 execute blog --local  --file=./migrations/0003_drop_legacy_columns.sql
DELETE FROM post_tags;
DELETE FROM posts;
UPDATE category SET post_count = 0;
ALTER TABLE posts DROP COLUMN content;
ALTER TABLE posts DROP COLUMN cover_image;
