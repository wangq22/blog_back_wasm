-- 用户资料扩展:时区 / 城市 / 邮箱 / 所属机构或公司。
-- GET /api/user 走 SELECT *,加列后自动返回,无需改查询。
-- 执行(任选其一):
--   npx wrangler d1 execute blog --remote --file=./migrations/0004_user_profile_extend.sql
--   npx wrangler d1 execute blog --local  --file=./migrations/0004_user_profile_extend.sql
ALTER TABLE user ADD COLUMN timezone TEXT;
ALTER TABLE user ADD COLUMN city TEXT;
ALTER TABLE user ADD COLUMN email TEXT;
ALTER TABLE user ADD COLUMN affiliation TEXT;
