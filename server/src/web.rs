//! Serves the web client. In release builds the files in web/dist are baked
//! into the binary; in debug builds they're read from disk on each request,
//! so `trunk watch` changes show up on reload.

use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../web/dist/"]
struct Assets;

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    if let Some(file) = Assets::get(path) {
        // Trunk puts a content hash in asset names, so they can be cached for good.
        let cache = if path == "index.html" {
            "no-cache"
        } else {
            "public, max-age=31536000, immutable"
        };
        return file_response(path, file.data.into_owned(), cache);
    }

    // Any other path is a page inside the app; let the client handle it.
    let looks_like_file = path != "index.html"
        && path.rsplit('/').next().is_some_and(|last| last.contains('.'));
    if !looks_like_file && let Some(index) = Assets::get("index.html") {
        return file_response("index.html", index.data.into_owned(), "no-cache");
    }
    if looks_like_file {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "The Paracord web client isn't built yet. Run `trunk build --release` in the web folder, then rebuild the server.",
    )
        .into_response()
}

fn file_response(path: &str, body: Vec<u8>, cache: &'static str) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let mut response = body.into_response();
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(mime.as_ref()) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}
