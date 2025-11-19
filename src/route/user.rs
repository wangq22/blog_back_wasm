use axum::{http::StatusCode, response::IntoResponse, Extension, Json};
use worker::{send::SendWrapper, Env};

#[worker::send]
pub async fn userinfo(Extension(env): Extension<SendWrapper<Env>>) -> impl IntoResponse {
    let db = env.d1("DB").unwrap();
    let query = db.prepare("SELECT * FROM user");
    let first_row: Option<serde_json::Value> = query.first(None).await.unwrap();

    match first_row {
        Some(user) => (StatusCode::OK, Json(user)).into_response(),
        None => (StatusCode::NOT_FOUND, "No user found").into_response(),
    }
}
