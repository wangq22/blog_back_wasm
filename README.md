# Blog Worker

Rust 编写的 Cloudflare Worker，为博客提供文章、归档、搜索、用户资料、媒体、日程和 MCP 接口。

## 结构

```text
src/
├── lib.rs                 # Router、Worker fetch 与定时任务入口
├── middleware/            # Cloudflare Access 鉴权
├── model/                 # API / 数据库 DTO
└── route/                 # 按领域划分的路由处理器
migrations/                # D1 增量迁移
tests/                     # Rust 回归测试
wrangler.toml              # Worker、D1、R2、Cron 与公开变量配置
MCP.md                     # MCP / OAuth 接入说明
```

`blogs.sql` 是完整数据库结构参考；线上变更应通过 `migrations/` 中的增量 SQL 执行。

## 开发与验证

```bash
npm install
npm run dev

cargo fmt --check
npm test
npm run build
```

`npm run build` 需要已安装 `worker-build`：

```bash
cargo install worker-build
```

## 部署

```bash
npm run deploy
```

机密值必须通过 `npx wrangler secret put <NAME>` 配置，不要写入 `wrangler.toml`。当前需要的 secret 及用途记录在该文件的注释中。

GitHub Actions 会在 `main` 分支更新后构建并部署 Worker。
