//! R2 媒体存取。
//!
//! 约定(按用户选择):
//! - 读取走 Worker 代理公开读: `GET /api/media/{*key}` (无需登录,带缓存头)
//! - 上传走 Worker 中转: `POST /api/protected/media?kind=cover|content&filename=xxx`
//!   Body 为文件原始字节(前端 `fetch(body=file)` 直传,不用 multipart,省依赖)。
//!   需要 Cloudflare Access 登录(受 `auth_middleware` 保护)。
//! - 删除: `DELETE /api/protected/media?key=...`
//!
//! Key 规范:
//! - 封面: `covers/<ts>-<hash>-<safe-name>`
//! - 正文: `posts/<ts>-<hash>-<safe-name>.md`
//! D1 只存 key(`content_key`/`cover_key`),见 `migrations/0003_drop_legacy_columns.sql`。

use axum::{
    body::Bytes,
    extract::Query,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use worker::send::SendWrapper;
use worker::Env;

const MAX_COVER_BYTES: usize = 8 * 1024 * 1024;
const MAX_CONTENT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Deserialize)]
pub struct UploadQuery {
    pub kind: Option<String>,
    pub filename: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DeleteQuery {
    pub key: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UploadResponse {
    pub key: String,
    pub url: String,
}

fn sanitize_filename(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let mut s: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches(['-', '.', ' ']).to_string();
    let s = if s.is_empty() { "file".to_string() } else { s };
    s.chars().take(80).collect()
}

fn fnv1a64(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in data {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn ext_of(name: &str) -> String {
    name.rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect()
}

fn content_type_for(kind: &str, filename: &str, header_ct: Option<&str>) -> String {
    if let Some(ct) = header_ct {
        let ct = ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
        if !ct.is_empty() && ct.contains('/') && !ct.contains("multipart") {
            // 封面只允许图片,正文只允许文本,防止把可执行文件顶着图片头存进去
            let ok = if kind == "cover" {
                ct.starts_with("image/")
            } else {
                matches!(
                    ct.as_str(),
                    "text/markdown" | "text/plain" | "text/x-markdown"
                )
            };
            if ok {
                return ct;
            }
        }
    }
    let ext = ext_of(filename);
    match kind {
        "cover" => match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "svg" => "image/svg+xml",
            "avif" => "image/avif",
            _ => "image/webp",
        }
        .to_string(),
        _ => "text/markdown; charset=utf-8".to_string(),
    }
}

fn validate(kind: &str, filename: &str, len: usize) -> Result<(), (StatusCode, String)> {
    if kind != "cover" && kind != "content" {
        return Err((
            StatusCode::BAD_REQUEST,
            "kind must be cover|content".to_string(),
        ));
    }
    if len == 0 {
        return Err((StatusCode::BAD_REQUEST, "empty body".to_string()));
    }
    if kind == "cover" {
        if len > MAX_COVER_BYTES {
            return Err((StatusCode::PAYLOAD_TOO_LARGE, "cover > 8MB".to_string()));
        }
        let ext = ext_of(filename);
        if !matches!(
            ext.as_str(),
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "avif" | "svg"
        ) {
            return Err((
                StatusCode::BAD_REQUEST,
                "cover ext must be png/jpg/webp/gif/avif/svg".to_string(),
            ));
        }
    } else {
        if len > MAX_CONTENT_BYTES {
            return Err((
                StatusCode::PAYLOAD_TOO_LARGE,
                "markdown > 2MB".to_string(),
            ));
        }
        let ext = ext_of(filename);
        if !matches!(ext.as_str(), "md" | "markdown" | "txt" | "") {
            return Err((
                StatusCode::BAD_REQUEST,
                "content ext must be .md/.txt".to_string(),
            ));
        }
    }
    Ok(())
}

pub fn check_key(key: &str) -> bool {
    if key.is_empty() || key.len() > 256 || key.starts_with('/') || key.contains("..") {
        return false;
    }
    let ok_prefix = key.starts_with("covers/") || key.starts_with("posts/");
    if !ok_prefix {
        return false;
    }
    key.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '-' | '_'))
}

/// 供 post.rs 复用:删文章时尽力删 R2(失败只打日志,不阻断删 D1)。
pub async fn best_effort_delete(env: &Env, key: &str) {
    if !check_key(key) {
        return;
    }
    let Ok(bucket) = env.bucket("MEDIA") else {
        return;
    };
    let _ = bucket.delete(key).await;
}

#[worker::send]
pub async fn upload_media(
    Extension(env): Extension<SendWrapper<Env>>,
    Query(q): Query<UploadQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let kind = q.kind.unwrap_or_default().to_ascii_lowercase();
    let filename = sanitize_filename(&q.filename.unwrap_or_else(|| {
        if kind == "cover" {
            "cover.webp".to_string()
        } else {
            "post.md".to_string()
        }
    }));
    if let Err((code, msg)) = validate(&kind, &filename, body.len()) {
        return (code, Json(serde_json::json!({ "error": msg }))).into_response();
    }
    let header_ct = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let ct = content_type_for(&kind, &filename, header_ct.as_deref());

    let ts = worker::Date::now().as_millis();
    let hash = format!("{:x}", fnv1a64(&body) ^ (ts as u64));
    let short_hash = &hash[..hash.len().min(8)];
    let prefix = if kind == "cover" { "covers" } else { "posts" };
    // 正文强制 .md 结尾,方便读回时识别
    let mut fname = filename.clone();
    if kind == "content" && !fname.to_ascii_lowercase().ends_with(".md") {
        // 去掉原 txt 后缀再加 md,无后缀直接加
        if let Some(dot) = fname.rfind('.') {
            fname.truncate(dot);
        }
        fname.push_str(".md");
    }
    let key = format!("{}/{}-{}-{}", prefix, ts, short_hash, fname);

    let bucket = match env.bucket("MEDIA") {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("R2 binding MEDIA missing: {}", e) })),
            )
                .into_response()
        }
    };
    let meta = worker::HttpMetadata {
        content_type: Some(ct),
        cache_control: Some(if kind == "cover" {
            "public, max-age=31536000, immutable".to_string()
        } else {
            "public, max-age=60".to_string()
        }),
        ..Default::default()
    };
    if let Err(e) = bucket
        .put(key.clone(), body.to_vec())
        .http_metadata(meta)
        .execute()
        .await
    {
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": format!("R2 put failed: {}", e) })),
        )
            .into_response();
    }
    (
        StatusCode::OK,
        Json(UploadResponse {
            url: format!("/api/media/{}", key),
            key,
        }),
    )
        .into_response()
}

#[worker::send]
pub async fn get_media(
    Extension(env): Extension<SendWrapper<Env>>,
    axum::extract::Path(key): axum::extract::Path<String>,
) -> impl IntoResponse {
    if !check_key(&key) {
        return (StatusCode::BAD_REQUEST, "invalid key".to_string()).into_response();
    }
    let bucket = match env.bucket("MEDIA") {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("R2 binding MEDIA missing: {}", e),
            )
                .into_response()
        }
    };
    let obj = match bucket.get(key.clone()).execute().await {
        Ok(o) => o,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!("R2 get failed: {}", e),
            )
                .into_response()
        }
    };
    let Some(obj) = obj else {
        return (StatusCode::NOT_FOUND, "media not found".to_string()).into_response();
    };
    let ct = obj
        .http_metadata()
        .content_type
        .unwrap_or_else(|| {
            if key.ends_with(".md") {
                "text/markdown; charset=utf-8".to_string()
            } else {
                "application/octet-stream".to_string()
            }
        });
    let Some(wbody) = obj.body() else {
        return (StatusCode::NOT_FOUND, "media empty".to_string()).into_response();
    };
    let bytes = match wbody.bytes().await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!("R2 read failed: {}", e),
            )
                .into_response()
        }
    };
    let mut headers = HeaderMap::new();
    if let Ok(v) = axum::http::HeaderValue::from_str(&ct) {
        headers.insert(axum::http::header::CONTENT_TYPE, v);
    }
    let cache = if key.starts_with("covers/") {
        "public, max-age=31536000, immutable"
    } else {
        "public, max-age=60"
    };
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static(cache),
    );
    (StatusCode::OK, headers, bytes).into_response()
}

#[worker::send]
pub async fn delete_media(
    Extension(env): Extension<SendWrapper<Env>>,
    Query(q): Query<DeleteQuery>,
) -> impl IntoResponse {
    let key = q.key.unwrap_or_default();
    if !check_key(&key) {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid key" })))
            .into_response();
    }
    let bucket = match env.bucket("MEDIA") {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("R2 binding MEDIA missing: {}", e) })),
            )
                .into_response()
        }
    };
    if let Err(e) = bucket.delete(key).await {
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": format!("R2 delete failed: {}", e) })),
        )
            .into_response();
    }
    (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
}
