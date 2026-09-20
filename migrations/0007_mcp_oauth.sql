-- OAuth 2.1 authorization-code state and opaque MCP tokens.
-- These tables are intentionally separate from schedule_tasks so the MCP
-- connection can be revoked/expired without touching the todo history.
CREATE TABLE IF NOT EXISTS mcp_oauth_codes (
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

CREATE INDEX IF NOT EXISTS mcp_oauth_codes_expiry ON mcp_oauth_codes(expires_at);

CREATE TABLE IF NOT EXISTS mcp_oauth_tokens (
    token_hash TEXT PRIMARY KEY,
    token_type TEXT NOT NULL,
    resource TEXT NOT NULL,
    scope TEXT NOT NULL,
    subject TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    revoked_at TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS mcp_oauth_tokens_lookup
    ON mcp_oauth_tokens(token_type, expires_at, revoked_at);
