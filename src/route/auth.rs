//! BFF 登录代换接口。
//!
//! 背景:Cloudflare Access SaaS OIDC 的 `/token` 端点不返回 CORS 头,
//! 浏览器 SPA 无法直接完成 code→token 交换(表现为 CORS blocked,真实错误被盖住)。
//! 因此 code 交换与 refresh 都由 Worker 服务端代发(无 CORS 限制),浏览器只跟同源可控的
//! 本 API 打交道(本 API 的 CORS 已全开)。
//!
//! - POST /api/auth/exchange {code, code_verifier, redirect_uri} -> Access token 响应原样
//! - POST /api/auth/refresh  {refresh_token}                     -> Access token 响应原样
//!
//! 机密客户端模式:如配了 `CF_ACCESS_CLIENT_SECRET`(wrangler secret,禁止进明文 vars),
//! 代换时会自动带上 secret;没配则走纯 PKCE 公开客户端模式,两种都兼容。

use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

use axum::{extract::State, http::StatusCode, Json};
use serde::Deserialize;

use crate::midware::auth_middleware::CfAccessConfig;

#[derive(Debug, Clone)]
pub struct AuthState {
    pub cfg: Arc<CfAccessConfig>,
    pub client_secret: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExchangeRequest {
    code: String,
    code_verifier: String,
    redirect_uri: String,
}

#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    refresh_token: String,
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

/// 把 Access 的 HTML 错误页提炼成可读文本(去标签/压空白)。
fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len().min(2048));
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(target_arch = "wasm32")]
fn log_upstream(prefix: &str, body: &str) {
    worker::console_log!("{}: {}", prefix, truncate(body.trim(), 4000));
}

#[cfg(not(target_arch = "wasm32"))]
fn log_upstream(_prefix: &str, _body: &str) {}

async fn proxy_token(
    state: &AuthState,
    params: Vec<(&str, String)>,
) -> Result<serde_json::Value, (StatusCode, String)> {
    // wasm32 的 fetch 后端不支持 timeout,只在非 wasm 目标上设置
    let builder = reqwest::Client::builder();
    #[cfg(not(target_arch = "wasm32"))]
    let builder = builder.timeout(Duration::from_secs(10));
    let client = builder.build().map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("HTTP client error: {}", e),
        )
    })?;

    let mut form: Vec<(String, String)> = vec![("client_id".to_string(), state.cfg.audience.clone())];
    for (k, v) in params {
        form.push((k.to_string(), v));
    }
    if let Some(secret) = state.client_secret.as_ref() {
        form.push(("client_secret".to_string(), secret.clone()));
    }

    let res = client
        .post(state.cfg.token_url())
        .form(&form)
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Access token endpoint unreachable: {}", e),
            )
        })?;
    let status = res.status();
    let body = res.text().await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("Read token response failed: {}", e),
        )
    })?;
    // 注意:此端点报 grant 错误时返回 302(redirect_uri?error=...)而非 JSON;
    // 在 wasm 下 fetch 会自动跟随跳转,最终拿到的是 blog 回调页 HTML。
    // 只要不是 JSON,一律按 HTML/异常处理,提炼可读信息。
    let trimmed = body.trim();
    let is_json = trimmed.starts_with('{');
    if !status.is_success() || !is_json {
        log_upstream("Access token endpoint raw response", &body);
        if is_json {
            return Err((
                StatusCode::BAD_GATEWAY,
                format!(
                    "Access rejected token request ({}): {}",
                    status.as_u16(),
                    truncate(trimmed, 500)
                ),
            ));
        }
        let text = html_to_text(trimmed);
        if status.is_server_error() {
            return Err((
                StatusCode::BAD_GATEWAY,
                format!(
                    "Access 换 token 时内部错误 ({}): {}",
                    status.as_u16(),
                    truncate(&text, 800)
                ),
            ));
        }
        return Err((
            StatusCode::BAD_GATEWAY,
            format!(
                "Access 未返回 token ({}): {}。code 可能已使用/过期,请重新登录再试",
                status.as_u16(),
                truncate(&text, 300)
            ),
        ));
    }
    serde_json::from_str(trimmed).map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("Invalid token response: {}", e),
        )
    })
}

#[worker::send]
pub async fn exchange(
    State(state): State<AuthState>,
    Json(req): Json<ExchangeRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if req.code.is_empty() || req.code_verifier.is_empty() || req.redirect_uri.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "code / code_verifier / redirect_uri required".to_string(),
        ));
    }
    let v = proxy_token(
        &state,
        vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", req.code),
            ("redirect_uri", req.redirect_uri),
            ("code_verifier", req.code_verifier),
        ],
    )
    .await?;
    Ok(Json(v))
}

#[worker::send]
pub async fn refresh(
    State(state): State<AuthState>,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if req.refresh_token.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "refresh_token required".to_string(),
        ));
    }
    let v = proxy_token(
        &state,
        vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", req.refresh_token),
        ],
    )
    .await?;
    Ok(Json(v))
}
