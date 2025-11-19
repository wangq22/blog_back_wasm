use axum::{extract::Path, http::StatusCode, response::IntoResponse, Extension, Json};
use worker::{send::SendWrapper, Env};

#[worker::send]
pub async fn search(
    Extension(env): Extension<SendWrapper<Env>>,
    Path(keyword): Path<String>,
) -> impl IntoResponse {
    let db = env.d1("DB").unwrap();

    let query = db.prepare(
        "SELECT * FROM posts
        WHERE content LIKE '%' || ?1 || '%'
        OR title LIKE '%' || ?1 || '%';",
    );

    let result = query
        .bind(&[keyword.into()])
        .unwrap()
        .all()
        .await
        .unwrap()
        .results::<serde_json::Value>()
        .unwrap();

    (StatusCode::OK, Json(result)).into_response()
}
