mod account;
mod communities;
mod dms;
mod messages;

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::header::{HOST, ORIGIN};
use axum::http::{HeaderMap, HeaderValue, Method, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde_json::json;

use crate::error::ApiError;
use crate::state::AppState;
use crate::{web, ws};

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(|| async { Json(json!({ "ok": true })) }))
        .route("/auth/register", post(account::register))
        .route("/auth/login", post(account::login))
        .route("/auth/logout", post(account::logout))
        .route("/me", get(account::me).patch(account::update_me))
        .route("/communities", get(communities::list).post(communities::create))
        .route("/communities/discover", get(communities::discover))
        .route(
            "/communities/{id}",
            patch(communities::rename).delete(communities::delete),
        )
        .route("/communities/{id}/join", post(communities::join))
        .route("/communities/{id}/leave", post(communities::leave))
        .route("/communities/{id}/members", get(communities::members))
        .route("/communities/{id}/channels", post(communities::create_channel))
        .route(
            "/channels/{id}",
            patch(communities::update_channel).delete(communities::delete_channel),
        )
        .route(
            "/channels/{id}/messages",
            get(messages::list_channel).post(messages::send_channel),
        )
        .route("/messages/{id}", patch(messages::edit).delete(messages::delete))
        .route("/dms", get(dms::list).post(dms::open))
        .route("/dms/{id}/messages", get(dms::list_messages).post(dms::send))
        .fallback(|| async { ApiError::not_found("No such API route.") })
        .layer(middleware::from_fn_with_state(state.clone(), require_same_origin))
        .layer(DefaultBodyLimit::max(64 * 1024));

    Router::new()
        .nest("/api", api)
        .route("/ws", get(ws::handler))
        .fallback(web::serve)
        .layer(middleware::map_response(security_headers))
        .layer(tower_http::compression::CompressionLayer::new())
        .with_state(state)
}

/// True when a browser request comes from this site rather than another one.
/// Requests without an Origin header (curl, scripts) carry no cookies from a
/// victim's browser, so they're allowed.
pub fn origin_allowed(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    let origin = origin.trim_end_matches('/');
    let origin_host = origin.split_once("://").map(|(_, rest)| rest).unwrap_or("");
    let header_is = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|host| host.eq_ignore_ascii_case(origin_host))
    };
    header_is(HOST.as_str())
        || header_is("x-forwarded-host")
        || state.cfg.public_origins.iter().any(|o| o.eq_ignore_ascii_case(origin))
}

/// Blocks cross-site requests that try to change data with the user's cookie.
async fn require_same_origin(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let safe = matches!(*request.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if !safe && !origin_allowed(&state, request.headers()) {
        return ApiError::forbidden("Requests from other sites are not allowed.").into_response();
    }
    next.run(request).await
}

async fn security_headers(mut response: Response) -> Response {
    let headers = response.headers_mut();
    let set = |headers: &mut HeaderMap, name: header::HeaderName, value: &'static str| {
        headers.entry(name).or_insert(HeaderValue::from_static(value));
    };
    set(headers, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(headers, header::REFERRER_POLICY, "same-origin");
    set(headers, header::X_FRAME_OPTIONS, "DENY");
    set(
        headers,
        header::CONTENT_SECURITY_POLICY,
        "default-src 'self'; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; \
         style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' ws: wss:; \
         frame-ancestors 'none'; base-uri 'self'; form-action 'self'",
    );
    response
}
