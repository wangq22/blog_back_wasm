use axum::routing::{delete, post, put};
use axum::{http, middleware};
use axum::{routing::get, Extension, Router};
use tower_service::Service;
use worker::send::SendWrapper;
use worker::{Context, Env, HttpRequest, Result};

use http::Method;
use tower_http::cors::{Any, CorsLayer};

use crate::midware::auth_middleware::auth_middleware;
use crate::route::archive::get_by_class;
use crate::route::category::{add_category, get_all_category};
use crate::route::post::{add_post, delete_post, get_all_posts, get_post_detail, update_post};
use crate::route::search::search;
use crate::route::tags::{add_tag, get_tags};
use crate::route::user::userinfo;

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
        .route("/category", get(get_all_category));

    let protected_routes = Router::new()
        .route("/post", post(add_post))
        .route("/post/{id}", delete(delete_post))
        .route("/post", put(update_post))
        .route("/category", post(add_category))
        .route("/tag", post(add_tag))
        .layer(middleware::from_fn(auth_middleware));

    Router::new()
        .nest("/api", public_routes)
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
