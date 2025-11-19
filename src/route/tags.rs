use axum::{http::StatusCode, response::IntoResponse, Extension, Json};
use worker::{send::SendWrapper, Env};

#[worker::send]
pub async fn get_tags(Extension(env): Extension<SendWrapper<Env>>) -> impl IntoResponse {
    let db = env.d1("DB").unwrap();

    let query = db.prepare("SELECT * FROM tags").all().await.unwrap();

    let result = query.results::<serde_json::Value>().unwrap();

    (StatusCode::OK, Json(result)).into_response()
}

#[worker::send]
pub async fn add_tag(Extension(env): Extension<SendWrapper<Env>>) -> impl IntoResponse {
    let db = env.d1("DB").unwrap();

    let query = db.prepare("SELECT * FROM tags").all().await.unwrap();

    let result = query.results::<serde_json::Value>().unwrap();

    (StatusCode::OK, Json(result)).into_response()
}
