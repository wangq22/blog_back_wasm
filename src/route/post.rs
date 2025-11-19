use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::format_description::well_known::Rfc3339;
use worker::{send::SendWrapper, Env};

use crate::model::post::{PostDetailDTO, PostInsertDTO};

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

#[worker::send]
pub async fn get_all_posts(
    Extension(env): Extension<SendWrapper<Env>>,
    Query(param): Query<Page>,
) -> impl IntoResponse {
    let db = env.d1("DB").unwrap();
    let query = db.prepare(
        "SELECT p._id, p.title, p.excerpt, p.category, p.date, p.cover_image, p.word_count, p.read_time,
        json_group_array(t.name) AS tags
        FROM posts AS p
        LEFT JOIN post_tags AS pt ON p._id = pt.post_id
        LEFT JOIN tags AS t ON pt.tag_id = t._id
        GROUP BY p._id
        LIMIT ?2 OFFSET ?1",
    );
    let offset = (param.page.saturating_sub(1) * param.page_size) as i32;

    let result = query
        .bind(&[offset.into(), param.page_size.into()])
        .unwrap()
        .all()
        .await
        .unwrap();

    let rows = result.results::<serde_json::Value>().unwrap();

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

#[worker::send]
pub async fn add_post(
    Extension(env): Extension<SendWrapper<Env>>,
    Json(post): Json<PostInsertDTO>,
) -> impl IntoResponse {
    let db = env.d1("DB").unwrap();

    let post_result = db
        .prepare("INSERT INTO posts (title, excerpt, date, category, content, cover_image, word_count, read_time)
                  VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)")
        .bind(&[
            post.title.clone().into(),
            post.excerpt.clone().into(),
            post.date.format(&Rfc3339).unwrap().into(),
            post.category.clone().into(),
            post.content.clone().into(),
            post.cover_image.clone().into(),
            post.word_count.into(),
            post.read_time.into(),
        ])
        .unwrap()
        .run()
        .await
        .unwrap();

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
    let db = env.d1("DB").unwrap();

    let update_result = db
        .prepare(
            "UPDATE posts 
             SET title = ?1, excerpt = ?2, date = ?3, 
                 category = ?4, content = ?5, cover_image = ?6, 
                 word_count = ?7, read_time = ?8
             WHERE _id = ?9",
        )
        .bind(&[
            post.title.into(),
            post.excerpt.into(),
            post.date.format(&Rfc3339).unwrap().into(),
            post.category.into(),
            post.content.into(),
            post.cover_image.into(),
            post.word_count.into(),
            post.read_time.into(),
            post._id.into(),
        ])
        .unwrap()
        .run()
        .await;

    if let Err(err) = update_result {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(format!("DB error: {}", err)),
        )
            .into_response();
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

#[worker::send]
pub async fn delete_post(
    Extension(env): Extension<SendWrapper<Env>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    let db = env.d1("DB").unwrap();
    let delete_result = db
        .prepare("DELETE FROM posts where _id = ?1")
        .bind(&[id.into()])
        .unwrap()
        .run()
        .await;

    match delete_result {
        Ok(_) => (StatusCode::OK, Json("delete ok")).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(format!("delete failed: {}", err)),
        )
            .into_response(),
    }
}
