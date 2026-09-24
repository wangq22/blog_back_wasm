-- 双 profile 模式:专业头像与轻松模式简介。
-- 执行(任选其一):
--   npx wrangler d1 execute blog --remote --file=./migrations/0008_profile_modes.sql
--   npx wrangler d1 execute blog --local  --file=./migrations/0008_profile_modes.sql
ALTER TABLE user ADD COLUMN serious_avatar_url TEXT;
ALTER TABLE user ADD COLUMN casual_bio TEXT;
