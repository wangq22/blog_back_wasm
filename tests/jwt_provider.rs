//! 回归测试:jsonwebtoken v11 必须启用 `rust_crypto`(或 `aws_lc_rs`)二选一,
//! 否则任何 `decode()`(即任何带 token 的 `/api/protected/*` 请求)在 wasm 线上
//! 以 `CryptoProvider::from_crate_features` panic 炸掉整个请求(见 2026-09 线上事故)。
//! 此测试走与 `middleware::auth_middleware` 相同的解码路径,缺 provider 时会 panic 而非返回 Err。

use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Claims {
    sub: String,
    exp: usize,
    iss: String,
}

#[test]
fn rs256_verify_path_returns_err_instead_of_panicking() {
    // 与中间件同路:JuKS 的 n/e -> DecodingKey -> RS256 decode。
    // n/e 取假值,签名必错,关键是必须走完 verifier_factory 回到 Err,而不是 panic。
    let key = DecodingKey::from_rsa_components("AQAB", "AQAB").expect("key build");
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&["https://example.com"]);
    validation.set_audience(&["aud"]);
    // header {"alg":"RS256","typ":"JWT"} / payload {"sub":"x","exp":9999999999,"iss":"https://example.com"}
    let token = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ4IiwiZXhwIjo5OTk5OTk5OTk5LCJpc3MiOiJodHRwczovL2V4YW1wbGUuY29tIn0.c2ln";
    let res = decode::<Claims>(token, &key, &validation);
    assert!(res.is_err(), "expected InvalidSignature, got {:?}", res);
}
