use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;

/// Cloudflare Access SaaS OIDC 配置。
/// 推荐只配两个 vars,另外三个自动推导:
///   CF_ACCESS_TEAM_DOMAIN = https://<team>.cloudflareaccess.com (结尾不带 /)
///   CF_ACCESS_CLIENT_ID   = Access SaaS 应用的 Client ID
/// 推导:
///   issuer   = {team}/cdn-cgi/access/sso/oidc/{client_id}
///   jwks_url = {issuer}/jwks
///   audience = {client_id}
/// 也支持直接覆盖 CF_ACCESS_ISSUER / CF_ACCESS_AUDIENCE / CF_ACCESS_JWKS_URL。
#[derive(Debug, Clone)]
pub struct CfAccessConfig {
    pub issuer: String,
    pub audience: String,
    pub jwks_url: String,
}

impl CfAccessConfig {
    pub fn from_env(env: &worker::Env) -> Result<Self, String> {
        if let (Ok(issuer), Ok(audience), Ok(jwks_url)) = (
            env.var("CF_ACCESS_ISSUER"),
            env.var("CF_ACCESS_AUDIENCE"),
            env.var("CF_ACCESS_JWKS_URL"),
        ) {
            return Ok(Self {
                issuer: issuer.to_string(),
                audience: audience.to_string(),
                jwks_url: jwks_url.to_string(),
            });
        }
        let team = env
            .var("CF_ACCESS_TEAM_DOMAIN")
            .map(|v| v.to_string())
            .map_err(|_| {
                "Missing CF_ACCESS_TEAM_DOMAIN (e.g. https://<team>.cloudflareaccess.com)"
                    .to_string()
            })?;
        let client_id = env
            .var("CF_ACCESS_CLIENT_ID")
            .map(|v| v.to_string())
            .map_err(|_| "Missing CF_ACCESS_CLIENT_ID (Access SaaS app Client ID)".to_string())?;
        let team = team.trim_end_matches('/').to_string();
        let issuer = format!("{}/cdn-cgi/access/sso/oidc/{}", team, client_id);
        Ok(Self {
            jwks_url: format!("{}/jwks", issuer),
            audience: client_id,
            issuer,
        })
    }

    pub fn token_url(&self) -> String {
        format!("{}/token", self.issuer)
    }
}

#[derive(Debug, Deserialize)]
struct Claims {
    sub: String,
    exp: usize,
    iss: String,
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Jwk {
    kid: String,
    n: String,
    e: String,
    #[allow(dead_code)]
    kty: String,
}

#[derive(Debug, Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

#[worker::send]
pub async fn auth_middleware(
    State(cfg): State<Arc<CfAccessConfig>>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, Response> {
    // Local API tests can opt into a bypass only with this exact loopback
    // issuer. Production Cloudflare Access configurations never match it.
    if cfg.issuer == "http://localhost/cdn-cgi/access/sso/oidc/local-test"
        && req
            .headers()
            .get("x-local-test-auth")
            .and_then(|value| value.to_str().ok())
            == Some("1")
    {
        return Ok(next.run(req).await);
    }

    let auth = match req.headers().get(axum::http::header::AUTHORIZATION) {
        Some(v) => v.to_str().unwrap_or_default(),
        None => {
            return Err((StatusCode::UNAUTHORIZED, "Missing Authorization header").into_response())
        }
    };

    if !auth.starts_with("Bearer ") {
        return Err((StatusCode::UNAUTHORIZED, "Invalid Authorization header").into_response());
    }
    let token = &auth["Bearer ".len()..];

    let header = decode_header(token)
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid token header").into_response())?;
    let kid = header
        .kid
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Missing kid").into_response())?;

    let jwks: Jwks = reqwest::Client::new()
        .get(cfg.jwks_url.as_str())
        .send()
        .await
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Failed to fetch JWKS").into_response())?
        .json()
        .await
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Failed to parse JWKS").into_response())?;

    let jwk = jwks
        .keys
        .iter()
        .find(|j| j.kid == kid)
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "No matching JWK").into_response())?;

    let decoding_key = DecodingKey::from_rsa_components(&jwk.n, &jwk.e)
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid JWK components").into_response())?;

    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[cfg.issuer.as_str()]);
    validation.set_audience(&[cfg.audience.as_str()]);

    let Claims {
        sub: _sub,
        exp: _exp,
        iss: _iss,
        email: _email,
    } = decode::<Claims>(token, &decoding_key, &validation)
        .map_err(|e| {
            (
                StatusCode::UNAUTHORIZED,
                format!("Token validation failed: {:?} ({:?})", e, e.kind()),
            )
                .into_response()
        })?
        .claims;

    Ok(next.run(req).await)
}
