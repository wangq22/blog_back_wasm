use axum::{http::StatusCode, response::IntoResponse, Extension, Json};
use worker::{send::SendWrapper, Env};

#[worker::send]
pub async fn get_all_category(Extension(env): Extension<SendWrapper<Env>>) -> impl IntoResponse {
    let db = env.d1("DB").unwrap();

    let query = db
        .prepare("SELECT name, post_count FROM category")
        .all()
        .await
        .unwrap();

    let result = query.results::<serde_json::Value>().unwrap();
    (StatusCode::OK, Json(result))
}

#[worker::send]
pub async fn add_category(Extension(env): Extension<SendWrapper<Env>>) -> impl IntoResponse {
    let db = env.d1("DB").unwrap();

    let query = db
        .prepare("SELECT name, post_count FROM category")
        .all()
        .await
        .unwrap();

    let result = query.results::<serde_json::Value>().unwrap();
    (StatusCode::OK, Json(result))
}
