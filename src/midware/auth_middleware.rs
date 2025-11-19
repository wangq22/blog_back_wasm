use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;

const ISSUER: &str = "https://auth-backend.charlie-cloud.me/realms/charlie-cloud";
const CLIENT_ID: &str = "blog";
const JWKS_URL: &str =
    "https://auth-backend.charlie-cloud.me/realms/charlie-cloud/protocol/openid-connect/certs";

#[derive(Debug, Deserialize)]
struct Claims {
    sub: String,
    exp: usize,
    iss: String,
    aud: Option<Vec<String>>,
    preferred_username: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Jwk {
    kid: String,
    n: String,
    e: String,
    kty: String,
}
#[derive(Debug, Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

#[worker::send]
pub async fn auth_middleware(req: Request<Body>, next: Next) -> Result<Response, Response> {
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
        .get(JWKS_URL)
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
    let _kty = &jwk.kty;

    let decoding_key = DecodingKey::from_rsa_components(&jwk.n, &jwk.e)
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid JWK components").into_response())?;

    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[ISSUER]);
    validation.set_audience(&[CLIENT_ID]);

    let Claims {
        sub: _sub,
        exp: _exp,
        iss: _iss,
        aud: _aud,
        preferred_username: _preferred_username,
    } = decode::<Claims>(token, &decoding_key, &validation)
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Token validation failed").into_response())?
        .claims;

    Ok(next.run(req).await)
}
