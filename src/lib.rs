use axum::routing::{delete, post, put};
use axum::{http, middleware};
use axum::{routing::get, Extension, Router};
use tower_service::Service;
use worker::send::SendWrapper;
use worker::{Context, Env, HttpRequest, Result};

use http::Method;
use tower_http::cors::{Any, CorsLayer};

use std::sync::Arc;

use crate::midware::auth_middleware::{auth_middleware, CfAccessConfig};
use crate::route::archive::get_by_class;
use crate::route::auth::{exchange, refresh, AuthState};
use crate::route::category::{add_category, get_all_category};
use crate::route::media::{delete_media, get_media, upload_media};
use crate::route::post::{add_post, delete_post, get_all_posts, get_post_detail, update_post};
use crate::route::search::search;
use crate::route::tags::{add_tag, get_tags};
use crate::route::user::{update_user, userinfo};

mod midware;
mod model;
mod route;

pub fn router(env: Env) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers(Any);

    let public_routes = Router::new()
        .route("/user", get(userinfo))
        .route("/posts", get(get_all_posts))
        .route("/tags", get(get_tags))
        .route("/post/{id}", get(get_post_detail))
        .route("/archive", get(get_by_class))
        .route("/search/{keyword}", get(search))
        .route("/category", get(get_all_category))
        // R2 公开读(无需登录):/api/media/covers/xxx /api/media/posts/yyy.md
        .route("/media/{*key}", get(get_media));

    let cf_cfg = Arc::new(
        CfAccessConfig::from_env(&env)
            .expect("Missing Cloudflare Access env: set CF_ACCESS_TEAM_DOMAIN + CF_ACCESS_CLIENT_ID"),
    );

    // BFF 登录代换(公开接口):client secret 只在 Worker 侧出现,通过 wrangler secret 注入
    let auth_state = AuthState {
        cfg: Arc::clone(&cf_cfg),
        client_secret: env
            .var("CF_ACCESS_CLIENT_SECRET")
            .ok()
            .map(|v| v.to_string()),
    };
    let auth_routes = Router::new()
        .route("/auth/exchange", post(exchange))
        .route("/auth/refresh", post(refresh))
        .with_state(auth_state);

    let protected_routes = Router::new()
        .route("/post", post(add_post))
        .route("/post/{id}", delete(delete_post))
        .route("/post", put(update_post))
        .route("/category", post(add_category))
        .route("/tag", post(add_tag))
        // 站长资料更新(资料页预渲染,成功后同样触发 Pages 重建)
        .route("/user", put(update_user))
        // R2 中转上传/删除(需 Access 登录)
        .route("/media", post(upload_media))
        .route("/media", delete(delete_media))
        .layer(middleware::from_fn_with_state(cf_cfg, auth_middleware));

    Router::new()
        .nest("/api", public_routes)
        .nest("/api", auth_routes)
        .nest("/api/protected", protected_routes)
        .layer(Extension(SendWrapper::new(env)))
        .layer(cors)
}

#[worker::event(fetch)]
pub async fn fetch(
    req: HttpRequest,
    env: Env,
    _ctx: Context,
) -> Result<axum::http::Response<axum::body::Body>> {
    Ok(router(env).call(req).await?)
}
