# ChatGPT MCP schedule connection

The Worker exposes a private Streamable HTTP MCP endpoint at:

```text
https://blog-api.charlie-cloud.me/mcp
```

It exposes `list_tasks`, `create_task`, `update_task`, `delete_task`,
`list_reviews`, `save_review`, and `get_scheduling_experience`. `create_task`
uses the same class-aware DeepSeek planner as `/admin/schedule`; the class
reference remains code-only and the source timetable image is never uploaded.

## First deployment

Apply the migration and set the owner approval secret before deploying:

```sh
npx wrangler d1 execute blog --remote --file=./migrations/0007_mcp_oauth.sql
npx wrangler secret put MCP_OWNER_SECRET
npx wrangler deploy
```

The MCP migration is idempotent and creates only its two OAuth tables. Using
the direct file command also avoids replaying older migrations when a D1
database already has the legacy migration history.

Use a long random value for `MCP_OWNER_SECRET`. It is only entered on the
OAuth approval page when ChatGPT first connects. The Worker stores only a
hash of the resulting authorization codes and access/refresh tokens in D1.

The schedule planner calls Cloudflare's account-level AI Gateway REST API.
Set its Cloudflare API token separately:

```sh
npx wrangler secret put CLOUDFLARE_API_TOKEN
```

This token needs the account-level `Workers AI > Read` permission. The
DeepSeek provider key remains configured inside AI Gateway; it is not sent by
the Worker. `SCHEDULE_AI_MODEL` defaults to `deepseek/deepseek-chat` and can
be changed to another `deepseek/<model>` id exposed by the gateway.

In ChatGPT Developer Mode, add the `/mcp` URL. ChatGPT will discover the
OAuth metadata, open the approval page, and then use the returned token for
the MCP calls. Keep write-tool approval enabled so task creation, edits,
deletions, and reviews remain visible actions.

For direct local or script testing, an optional `MCP_BEARER_TOKEN` secret is
also supported. That token grants both schedule scopes and should be treated
like a password.
