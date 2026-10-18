# ChatGPT MCP schedule connection

The Worker exposes a private Streamable HTTP MCP endpoint at:

```text
https://blog-api.charlie-cloud.me/mcp
```

It exposes `list_tasks`, `create_task`, `update_task`, `delete_task`,
`list_reviews`, `save_review`, `get_scheduling_experience`, `get_schedule_settings`,
`update_schedule_settings`, `replan_schedule`, and `lock_schedule_block`. AI
assesses task parameters; the same deterministic Rust scheduler used by
`/admin/schedule` chooses work blocks. Fixed contests keep their exact times.
See [SCHEDULER.md](SCHEDULER.md) for migration 0009, review semantics and defaults.
The source timetable image is never uploaded.

## Explicit priorities and dependencies

Use `priority_level` for P1/P2: **P1 is highest**, P3 is the default, P5 is lowest. Do not encode priority only in `notes`. The old numeric `priority` remains compatible (5 highest); responses return both `priority` and `priority_level`.

Create the prerequisite first, then use its returned ID:

```json
{"name":"create_task","arguments":{"title":"Design A","priority_level":"P2"}}
{"name":"create_task","arguments":{"title":"Implement A","priority_level":"P1","depends_on":[123]}}
```

Replace `123` with the design task's actual ID. Use `list_tasks` for existing IDs. The implementation starts after **all** design work blocks finish, even with higher priority. Multiple IDs require every predecessor to finish. Missing, cyclic or self dependencies are rejected; unavailable or unreviewed predecessors leave successors waiting. Fixed and protected conflicting times are retained and flagged.

`update_task` accepts the same fields. Omitted `depends_on` preserves prerequisites; `[]` clears them. Remove dependent references before deleting a predecessor. No new migration is required for these fields. After deploying the backend, refresh the MCP client's tool definitions so it discovers the new arguments. Existing notes containing P1/P2 are not retroactively converted; update those tasks explicitly.

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
