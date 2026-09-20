//! Streamable HTTP MCP endpoint for the private schedule tools.
//!
//! ChatGPT connects to `/mcp`.  The endpoint implements the small part of the
//! MCP protocol needed by this app and delegates task mutations to the same
//! schedule handlers used by the owner dashboard.  OAuth state and opaque
//! tokens live in D1; no task data is copied into the MCP layer.

use axum::{
    body::{to_bytes, Body},
    extract::{Form, Query},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    Extension, Json,
};
use getrandom::getrandom;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime};
use worker::{send::SendWrapper, Date as WorkerDate, Env};

use super::schedule::{self, ReviewDTO, TaskCreateDTO, TaskUpdateDTO};

const MCP_PROTOCOL_VERSION: &str = "2025-03-26";
const MCP_SERVER_NAME: &str = "charlie-cloud-schedule";
const MCP_SERVER_VERSION: &str = "1.0.0";
const DEFAULT_ISSUER: &str = "https://blog-api.charlie-cloud.me";
const DEFAULT_RESOURCE: &str = "https://blog-api.charlie-cloud.me/mcp";
const READ_SCOPE: &str = "schedule:read";
const WRITE_SCOPE: &str = "schedule:write";
const ALL_SCOPES: &str = "schedule:read schedule:write";

#[derive(Debug, Clone)]
struct McpPrincipal {
    scope: String,
}

impl McpPrincipal {
    fn has_scope(&self, expected: &str) -> bool {
        self.scope.split_whitespace().any(|scope| scope == expected)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct OAuthAuthorizeParams {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    state: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    resource: Option<String>,
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OAuthAuthorizeForm {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    state: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    resource: Option<String>,
    scope: Option<String>,
    owner_secret: Option<String>,
}

impl From<OAuthAuthorizeForm> for OAuthAuthorizeParams {
    fn from(form: OAuthAuthorizeForm) -> Self {
        Self {
            response_type: form.response_type,
            client_id: form.client_id,
            redirect_uri: form.redirect_uri,
            state: form.state,
            code_challenge: form.code_challenge,
            code_challenge_method: form.code_challenge_method,
            resource: form.resource,
            scope: form.scope,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct OAuthTokenForm {
    grant_type: Option<String>,
    code: Option<String>,
    refresh_token: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    code_verifier: Option<String>,
    resource: Option<String>,
}

fn env_string(env: &Env, key: &str, fallback: &str) -> String {
    env.var(key)
        .ok()
        .map(|value| value.to_string().trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn issuer(env: &Env) -> String {
    env_string(env, "MCP_ISSUER", DEFAULT_ISSUER)
        .trim_end_matches('/')
        .to_string()
}

fn resource_url(env: &Env) -> String {
    env_string(env, "MCP_RESOURCE_URL", DEFAULT_RESOURCE)
}

fn protected_resource_metadata_url(env: &Env) -> String {
    format!("{}/.well-known/oauth-protected-resource", issuer(env))
}

fn now_iso() -> String {
    OffsetDateTime::from_unix_timestamp_nanos(WorkerDate::now().as_millis() as i128 * 1_000_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn future_iso(minutes: i64) -> String {
    OffsetDateTime::from_unix_timestamp_nanos(WorkerDate::now().as_millis() as i128 * 1_000_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .checked_add(Duration::minutes(minutes))
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn random_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom(&mut bytes)
        .map_err(|error| format!("secure random source unavailable: {}", error))?;
    Ok(base64url(&bytes))
}

fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = String::with_capacity((bytes.len() * 4).div_ceil(3));
    let mut index = 0;
    while index < bytes.len() {
        let a = bytes[index] as u32;
        let b = bytes.get(index + 1).copied().unwrap_or(0) as u32;
        let c = bytes.get(index + 2).copied().unwrap_or(0) as u32;
        let value = (a << 16) | (b << 8) | c;
        output.push(ALPHABET[((value >> 18) & 63) as usize] as char);
        output.push(ALPHABET[((value >> 12) & 63) as usize] as char);
        if index + 1 < bytes.len() {
            output.push(ALPHABET[((value >> 6) & 63) as usize] as char);
        }
        if index + 2 < bytes.len() {
            output.push(ALPHABET[(value & 63) as usize] as char);
        }
        index += 3;
    }
    output
}

fn sha256_base64url(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    base64url(&digest)
}

fn sha256_hex(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest.iter().map(|byte| format!("{:02x}", byte)).collect()
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn percent_encode(value: &str) -> String {
    let mut output = String::new();
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            output.push(*byte as char);
        } else {
            output.push('%');
            output.push_str(&format!("{:02X}", byte));
        }
    }
    output
}

fn redirect_response(location: &str) -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::FOUND;
    if let Ok(value) = HeaderValue::from_str(location) {
        response.headers_mut().insert(header::LOCATION, value);
    }
    response
}

fn oauth_error(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "error": "invalid_request",
            "error_description": message,
        })),
    )
        .into_response()
}

fn oauth_token_error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(json!({ "error": code, "error_description": message })),
    )
        .into_response()
}

fn valid_chatgpt_client(client_id: &str) -> bool {
    client_id.starts_with("https://chatgpt.com/")
        || client_id.starts_with("https://chat.openai.com/")
}

fn valid_chatgpt_redirect(redirect_uri: &str) -> bool {
    redirect_uri.starts_with("https://chatgpt.com/")
        || redirect_uri.starts_with("https://chat.openai.com/")
}

fn normalize_scope(requested: Option<&str>) -> Result<String, String> {
    let requested = requested.unwrap_or(ALL_SCOPES).trim();
    if requested.is_empty() {
        return Ok(ALL_SCOPES.to_string());
    }
    let mut scopes = Vec::new();
    for scope in requested.split_whitespace() {
        if scope != READ_SCOPE && scope != WRITE_SCOPE {
            return Err(format!("unsupported scope: {}", scope));
        }
        if !scopes.iter().any(|existing| existing == &scope) {
            scopes.push(scope);
        }
    }
    Ok(scopes.join(" "))
}

fn validated_authorize_params(
    env: &Env,
    params: &OAuthAuthorizeParams,
) -> Result<(String, String, String, String, String, String), String> {
    if params.response_type.as_deref() != Some("code") {
        return Err("response_type must be code".to_string());
    }
    let client_id = params
        .client_id
        .as_deref()
        .filter(|value| valid_chatgpt_client(value))
        .ok_or_else(|| "client_id must be a ChatGPT client metadata URL".to_string())?;
    let redirect_uri = params
        .redirect_uri
        .as_deref()
        .filter(|value| valid_chatgpt_redirect(value))
        .ok_or_else(|| "redirect_uri must point to ChatGPT".to_string())?;
    let code_challenge = params
        .code_challenge
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "code_challenge is required".to_string())?;
    if params.code_challenge_method.as_deref() != Some("S256") {
        return Err("code_challenge_method must be S256".to_string());
    }
    let resource = params.resource.clone().unwrap_or_else(|| resource_url(env));
    if resource != resource_url(env) {
        return Err("resource does not match this MCP server".to_string());
    }
    let scope = normalize_scope(params.scope.as_deref())?;
    Ok((
        client_id.to_string(),
        redirect_uri.to_string(),
        code_challenge.to_string(),
        resource,
        scope,
        params.state.clone().unwrap_or_default(),
    ))
}

fn authorize_form(env: &Env, params: &OAuthAuthorizeParams, error: Option<&str>) -> Response {
    let error_block = error
        .map(|message| format!("<p class=\"error\">{}</p>", html_escape(message)))
        .unwrap_or_default();
    let hidden = |name: &str, value: Option<&str>| {
        format!(
            "<input type=\"hidden\" name=\"{}\" value=\"{}\">",
            name,
            html_escape(value.unwrap_or_default())
        )
    };
    let body = format!(
        "<!doctype html><html lang=\"en\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Authorize schedule tools</title><style>body{{font:16px system-ui;max-width:34rem;margin:10vh auto;padding:0 1.25rem;color:#182230}}form{{display:grid;gap:1rem;padding:1.25rem;border:1px solid #d8dee8;border-radius:1rem;box-shadow:0 8px 30px #18223014}}input[type=password]{{padding:.75rem;border:1px solid #aab5c5;border-radius:.5rem;font:inherit}}button{{padding:.75rem;border:0;border-radius:.5rem;background:#182230;color:white;font:inherit;cursor:pointer}}.error{{color:#b42318}}</style><h1>Authorize schedule tools</h1><p>ChatGPT is asking to read and update the private schedule todo list.</p>{}<form method=\"post\" action=\"{}/oauth/authorize\">{}{}{}{}{}{}{}{}<label for=\"owner-secret\">Owner secret</label><input id=\"owner-secret\" name=\"owner_secret\" type=\"password\" autocomplete=\"current-password\" required><button type=\"submit\">Authorize ChatGPT</button></form></html>",
        error_block,
        issuer(env),
        hidden("response_type", params.response_type.as_deref()),
        hidden("client_id", params.client_id.as_deref()),
        hidden("redirect_uri", params.redirect_uri.as_deref()),
        hidden("state", params.state.as_deref()),
        hidden("code_challenge", params.code_challenge.as_deref()),
        hidden("code_challenge_method", params.code_challenge_method.as_deref()),
        hidden("resource", params.resource.as_deref()),
        hidden("scope", params.scope.as_deref()),
    );
    Html(body).into_response()
}

#[worker::send]
pub async fn oauth_authorize_get(
    Extension(env): Extension<SendWrapper<Env>>,
    Query(params): Query<OAuthAuthorizeParams>,
) -> Response {
    match validated_authorize_params(&env, &params) {
        Ok(_) => authorize_form(&env, &params, None),
        Err(error) => oauth_error(&error),
    }
}

#[worker::send]
pub async fn oauth_authorize_post(
    Extension(env): Extension<SendWrapper<Env>>,
    Form(form): Form<OAuthAuthorizeForm>,
) -> Response {
    let supplied_owner_secret = form.owner_secret.clone().unwrap_or_default();
    let params: OAuthAuthorizeParams = form.into();
    let validated = match validated_authorize_params(&env, &params) {
        Ok(value) => value,
        Err(error) => return oauth_error(&error),
    };
    let owner_secret = match env.secret("MCP_OWNER_SECRET") {
        Ok(secret) if !secret.to_string().is_empty() => secret.to_string(),
        _ => return oauth_error("MCP_OWNER_SECRET is not configured on the Worker"),
    };
    if supplied_owner_secret != owner_secret {
        return authorize_form(&env, &params, Some("The owner secret was not accepted."));
    }

    let code = match random_token() {
        Ok(value) => value,
        Err(error) => return oauth_error(&error),
    };
    let now = now_iso();
    let (client_id, redirect_uri, code_challenge, resource, scope, state) = validated;
    let db = match env.d1("DB") {
        Ok(db) => db,
        Err(error) => return oauth_error(&format!("D1 is unavailable: {}", error)),
    };
    let subject = env_string(&env, "MCP_OWNER_EMAIL", "owner");
    let statement = match db
        .prepare(
            "INSERT INTO mcp_oauth_codes
             (code_hash, client_id, redirect_uri, code_challenge, resource, scope, subject, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .bind(&[
            sha256_hex(&code).into(),
            client_id.into(),
            redirect_uri.clone().into(),
            code_challenge.into(),
            resource.into(),
            scope.into(),
            subject.into(),
            now.into(),
            future_iso(10).into(),
        ])
    {
        Ok(statement) => statement,
        Err(error) => return oauth_error(&format!("Could not store authorization code: {}", error)),
    };
    if let Err(error) = statement.run().await {
        return oauth_error(&format!("Could not store authorization code: {}", error));
    }

    let separator = if redirect_uri.contains('?') { '&' } else { '?' };
    let mut location = format!(
        "{}{}code={}",
        redirect_uri,
        separator,
        percent_encode(&code)
    );
    if !state.is_empty() {
        location.push_str("&state=");
        location.push_str(&percent_encode(&state));
    }
    redirect_response(&location)
}

#[worker::send]
pub async fn oauth_token(
    Extension(env): Extension<SendWrapper<Env>>,
    Form(form): Form<OAuthTokenForm>,
) -> Response {
    let grant_type = form.grant_type.as_deref().unwrap_or_default();
    match grant_type {
        "authorization_code" => exchange_authorization_code(&env, form).await,
        "refresh_token" => exchange_refresh_token(&env, form).await,
        _ => oauth_token_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "grant_type must be authorization_code or refresh_token",
        ),
    }
}

async fn exchange_authorization_code(env: &Env, form: OAuthTokenForm) -> Response {
    let code = match form.code.as_deref().filter(|value| !value.is_empty()) {
        Some(value) => value,
        None => {
            return oauth_token_error(StatusCode::BAD_REQUEST, "invalid_grant", "code is required")
        }
    };
    let verifier = match form
        .code_verifier
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        Some(value) => value,
        None => {
            return oauth_token_error(
                StatusCode::BAD_REQUEST,
                "invalid_grant",
                "code_verifier is required",
            )
        }
    };
    let client_id = form.client_id.as_deref().unwrap_or_default();
    let redirect_uri = form.redirect_uri.as_deref().unwrap_or_default();
    let expected_resource = resource_url(env);
    let resource = form
        .resource
        .as_deref()
        .unwrap_or(expected_resource.as_str());
    if !valid_chatgpt_client(client_id) || !valid_chatgpt_redirect(redirect_uri) {
        return oauth_token_error(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "invalid client or redirect_uri",
        );
    }
    if resource != resource_url(env) {
        return oauth_token_error(
            StatusCode::BAD_REQUEST,
            "invalid_target",
            "resource does not match this MCP server",
        );
    }

    let db = match env.d1("DB") {
        Ok(db) => db,
        Err(error) => {
            return oauth_token_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                &error.to_string(),
            )
        }
    };
    let row = match db
        .prepare(
            "SELECT code_hash, client_id, redirect_uri, code_challenge, resource, scope, subject
             FROM mcp_oauth_codes WHERE code_hash = ?1 AND expires_at > ?2",
        )
        .bind(&[sha256_hex(code).into(), now_iso().into()])
    {
        Ok(statement) => match statement.first::<Value>(None).await {
            Ok(Some(row)) => row,
            _ => {
                return oauth_token_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "authorization code is invalid or expired",
                )
            }
        },
        Err(error) => {
            return oauth_token_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                &error.to_string(),
            )
        }
    };
    let verifier_challenge = sha256_base64url(verifier);
    if row.get("client_id").and_then(Value::as_str) != Some(client_id)
        || row.get("redirect_uri").and_then(Value::as_str) != Some(redirect_uri)
        || row.get("resource").and_then(Value::as_str) != Some(resource)
        || row.get("code_challenge").and_then(Value::as_str) != Some(verifier_challenge.as_str())
    {
        return oauth_token_error(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "authorization code verification failed",
        );
    }

    let delete = match db
        .prepare("DELETE FROM mcp_oauth_codes WHERE code_hash = ?1")
        .bind(&[sha256_hex(code).into()])
    {
        Ok(statement) => statement,
        Err(error) => {
            return oauth_token_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                &error.to_string(),
            )
        }
    };
    if let Err(error) = delete.run().await {
        return oauth_token_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            &error.to_string(),
        );
    }
    issue_tokens(
        &db,
        row.get("resource")
            .and_then(Value::as_str)
            .unwrap_or(resource),
        row.get("scope")
            .and_then(Value::as_str)
            .unwrap_or(ALL_SCOPES),
        row.get("subject")
            .and_then(Value::as_str)
            .unwrap_or("owner"),
    )
    .await
}

async fn exchange_refresh_token(env: &Env, form: OAuthTokenForm) -> Response {
    let refresh_token = match form
        .refresh_token
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        Some(value) => value,
        None => {
            return oauth_token_error(
                StatusCode::BAD_REQUEST,
                "invalid_grant",
                "refresh_token is required",
            )
        }
    };
    if let Some(client_id) = form.client_id.as_deref() {
        if !client_id.is_empty() && !valid_chatgpt_client(client_id) {
            return oauth_token_error(
                StatusCode::BAD_REQUEST,
                "invalid_client",
                "invalid client_id",
            );
        }
    }
    let db = match env.d1("DB") {
        Ok(db) => db,
        Err(error) => {
            return oauth_token_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                &error.to_string(),
            )
        }
    };
    let row = match db
        .prepare(
            "SELECT resource, scope, subject FROM mcp_oauth_tokens
             WHERE token_hash = ?1 AND token_type = 'refresh' AND revoked_at = '' AND expires_at > ?2",
        )
        .bind(&[sha256_hex(refresh_token).into(), now_iso().into()])
    {
        Ok(statement) => match statement.first::<Value>(None).await {
            Ok(Some(row)) => row,
            _ => return oauth_token_error(StatusCode::BAD_REQUEST, "invalid_grant", "refresh_token is invalid or expired"),
        },
        Err(error) => return oauth_token_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error", &error.to_string()),
    };
    let expected_resource = resource_url(env);
    if form
        .resource
        .as_deref()
        .unwrap_or(expected_resource.as_str())
        != row
            .get("resource")
            .and_then(Value::as_str)
            .unwrap_or_default()
    {
        return oauth_token_error(
            StatusCode::BAD_REQUEST,
            "invalid_target",
            "resource does not match this MCP server",
        );
    }
    issue_access_token(
        &db,
        row.get("resource")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        row.get("scope")
            .and_then(Value::as_str)
            .unwrap_or(ALL_SCOPES),
        row.get("subject")
            .and_then(Value::as_str)
            .unwrap_or("owner"),
        Some(refresh_token),
    )
    .await
}

async fn issue_tokens(
    db: &worker::D1Database,
    resource: &str,
    scope: &str,
    subject: &str,
) -> Response {
    let refresh_token = match random_token() {
        Ok(value) => value,
        Err(error) => {
            return oauth_token_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error", &error)
        }
    };
    let refresh_insert = match db
        .prepare(
            "INSERT INTO mcp_oauth_tokens
             (token_hash, token_type, resource, scope, subject, created_at, expires_at, revoked_at)
             VALUES (?1, 'refresh', ?2, ?3, ?4, ?5, ?6, '')",
        )
        .bind(&[
            sha256_hex(&refresh_token).into(),
            resource.to_string().into(),
            scope.to_string().into(),
            subject.to_string().into(),
            now_iso().into(),
            future_iso(90 * 24 * 60).into(),
        ]) {
        Ok(statement) => statement,
        Err(error) => {
            return oauth_token_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                &error.to_string(),
            )
        }
    };
    if let Err(error) = refresh_insert.run().await {
        return oauth_token_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            &error.to_string(),
        );
    }
    issue_access_token(&db, resource, scope, subject, Some(&refresh_token)).await
}

async fn issue_access_token(
    db: &worker::D1Database,
    resource: &str,
    scope: &str,
    subject: &str,
    refresh_token: Option<&str>,
) -> Response {
    let access_token = match random_token() {
        Ok(value) => value,
        Err(error) => {
            return oauth_token_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error", &error)
        }
    };
    let insert = match db
        .prepare(
            "INSERT INTO mcp_oauth_tokens
             (token_hash, token_type, resource, scope, subject, created_at, expires_at, revoked_at)
             VALUES (?1, 'access', ?2, ?3, ?4, ?5, ?6, '')",
        )
        .bind(&[
            sha256_hex(&access_token).into(),
            resource.to_string().into(),
            scope.to_string().into(),
            subject.to_string().into(),
            now_iso().into(),
            future_iso(60).into(),
        ]) {
        Ok(statement) => statement,
        Err(error) => {
            return oauth_token_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                &error.to_string(),
            )
        }
    };
    if let Err(error) = insert.run().await {
        return oauth_token_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            &error.to_string(),
        );
    }
    let mut response = json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": 3600,
        "scope": scope,
        "resource": resource,
    });
    if let Some(refresh_token) = refresh_token {
        response["refresh_token"] = Value::String(refresh_token.to_string());
    }
    (StatusCode::OK, Json(response)).into_response()
}

async fn authenticate(env: &Env, headers: &HeaderMap) -> Result<McpPrincipal, Response> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| unauthorized(env))?;

    if let Ok(secret) = env.secret("MCP_BEARER_TOKEN") {
        if !secret.to_string().is_empty() && secret.to_string() == token {
            return Ok(McpPrincipal {
                scope: ALL_SCOPES.to_string(),
            });
        }
    }

    let db = env.d1("DB").map_err(|_| unauthorized(env))?;
    let statement = db
        .prepare(
            "SELECT resource, scope, subject FROM mcp_oauth_tokens
             WHERE token_hash = ?1 AND token_type = 'access' AND revoked_at = '' AND expires_at > ?2",
        )
        .bind(&[sha256_hex(token).into(), now_iso().into()])
        .map_err(|_| unauthorized(env))?;
    let row = statement
        .first::<Value>(None)
        .await
        .map_err(|_| unauthorized(env))?
        .ok_or_else(|| unauthorized(env))?;
    if row.get("resource").and_then(Value::as_str) != Some(resource_url(env).as_str()) {
        return Err(unauthorized(env));
    }
    Ok(McpPrincipal {
        scope: row
            .get("scope")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
}

fn unauthorized(env: &Env) -> Response {
    let mut response = (StatusCode::UNAUTHORIZED, "MCP authorization required").into_response();
    let value = format!(
        "Bearer resource_metadata=\"{}\"",
        protected_resource_metadata_url(env)
    );
    if let Ok(header_value) = HeaderValue::from_str(&value) {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, header_value);
    }
    response
}

#[worker::send]
pub async fn protected_resource_metadata(
    Extension(env): Extension<SendWrapper<Env>>,
) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({
            "resource": resource_url(&env),
            "authorization_servers": [issuer(&env)],
            "scopes_supported": [READ_SCOPE, WRITE_SCOPE],
            "bearer_methods_supported": ["header"],
        })),
    )
}

#[worker::send]
pub async fn authorization_server_metadata(
    Extension(env): Extension<SendWrapper<Env>>,
) -> impl IntoResponse {
    let base = issuer(&env);
    (
        StatusCode::OK,
        Json(json!({
            "issuer": base,
            "authorization_endpoint": format!("{}/oauth/authorize", issuer(&env)),
            "token_endpoint": format!("{}/oauth/token", issuer(&env)),
            "response_types_supported": ["code"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "code_challenge_methods_supported": ["S256"],
            "scopes_supported": [READ_SCOPE, WRITE_SCOPE],
            "token_endpoint_auth_methods_supported": ["none"],
            "client_id_metadata_document_supported": true,
        })),
    )
}

#[worker::send]
pub async fn mcp_endpoint(
    Extension(env): Extension<SendWrapper<Env>>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Response {
    let principal = match authenticate(&env, &headers).await {
        Ok(principal) => principal,
        Err(response) => return response,
    };
    handle_rpc(&env, principal, payload).await
}

async fn handle_rpc(env: &Env, principal: McpPrincipal, payload: Value) -> Response {
    if payload.is_array() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "JSON-RPC batches are not supported; send one MCP request at a time" })),
        )
            .into_response();
    }
    let id = payload.get("id").cloned().unwrap_or(Value::Null);
    let method = match payload.get("method").and_then(Value::as_str) {
        Some(method) => method,
        None => {
            return (
                StatusCode::OK,
                Json(rpc_error(id, -32600, "Invalid JSON-RPC request")),
            )
                .into_response()
        }
    };

    if method.starts_with("notifications/") {
        return StatusCode::ACCEPTED.into_response();
    }
    let params = payload.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match method {
        "initialize" => {
            let protocol_version = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(MCP_PROTOCOL_VERSION);
            Ok(json!({
                "protocolVersion": protocol_version,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": MCP_SERVER_NAME, "version": MCP_SERVER_VERSION },
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => call_tool(env, &principal, &params).await,
        _ => Err((-32601, format!("Method not found: {}", method))),
    };
    let response = match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => rpc_error(id, code, &message),
    };
    (StatusCode::OK, Json(response)).into_response()
}

fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    })
}

fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "list_tasks",
            "description": "List the private schedule todo tasks, including their AI-selected time blocks and current status.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "openWorldHint": false },
        }),
        json!({
            "name": "create_task",
            "description": "Create a private todo task. The existing schedule planner uses the class timetable, active tasks, deadlines, and previous scheduling experience before choosing a time.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "A concise task title." },
                    "notes": { "type": "string", "description": "Optional context or acceptance criteria." },
                    "deadline": { "type": "string", "description": "Optional RFC3339 deadline, for example 2026-09-25T23:59:00-04:00." },
                    "timezone": { "type": "string", "description": "Optional IANA timezone label used as context for scheduling." },
                },
                "required": ["title"],
                "additionalProperties": false,
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "openWorldHint": false },
        }),
        json!({
            "name": "update_task",
            "description": "Update a private todo task. Set replan to true when its time block should be selected again around classes and other tasks.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer" },
                    "title": { "type": "string" },
                    "notes": { "type": "string" },
                    "deadline": { "type": "string" },
                    "timezone": { "type": "string" },
                    "replan": { "type": "boolean" },
                },
                "required": ["id", "title"],
                "additionalProperties": false,
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "openWorldHint": false },
        }),
        json!({
            "name": "delete_task",
            "description": "Delete a private todo task and its review/reaction records.",
            "inputSchema": { "type": "object", "properties": { "id": { "type": "integer" } }, "required": ["id"], "additionalProperties": false },
            "annotations": { "readOnlyHint": false, "destructiveHint": true, "openWorldHint": false },
        }),
        json!({
            "name": "list_reviews",
            "description": "List tasks whose scheduled time has passed and are waiting for a completion/defer/skip decision.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "openWorldHint": false },
        }),
        json!({
            "name": "save_review",
            "description": "Record whether a due task was completed, deferred, or skipped. The existing Worker will update the task and refresh the AI scheduling experience.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "review_id": { "type": "integer" },
                    "outcome": { "type": "string", "enum": ["completed", "deferred", "skipped"] },
                    "spent_minutes": { "type": "integer", "minimum": 0, "maximum": 1440 },
                    "summary": { "type": "string", "description": "One or two sentences about what happened." },
                    "next_start": { "type": "string", "description": "Optional RFC3339 time when a deferred task should resume." },
                },
                "required": ["review_id", "outcome"],
                "additionalProperties": false,
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "openWorldHint": false },
        }),
        json!({
            "name": "get_scheduling_experience",
            "description": "Read the AI-generated previous scheduling experience before discussing or creating another task.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "openWorldHint": false },
        }),
    ]
}

async fn call_tool(
    env: &Env,
    principal: &McpPrincipal,
    params: &Value,
) -> Result<Value, (i32, String)> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or((-32602, "tools/call requires a tool name".to_string()))?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let requires_write = matches!(
        name,
        "create_task" | "update_task" | "delete_task" | "save_review"
    );
    let required_scope = if requires_write {
        WRITE_SCOPE
    } else {
        READ_SCOPE
    };
    if !principal.has_scope(required_scope) {
        return Err((
            -32001,
            format!("OAuth token does not include {}", required_scope),
        ));
    }

    let response = match name {
        "list_tasks" => schedule::list_owner_tasks(Extension(SendWrapper::new(env.clone())))
            .await
            .into_response(),
        "create_task" => {
            let title = required_string(&arguments, "title")?;
            schedule::add_task(
                Extension(SendWrapper::new(env.clone())),
                Json(TaskCreateDTO {
                    title,
                    notes: optional_string(&arguments, "notes"),
                    deadline: optional_string_value(&arguments, "deadline"),
                    timezone: optional_string_value(&arguments, "timezone"),
                }),
            )
            .await
            .into_response()
        }
        "update_task" => {
            let id = required_i64(&arguments, "id")?;
            schedule::update_task(
                Extension(SendWrapper::new(env.clone())),
                axum::extract::Path(id),
                Json(TaskUpdateDTO {
                    title: required_string(&arguments, "title")?,
                    notes: optional_string(&arguments, "notes"),
                    deadline: optional_string_value(&arguments, "deadline"),
                    timezone: optional_string_value(&arguments, "timezone"),
                    replan: arguments
                        .get("replan")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                }),
            )
            .await
            .into_response()
        }
        "delete_task" => {
            let id = required_i64(&arguments, "id")?;
            schedule::delete_task(
                Extension(SendWrapper::new(env.clone())),
                axum::extract::Path(id),
            )
            .await
            .into_response()
        }
        "list_reviews" => schedule::get_reviews(Extension(SendWrapper::new(env.clone())))
            .await
            .into_response(),
        "save_review" => {
            let review_id = required_i64(&arguments, "review_id")?;
            schedule::save_review(
                Extension(SendWrapper::new(env.clone())),
                axum::extract::Path(review_id),
                Json(ReviewDTO {
                    outcome: required_string(&arguments, "outcome")?,
                    spent_minutes: arguments
                        .get("spent_minutes")
                        .and_then(Value::as_i64)
                        .unwrap_or(0)
                        .clamp(0, 1440) as i32,
                    summary: optional_string(&arguments, "summary"),
                    next_start: optional_string_value(&arguments, "next_start"),
                }),
            )
            .await
            .into_response()
        }
        "get_scheduling_experience" => {
            schedule::get_learning(Extension(SendWrapper::new(env.clone())))
                .await
                .into_response()
        }
        _ => return Err((-32602, format!("Unknown tool: {}", name))),
    };
    let status = response.status();
    let body = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .map_err(|error| {
            (
                -32000,
                format!("schedule response could not be read: {}", error),
            )
        })?;
    let value = serde_json::from_slice::<Value>(&body)
        .map_err(|error| (-32000, format!("schedule response was not JSON: {}", error)))?;
    if !status.is_success() {
        return Ok(tool_result(value, true));
    }
    Ok(tool_result(value, false))
}

fn required_string(arguments: &Value, key: &str) -> Result<String, (i32, String)> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or((-32602, format!("{} is required", key)))
}

fn optional_string(arguments: &Value, key: &str) -> String {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn optional_string_value(arguments: &Value, key: &str) -> Option<String> {
    let value = optional_string(arguments, key);
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn required_i64(arguments: &Value, key: &str) -> Result<i64, (i32, String)> {
    arguments
        .get(key)
        .and_then(Value::as_i64)
        .ok_or((-32602, format!("{} must be an integer", key)))
}

fn tool_result(value: Value, is_error: bool) -> Value {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": value,
        "isError": is_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_has_no_padding() {
        assert_eq!(base64url(&[0, 1, 2]), "AAEC");
        assert_eq!(base64url(&[251, 255]), "-_8");
    }

    #[test]
    fn pkce_sha256_matches_rfc7636_example() {
        assert_eq!(
            sha256_base64url("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn scopes_are_restricted_and_deduplicated() {
        assert_eq!(
            normalize_scope(Some("schedule:write schedule:read schedule:write")).unwrap(),
            "schedule:write schedule:read"
        );
        assert!(normalize_scope(Some("openid")).is_err());
    }

    #[test]
    fn chatgpt_redirect_checks_are_origin_scoped() {
        assert!(valid_chatgpt_client(
            "https://chatgpt.com/oauth/client.json"
        ));
        assert!(valid_chatgpt_redirect(
            "https://chatgpt.com/connector_platform_oauth_redirect"
        ));
        assert!(!valid_chatgpt_redirect(
            "https://chatgpt.com.attacker.example/callback"
        ));
    }
}
