use axum::{extract::Path, http::StatusCode, response::IntoResponse, Extension, Json};
use worker::{send::SendWrapper, Env};

#[worker::send]
pub async fn search(
    Extension(env): Extension<SendWrapper<Env>>,
    Path(keyword): Path<String>,
) -> impl IntoResponse {
    let db = env.d1("DB").unwrap();

    // 正文在 R2,只搜标题+摘要
    let result = db
        .prepare(
            "SELECT _id, title, excerpt, date, category, content_key, cover_key, word_count, read_time FROM posts
             WHERE title LIKE '%' || ?1 || '%'
             OR excerpt LIKE '%' || ?1 || '%';",
        )
        .bind(&[keyword.into()])
        .unwrap()
        .all()
        .await;
    let result = match result {
        Ok(r) => r.results::<serde_json::Value>().unwrap_or_default(),
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("DB error: {}", e),
            )
                .into_response()
        }
    };

    (StatusCode::OK, Json(result)).into_response()
}
