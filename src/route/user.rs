use axum::{http::StatusCode, response::IntoResponse, Extension, Json};
use serde_json::json;
use worker::{send::SendWrapper, Env};

use crate::model::user::{UserRow, UserUpdateDTO};
use crate::route::post::trigger_pages_rebuild;

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

/// 更新站长资料(需 Access 登录):`PUT /api/protected/user`,body 只带要改的字段即可。
/// 前端改资料后静态页需重建才能生效,此处复用文章接口的 Pages 重建钩子。
#[worker::send]
pub async fn update_user(
    Extension(env): Extension<SendWrapper<Env>>,
    Json(payload): Json<UserUpdateDTO>,
) -> impl IntoResponse {
    // 轻量校验:非空邮箱必须含 @。
    if let Some(email) = payload.email.as_ref() {
        let email = email.trim();
        if !email.is_empty() && (!email.contains('@') || email.len() > 254) {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "invalid email" })),
            )
                .into_response();
        }
    }

    let db = env.d1("DB").unwrap();
    let existing: Option<UserRow> = match db.prepare("SELECT * FROM user LIMIT 1").first(None).await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("DB error: {}", e) })),
            )
                .into_response()
        }
    };

    // 缺省字段沿用旧值(全 String 绑定,避开 D1 对 NULL 参数的绑定差异)。
    let clean = |v: Option<String>| v.map(|s| s.trim().to_string());
    let old = |v: Option<String>| v.unwrap_or_default();
    let (existing_id, e_name, e_bio, e_avatar, e_github, e_bilibili, e_tz, e_city, e_email, e_aff) =
        match existing {
            Some(r) => (
                Some(r._id),
                r.name,
                old(r.bio),
                old(r.avatar_url),
                old(r.github_url),
                old(r.bilibili_url),
                old(r.timezone),
                old(r.city),
                old(r.email),
                old(r.affiliation),
            ),
            None => (
                None,
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ),
        };
    let name = clean(payload.name).unwrap_or(e_name);
    let bio = clean(payload.bio).unwrap_or(e_bio);
    let avatar_url = clean(payload.avatar_url).unwrap_or(e_avatar);
    let github_url = clean(payload.github_url).unwrap_or(e_github);
    let bilibili_url = clean(payload.bilibili_url).unwrap_or(e_bilibili);
    let timezone = clean(payload.timezone).unwrap_or(e_tz);
    let city = clean(payload.city).unwrap_or(e_city);
    let email = clean(payload.email).unwrap_or(e_email);
    let affiliation = clean(payload.affiliation).unwrap_or(e_aff);

    if name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "name is required" })),
        )
            .into_response();
    }

    // 单用户表:有行则按 _id 更新,无行则首插一行(_id 固定 1)。
    let db_result = if let Some(user_id) = existing_id {
        db.prepare(
            "UPDATE user SET name = ?1, bio = ?2, avatar_url = ?3, github_url = ?4,
                 bilibili_url = ?5, timezone = ?6, city = ?7, email = ?8, affiliation = ?9
             WHERE _id = ?10",
        )
        .bind(&[
            name.into(),
            bio.into(),
            avatar_url.into(),
            github_url.into(),
            bilibili_url.into(),
            timezone.into(),
            city.into(),
            email.into(),
            affiliation.into(),
            user_id.into(),
        ])
        .unwrap()
        .run()
        .await
    } else {
        db.prepare(
            "INSERT INTO user (_id, name, bio, avatar_url, github_url, bilibili_url,
                 timezone, city, email, affiliation)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .bind(&[
            name.into(),
            bio.into(),
            avatar_url.into(),
            github_url.into(),
            bilibili_url.into(),
            timezone.into(),
            city.into(),
            email.into(),
            affiliation.into(),
        ])
        .unwrap()
        .run()
        .await
    };
    if let Err(e) = db_result {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("DB error: {}", e) })),
        )
            .into_response();
    }

    trigger_pages_rebuild(&env).await;
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}
