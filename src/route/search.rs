use axum::{extract::Path, http::StatusCode, response::IntoResponse, Extension, Json};
use worker::{send::SendWrapper, Env};

#[worker::send]
pub async fn search(
    Extension(env): Extension<SendWrapper<Env>>,
    Path(keyword): Path<String>,
) -> impl IntoResponse {
    let db = env.d1("DB").unwrap();

    // 正文已搬 R2,D1 不再 LIKE content,只搜标题+摘要
    let keyword2 = keyword.clone();
    let first = db
        .prepare(
            "SELECT _id, title, excerpt, date, category, cover_image, content_key, cover_key, word_count, read_time FROM posts
             WHERE title LIKE '%' || ?1 || '%'
             OR excerpt LIKE '%' || ?1 || '%';",
        )
        .bind(&[keyword.into()])
        .unwrap()
        .all()
        .await;
    let result = match first {
        Ok(r) => r.results::<serde_json::Value>().unwrap_or_default(),
        Err(e) if e.to_string().contains("no such column") => db
            .prepare(
                "SELECT _id, title, excerpt, date, category, cover_image, word_count, read_time FROM posts
                 WHERE title LIKE '%' || ?1 || '%'
                 OR excerpt LIKE '%' || ?1 || '%';",
            )
            .bind(&[keyword2.into()])
            .unwrap()
            .all()
            .await
            .unwrap()
            .results::<serde_json::Value>()
            .unwrap_or_default(),
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
