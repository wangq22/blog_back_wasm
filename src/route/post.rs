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

use crate::model::post::{PostDetailDTO, PostInsertDTO};
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

fn media_url(key: &str) -> String {
    format!("/api/media/{}", key)
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

/// 列表不需要正文(R2 不读),只把 cover_key 折成 cover_image 方便新老前端。
fn resolve_list_row(mut row: Value) -> Value {
    if let Some(key) = row.get("cover_key").and_then(|v| v.as_str()) {
        let key = key.trim();
        if !key.is_empty() {
            let need = row
                .get("cover_image")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().is_empty())
                .unwrap_or(true);
            if need {
                row["cover_image"] = Value::String(media_url(key));
            }
        }
    }
    row
}

#[worker::send]
pub async fn get_all_posts(
    Extension(env): Extension<SendWrapper<Env>>,
    Query(param): Query<Page>,
) -> impl IntoResponse {
    let db = env.d1("DB").unwrap();
    let offset = (param.page.saturating_sub(1) * param.page_size) as i32;

    // 新列优先;库还没迁移(no such column)时回退到老查询,保证发版窗口不 500
    let rows: Vec<Value> = async {
        let q = db.prepare(
            "SELECT p._id, p.title, p.excerpt, p.category, p.date, p.cover_image,
             p.content_key, p.cover_key, p.word_count, p.read_time,
             json_group_array(t.name) AS tags
             FROM posts AS p
             LEFT JOIN post_tags AS pt ON p._id = pt.post_id
             LEFT JOIN tags AS t ON pt.tag_id = t._id
             GROUP BY p._id
             LIMIT ?2 OFFSET ?1",
        );
        match q
            .bind(&[offset.into(), param.page_size.into()])
            .unwrap()
            .all()
            .await
        {
            Ok(r) => Ok(r.results::<Value>().unwrap_or_default()),
            Err(e) => {
                let msg = e.to_string();
                if !msg.contains("no such column") {
                    return Err(msg);
                }
                let q2 = db.prepare(
                    "SELECT p._id, p.title, p.excerpt, p.category, p.date, p.cover_image,
                     p.word_count, p.read_time,
                     json_group_array(t.name) AS tags
                     FROM posts AS p
                     LEFT JOIN post_tags AS pt ON p._id = pt.post_id
                     LEFT JOIN tags AS t ON pt.tag_id = t._id
                     GROUP BY p._id
                     LIMIT ?2 OFFSET ?1",
                );
                let r2 = q2
                    .bind(&[offset.into(), param.page_size.into()])
                    .unwrap()
                    .all()
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(r2.results::<Value>().unwrap_or_default())
            }
        }
    }
    .await
    .unwrap_or_default();

    let rows: Vec<_> = rows
        .into_iter()
        .map(|mut row| {
            if let Some(tags_str) = row.get("tags").and_then(|v| v.as_str()) {
                if let Ok(tags_json) = serde_json::from_str::<serde_json::Value>(tags_str) {
                    row["tags"] = tags_json;
                }
            }
            resolve_list_row(row)
        })
        .collect();

    let response = PageResponse {
        data: Some(rows),
        meta: param.clone(),
    };

    (StatusCode::OK, Json(response))
}
#[worker::send]
pub async fn get_post_detail(
    Extension(env): Extension<SendWrapper<Env>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    let db = env.d1("DB").unwrap();

    let post_query = db
        .prepare("SELECT * FROM posts WHERE _id = ?1")
        .bind(&[id.into()])
        .unwrap()
        .first::<PostDetailDTO>(None)
        .await;

    let mut post = match post_query {
        Ok(Some(row)) => row,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, "Post not found").into_response();
        }
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("DB error: {}", err),
            )
                .into_response();
        }
    };

    // R2 优先,老数据回退 D1 content
    if let Some(key) = post.content_key.clone() {
        let key = key.trim().to_string();
        if !key.is_empty() {
            if !check_key(&key) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Invalid content_key: {}", key),
                )
                    .into_response();
            }
            match r2_text(&env, &key).await {
                Ok(text) => post.content = text,
                Err(e) => {
                    return (
                        StatusCode::BAD_GATEWAY,
                        format!("Read R2 markdown failed ({}): {}", key, e),
                    )
                        .into_response()
                }
            }
        }
    }
    if let Some(key) = post.cover_key.clone() {
        let key = key.trim().to_string();
        if !key.is_empty() && post.cover_image.trim().is_empty() {
            post.cover_image = media_url(&key);
        }
    }

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

    post.tags = Some(tag_list);

    (StatusCode::OK, Json(post)).into_response()
}

#[derive(Debug, Deserialize)]
pub struct TagNameRow {
    pub name: String,
}

fn normalize_keys(post: &PostInsertDTO) -> Result<(String, String, String, String), (StatusCode, String)> {
    let content_key = post.content_key.clone().unwrap_or_default().trim().to_string();
    let cover_key = post.cover_key.clone().unwrap_or_default().trim().to_string();
    if !content_key.is_empty() && !check_key(&content_key) {
        return Err((StatusCode::BAD_REQUEST, "invalid content_key".to_string()));
    }
    if !cover_key.is_empty() && !check_key(&cover_key) {
        return Err((StatusCode::BAD_REQUEST, "invalid cover_key".to_string()));
    }
    // 有 key 就不再往 D1 写全文(省空间);没 key 则兼容老客户端直传
    let content = if content_key.is_empty() {
        post.content.clone()
    } else {
        String::new()
    };
    let cover_image = if cover_key.is_empty() {
        post.cover_image.clone()
    } else {
        String::new()
    };
    Ok((content, cover_image, content_key, cover_key))
}

#[worker::send]
pub async fn add_post(
    Extension(env): Extension<SendWrapper<Env>>,
    Json(post): Json<PostInsertDTO>,
) -> impl IntoResponse {
    let (content, cover_image, content_key, cover_key) = match normalize_keys(&post) {
        Ok(v) => v,
        Err((c, m)) => return (c, Json(json!({ "error": m }))).into_response(),
    };
    let db = env.d1("DB").unwrap();

    let date = post.date.format(&Rfc3339).unwrap();
    let run_new = db
        .prepare("INSERT INTO posts (title, excerpt, date, category, content, cover_image, content_key, cover_key, word_count, read_time)
                  VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)")
        .bind(&[
            post.title.clone().into(),
            post.excerpt.clone().into(),
            date.clone().into(),
            post.category.clone().into(),
            content.clone().into(),
            cover_image.clone().into(),
            content_key.clone().into(),
            cover_key.clone().into(),
            post.word_count.into(),
            post.read_time.into(),
        ])
        .unwrap()
        .run()
        .await;
    let post_result = match run_new {
        Ok(r) => r,
        Err(e) if e.to_string().contains("no such column") => {
            // 未迁移的库:回退老写法
            match db
                .prepare("INSERT INTO posts (title, excerpt, date, category, content, cover_image, word_count, read_time)
                          VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)")
                .bind(&[
                    post.title.clone().into(),
                    post.excerpt.clone().into(),
                    date.into(),
                    post.category.clone().into(),
                    post.content.clone().into(),
                    post.cover_image.clone().into(),
                    post.word_count.into(),
                    post.read_time.into(),
                ])
                .unwrap()
                .run()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": format!("DB error: {}", e) })),
                    )
                        .into_response()
                }
            }
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("DB error: {}", e) })),
            )
                .into_response()
        }
    };

    let meta = post_result.meta().unwrap().ok_or("Missing meta").unwrap();
    let post_id = meta.last_row_id.ok_or("Missing last_row_id").unwrap();

    if let Some(tag_list) = &post.tags {
        for tag_name in tag_list {
            let existing = db
                .prepare("SELECT _id FROM tags WHERE name = ?1")
                .bind(&[tag_name.into()])
                .unwrap()
                .first::<TagRow>(None)
                .await;

            let tag_id = match existing {
                Ok(Some(row)) => row._id,
                _ => {
                    let insert_tag_res = db
                        .prepare("INSERT INTO tags (name, color_class) VALUES (?1, 'info_badge')")
                        .bind(&[tag_name.into()])
                        .unwrap()
                        .run()
                        .await
                        .unwrap();

                    insert_tag_res
                        .meta()
                        .unwrap()
                        .unwrap()
                        .last_row_id
                        .unwrap()
                        .try_into()
                        .unwrap()
                }
            };

            db.prepare("INSERT INTO post_tags (post_id, tag_id) VALUES (?1, ?2)")
                .bind(&[(post_id as i32).into(), (tag_id as i32).into()])
                .unwrap()
                .run()
                .await
                .unwrap();
        }
    }

    (StatusCode::OK, Json(json!({ "id": post_id }))).into_response()
}

#[worker::send]
pub async fn update_post(
    Extension(env): Extension<SendWrapper<Env>>,
    Json(post): Json<PostDetailDTO>,
) -> impl IntoResponse {
    // 复用同样的 key 归一逻辑
    let as_insert = PostInsertDTO {
        title: post.title.clone(),
        excerpt: post.excerpt.clone(),
        date: post.date,
        category: post.category.clone(),
        content: post.content.clone(),
        cover_image: post.cover_image.clone(),
        content_key: post.content_key.clone(),
        cover_key: post.cover_key.clone(),
        tags: post.tags.clone(),
        word_count: post.word_count,
        read_time: post.read_time,
    };
    let (content, cover_image, content_key, cover_key) = match normalize_keys(&as_insert) {
        Ok(v) => v,
        Err((c, m)) => return (c, Json(json!({ "error": m }))).into_response(),
    };
    let db = env.d1("DB").unwrap();

    let date = post.date.format(&Rfc3339).unwrap();
    let update_result = db
        .prepare(
            "UPDATE posts
             SET title = ?1, excerpt = ?2, date = ?3,
                 category = ?4, content = ?5, cover_image = ?6,
                 content_key = ?7, cover_key = ?8,
                 word_count = ?9, read_time = ?10
             WHERE _id = ?11",
        )
        .bind(&[
            post.title.clone().into(),
            post.excerpt.clone().into(),
            date.clone().into(),
            post.category.clone().into(),
            content.into(),
            cover_image.into(),
            content_key.into(),
            cover_key.into(),
            post.word_count.into(),
            post.read_time.into(),
            post._id.into(),
        ])
        .unwrap()
        .run()
        .await;

    if let Err(err) = update_result {
        if err.to_string().contains("no such column") {
            let legacy = db
                .prepare(
                    "UPDATE posts
                     SET title = ?1, excerpt = ?2, date = ?3,
                         category = ?4, content = ?5, cover_image = ?6,
                         word_count = ?7, read_time = ?8
                     WHERE _id = ?9",
                )
                .bind(&[
                    post.title.clone().into(),
                    post.excerpt.clone().into(),
                    date.into(),
                    post.category.clone().into(),
                    post.content.clone().into(),
                    post.cover_image.clone().into(),
                    post.word_count.into(),
                    post.read_time.into(),
                    post._id.into(),
                ])
                .unwrap()
                .run()
                .await;
            if let Err(err) = legacy {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(format!("DB error: {}", err)),
                )
                    .into_response();
            }
        } else {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(format!("DB error: {}", err)),
            )
                .into_response();
        }
    }

    db.prepare("DELETE FROM post_tags WHERE post_id = ?1")
        .bind(&[post._id.into()])
        .unwrap()
        .run()
        .await
        .unwrap();

    if let Some(tag_list) = &post.tags {
        for tag_name in tag_list {
            let existing = db
                .prepare("SELECT _id FROM tags WHERE name = ?1")
                .bind(&[tag_name.into()])
                .unwrap()
                .first::<TagRow>(None)
                .await;

            let tag_id = match existing {
                Ok(Some(row)) => row._id,
                _ => {
                    let insert_tag_res = db
                        .prepare("INSERT INTO tags (name, color_class) VALUES (?1, 'info_badge')")
                        .bind(&[tag_name.into()])
                        .unwrap()
                        .run()
                        .await
                        .unwrap();

                    insert_tag_res
                        .meta()
                        .unwrap()
                        .unwrap()
                        .last_row_id
                        .unwrap()
                        .try_into()
                        .unwrap()
                }
            };

            db.prepare("INSERT INTO post_tags (post_id, tag_id) VALUES (?1, ?2)")
                .bind(&[post._id.into(), tag_id.into()])
                .unwrap()
                .run()
                .await
                .unwrap();
        }
    }

    (StatusCode::OK, Json("update ok")).into_response()
}

#[derive(Debug, Deserialize)]
pub struct TagRow {
    pub _id: i32,
}

#[derive(Debug, Deserialize)]
struct KeyRow {
    #[serde(default)]
    content_key: Option<String>,
    #[serde(default)]
    cover_key: Option<String>,
}

#[worker::send]
pub async fn delete_post(
    Extension(env): Extension<SendWrapper<Env>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    let db = env.d1("DB").unwrap();
    // 先读 key,删库后尽力删 R2(删 R2 失败不阻断)
    // 未迁移的库这里会查不到列,直接当无 key 处理
    let keys: Option<KeyRow> = db
        .prepare("SELECT content_key, cover_key FROM posts WHERE _id = ?1")
        .bind(&[id.into()])
        .unwrap()
        .first(None)
        .await
        .unwrap_or(None);
    // First delete related tag mappings to satisfy foreign key constraint
    let delete_tags_result = db
        .prepare("DELETE FROM post_tags WHERE post_id = ?1")
        .bind(&[id.into()])
        .unwrap()
        .run()
        .await;

    if let Err(err) = delete_tags_result {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(format!("delete failed: {}", err)),
        )
            .into_response();
    }

    let delete_post_result = db
        .prepare("DELETE FROM posts WHERE _id = ?1")
        .bind(&[id.into()])
        .unwrap()
        .run()
        .await;

    match delete_post_result {
        Ok(_) => {
            if let Some(k) = keys.as_ref().and_then(|k| k.content_key.clone()) {
                if !k.trim().is_empty() {
                    best_effort_delete(&env, k.trim()).await;
                }
            }
            if let Some(k) = keys.as_ref().and_then(|k| k.cover_key.clone()) {
                if !k.trim().is_empty() {
                    best_effort_delete(&env, k.trim()).await;
                }
            }
            (StatusCode::OK, Json("delete ok")).into_response()
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(format!("delete failed: {}", err)),
        )
            .into_response(),
    }
}
