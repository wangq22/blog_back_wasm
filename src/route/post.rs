//! 文章 CRUD(D1 只存 R2 key,不兼容老 content/cover_image 列)。
//!
//! - 列表 `GET /api/posts`:只查 D1,不读 R2(无正文)。
//! - 详情 `GET /api/post/{id}`:按 `content_key` 读 R2 回填 `content`。
//! - 新建/更新:调用方须先 `POST /api/protected/media` 拿到 key,此处只校验 key 格式并存 key。

use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use time::format_description::well_known::Rfc3339;
use worker::{send::SendWrapper, Env};

use crate::model::post::{PostDetailDTO, PostInsertDTO, PostUpdateDTO};
use crate::route::media::{best_effort_delete, check_key};

#[derive(Deserialize, Serialize, Clone)]
pub struct Page {
    page: u32,
    page_size: u32,
}

#[derive(Serialize)]
pub struct PageResponse<T> {
    data: Option<T>,
    meta: Page,
}

#[derive(Debug, Deserialize)]
struct PostRow {
    _id: i32,
    title: String,
    excerpt: String,
    date: String,
    category: String,
    content_key: String,
    #[serde(default)]
    cover_key: Option<String>,
    word_count: i32,
    read_time: i32,
}

async fn r2_text(env: &Env, key: &str) -> Result<String, String> {
    let bucket = env
        .bucket("MEDIA")
        .map_err(|e| format!("R2 binding MEDIA missing: {}", e))?;
    let obj = bucket
        .get(key.to_string())
        .execute()
        .await
        .map_err(|e| format!("R2 get failed: {}", e))?
        .ok_or_else(|| format!("R2 object not found: {}", key))?;
    let body = obj.body().ok_or_else(|| "R2 object empty".to_string())?;
    body.text()
        .await
        .map_err(|e| format!("R2 read failed: {}", e))
}

/// 增/改/删文章成功后触发 Pages 重建。
/// hook URL 配在 secret `PAGES_DEPLOY_HOOK_URL`(Pages Build Hook),未配则静默跳过;
/// hook 本身失败只打日志,绝不影响主流程的响应。
/// 用户资料页同样预渲染,改资料时复用此钩子(见 route::user::update_user)。
pub(crate) async fn trigger_pages_rebuild(env: &Env) {
    let url = match env.var("PAGES_DEPLOY_HOOK_URL") {
        Ok(v) => v.to_string(),
        Err(_) => return,
    };
    let url = url.trim().to_string();
    if url.is_empty() {
        return;
    }
    match reqwest::Client::new().post(url).send().await {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => eprintln!("pages rebuild hook returned {}", r.status()),
        Err(e) => eprintln!("pages rebuild hook failed: {}", e),
    }
}

fn check_keys(
    content_key: &str,
    cover_key: &Option<String>,
) -> Result<(String, String), (StatusCode, String)> {
    let content_key = content_key.trim().to_string();
    if content_key.is_empty() || !check_key(&content_key) {
        return Err((StatusCode::BAD_REQUEST, "invalid content_key".to_string()));
    }
    let cover_key = cover_key.clone().unwrap_or_default().trim().to_string();
    if !cover_key.is_empty() && !check_key(&cover_key) {
        return Err((StatusCode::BAD_REQUEST, "invalid cover_key".to_string()));
    }
    Ok((content_key, cover_key))
}

#[worker::send]
pub async fn get_all_posts(
    Extension(env): Extension<SendWrapper<Env>>,
    Query(param): Query<Page>,
) -> impl IntoResponse {
    let db = env.d1("DB").unwrap();
    let offset = (param.page.saturating_sub(1) * param.page_size) as i32;

    let result = db
        .prepare(
            "SELECT p._id, p.title, p.excerpt, p.category, p.date,
             p.content_key, p.cover_key, p.word_count, p.read_time,
             json_group_array(t.name) AS tags
             FROM posts AS p
             LEFT JOIN post_tags AS pt ON p._id = pt.post_id
             LEFT JOIN tags AS t ON pt.tag_id = t._id
             GROUP BY p._id
             LIMIT ?2 OFFSET ?1",
        )
        .bind(&[offset.into(), param.page_size.into()])
        .unwrap()
        .all()
        .await;
    let result = match result {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("DB error: {}", e),
            )
                .into_response()
        }
    };
    let rows = result.results::<Value>().unwrap_or_default();
    let rows: Vec<_> = rows
        .into_iter()
        .map(|mut row| {
            if let Some(tags_str) = row.get("tags").and_then(|v| v.as_str()) {
                if let Ok(tags_json) = serde_json::from_str::<serde_json::Value>(tags_str) {
                    row["tags"] = tags_json;
                }
            }
            row
        })
        .collect();

    let response = PageResponse {
        data: Some(rows),
        meta: param.clone(),
    };
    (StatusCode::OK, Json(response)).into_response()
}

#[worker::send]
pub async fn get_post_detail(
    Extension(env): Extension<SendWrapper<Env>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    let db = env.d1("DB").unwrap();

    let row: Option<PostRow> = match db
        .prepare("SELECT * FROM posts WHERE _id = ?1")
        .bind(&[id.into()])
        .unwrap()
        .first(None)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("DB error: {}", e),
            )
                .into_response()
        }
    };
    let Some(row) = row else {
        return (StatusCode::NOT_FOUND, "Post not found").into_response();
    };
    if !check_key(&row.content_key) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Invalid content_key: {}", row.content_key),
        )
            .into_response();
    }
    let content = match r2_text(&env, &row.content_key).await {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!("Read R2 markdown failed ({}): {}", row.content_key, e),
            )
                .into_response()
        }
    };

    let tag_rows_res = db
        .prepare(
            "SELECT t.name
             FROM post_tags pt
             JOIN tags t ON t._id = pt.tag_id
             WHERE pt.post_id = ?1",
        )
        .bind(&[id.into()])
        .unwrap()
        .all()
        .await;
    let tag_list = match tag_rows_res {
        Ok(rows) => match rows.results::<TagNameRow>() {
            Ok(vec_rows) => vec_rows.into_iter().map(|r| r.name).collect::<Vec<_>>(),
            Err(_) => vec![],
        },
        Err(_) => vec![],
    };

    let date = time::OffsetDateTime::parse(&row.date, &Rfc3339)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    let post = PostDetailDTO {
        _id: row._id,
        title: row.title,
        excerpt: row.excerpt,
        date,
        category: row.category,
        content,
        content_key: row.content_key,
        cover_key: row.cover_key,
        tags: Some(tag_list),
        word_count: row.word_count,
        read_time: row.read_time,
    };
    (StatusCode::OK, Json(post)).into_response()
}

#[derive(Debug, Deserialize)]
pub struct TagNameRow {
    pub name: String,
}

async fn save_tags(env: &Env, post_id: i32, tags: &Option<Vec<String>>) -> Result<(), String> {
    let Some(tag_list) = tags else { return Ok(()) };
    let db = env.d1("DB").map_err(|e| e.to_string())?;
    for tag_name in tag_list {
        let existing = db
            .prepare("SELECT _id FROM tags WHERE name = ?1")
            .bind(&[tag_name.into()])
            .unwrap()
            .first::<TagRow>(None)
            .await
            .map_err(|e| e.to_string())?;
        let tag_id: i32 = match existing {
            Some(row) => row._id,
            None => {
                let r = db
                    .prepare("INSERT INTO tags (name, color_class) VALUES (?1, 'info_badge')")
                    .bind(&[tag_name.into()])
                    .unwrap()
                    .run()
                    .await
                    .map_err(|e| e.to_string())?;
                r.meta()
                    .unwrap()
                    .unwrap()
                    .last_row_id
                    .unwrap()
                    .try_into()
                    .unwrap()
            }
        };
        db.prepare("INSERT INTO post_tags (post_id, tag_id) VALUES (?1, ?2)")
            .bind(&[post_id.into(), tag_id.into()])
            .unwrap()
            .run()
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[worker::send]
pub async fn add_post(
    Extension(env): Extension<SendWrapper<Env>>,
    Json(post): Json<PostInsertDTO>,
) -> impl IntoResponse {
    let (content_key, cover_key) = match check_keys(&post.content_key, &post.cover_key) {
        Ok(v) => v,
        Err((c, m)) => return (c, Json(json!({ "error": m }))).into_response(),
    };
    let db = env.d1("DB").unwrap();
    let date = post.date.format(&Rfc3339).unwrap();

    let post_result = db
        .prepare("INSERT INTO posts (title, excerpt, date, category, content_key, cover_key, word_count, read_time)
                  VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)")
        .bind(&[
            post.title.clone().into(),
            post.excerpt.clone().into(),
            date.into(),
            post.category.clone().into(),
            content_key.into(),
            cover_key.into(),
            post.word_count.into(),
            post.read_time.into(),
        ])
        .unwrap()
        .run()
        .await;
    let post_result = match post_result {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("DB error: {}", e) })),
            )
                .into_response()
        }
    };
    let meta = post_result.meta().unwrap().ok_or("Missing meta").unwrap();
    let post_id: i32 = meta.last_row_id.ok_or("Missing last_row_id").unwrap() as i32;

    if let Err(e) = save_tags(&env, post_id, &post.tags).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Tag error: {}", e) })),
        )
            .into_response();
    }
    trigger_pages_rebuild(&env).await;
    (StatusCode::OK, Json(json!({ "id": post_id }))).into_response()
}

#[worker::send]
pub async fn update_post(
    Extension(env): Extension<SendWrapper<Env>>,
    Json(post): Json<PostUpdateDTO>,
) -> impl IntoResponse {
    let (content_key, cover_key) = match check_keys(&post.content_key, &post.cover_key) {
        Ok(v) => v,
        Err((c, m)) => return (c, Json(json!({ "error": m }))).into_response(),
    };
    let db = env.d1("DB").unwrap();
    let date = post.date.format(&Rfc3339).unwrap();

    if let Err(e) = db
        .prepare(
            "UPDATE posts
             SET title = ?1, excerpt = ?2, date = ?3, category = ?4,
                 content_key = ?5, cover_key = ?6, word_count = ?7, read_time = ?8
             WHERE _id = ?9",
        )
        .bind(&[
            post.title.clone().into(),
            post.excerpt.clone().into(),
            date.into(),
            post.category.clone().into(),
            content_key.into(),
            cover_key.into(),
            post.word_count.into(),
            post.read_time.into(),
            post._id.into(),
        ])
        .unwrap()
        .run()
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(format!("DB error: {}", e)),
        )
            .into_response();
    }

    db.prepare("DELETE FROM post_tags WHERE post_id = ?1")
        .bind(&[post._id.into()])
        .unwrap()
        .run()
        .await
        .unwrap();

    if let Err(e) = save_tags(&env, post._id, &post.tags).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(format!("Tag error: {}", e)),
        )
            .into_response();
    }
    trigger_pages_rebuild(&env).await;
    (StatusCode::OK, Json("update ok")).into_response()
}

#[derive(Debug, Deserialize)]
pub struct TagRow {
    pub _id: i32,
}

#[derive(Debug, Deserialize)]
struct KeyRow {
    content_key: String,
    #[serde(default)]
    cover_key: Option<String>,
}

#[worker::send]
pub async fn delete_post(
    Extension(env): Extension<SendWrapper<Env>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    let db = env.d1("DB").unwrap();
    let keys: Option<KeyRow> = db
        .prepare("SELECT content_key, cover_key FROM posts WHERE _id = ?1")
        .bind(&[id.into()])
        .unwrap()
        .first(None)
        .await
        .unwrap_or(None);

    if let Err(e) = db
        .prepare("DELETE FROM post_tags WHERE post_id = ?1")
        .bind(&[id.into()])
        .unwrap()
        .run()
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(format!("delete failed: {}", e)),
        )
            .into_response();
    }
    if let Err(e) = db
        .prepare("DELETE FROM posts WHERE _id = ?1")
        .bind(&[id.into()])
        .unwrap()
        .run()
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(format!("delete failed: {}", e)),
        )
            .into_response();
    }
    // 删库成功后再尽力删 R2(失败不阻断)
    if let Some(k) = keys {
        best_effort_delete(&env, k.content_key.trim()).await;
        if let Some(cover) = k.cover_key {
            if !cover.trim().is_empty() {
                best_effort_delete(&env, cover.trim()).await;
            }
        }
    }
    trigger_pages_rebuild(&env).await;
    (StatusCode::OK, Json("delete ok")).into_response()
}
