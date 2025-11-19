use axum::{extract::Query, http::StatusCode, response::IntoResponse, Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use worker::{send::SendWrapper, D1Database, Env, Error};

#[derive(Deserialize, Serialize)]
pub struct ClassParam {
    tag: Option<String>,
    category: Option<String>,
}

#[worker::send]
pub async fn get_by_class(
    Extension(env): Extension<SendWrapper<Env>>,
    Query(param): Query<ClassParam>,
) -> impl IntoResponse {
    let db = env.d1("DB").unwrap();

    let result = match (param.tag, param.category) {
        (Some(tag), _) => get_by_tag(db, tag).await,
        (None, Some(category)) => get_by_category(db, category).await,
        (None, None) => get_all(db).await,
    };

    match result {
        Ok(rows) if !rows.is_empty() => (StatusCode::OK, Json(rows)).into_response(),
        Ok(_) => (StatusCode::NOT_FOUND, "No records found").into_response(),
        Err(_e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Something went wrong (っ╥╯﹏╰╥c) "),
        )
            .into_response(),
    }
}

#[allow(dead_code)]
pub async fn get_by_tag(db: D1Database, tag: String) -> Result<Vec<Value>, Error> {
    let query = db.prepare(
        "SELECT DISTINCT p.*
            FROM posts AS p
            JOIN post_tags AS pt ON p._id = pt.post_id
            JOIN tags AS t ON pt.tag_id = t._id
            WHERE t.name = ?1;
        ",
    );

    let result = query
        .bind(&[tag.into()])
        .unwrap()
        .all()
        .await
        .unwrap()
        .results::<serde_json::Value>();

    result
}

pub async fn get_by_category(db: D1Database, category: String) -> Result<Vec<Value>, Error> {
    let query = db.prepare("SELECT * FROM posts WHERE category=?1;");

    let result = query
        .bind(&[category.into()])
        .unwrap()
        .all()
        .await
        .unwrap()
        .results::<serde_json::Value>();

    result
}

pub async fn get_all(db: D1Database) -> Result<Vec<Value>, Error> {
    let query = db.prepare("SELECT * FROM posts;");
    let result = query.all().await.unwrap().results::<serde_json::Value>();

    result
}
