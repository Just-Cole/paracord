//! Small wrapper over fetch. The session lives in an HttpOnly cookie, so
//! requests carry it automatically and the page never sees the token.

use gloo_net::http::{Request, RequestBuilder, Response};
use paracord_shared::ErrorBody;
use serde::Serialize;
use serde::de::DeserializeOwned;

#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}

impl ApiError {
    pub fn unauthorized(&self) -> bool {
        self.status == 401
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

fn network(error: impl std::fmt::Display) -> ApiError {
    ApiError {
        status: 0,
        message: format!("Couldn't reach Paracord ({error})."),
    }
}

fn url(path: &str) -> String {
    format!("/api{path}")
}

async fn check(response: Response) -> ApiResult<Response> {
    if response.ok() {
        return Ok(response);
    }
    let status = response.status();
    let message = match response.json::<ErrorBody>().await {
        Ok(body) => body.error,
        Err(_) => format!("Request failed ({status})."),
    };
    Err(ApiError { status, message })
}

async fn run(builder: RequestBuilder, body: Option<String>) -> ApiResult<Response> {
    let request = match body {
        Some(json) => builder
            .header("Content-Type", "application/json")
            .body(json)
            .map_err(network)?,
        None => builder.build().map_err(network)?,
    };
    check(request.send().await.map_err(network)?).await
}

fn encode<B: Serialize>(body: &B) -> ApiResult<Option<String>> {
    serde_json::to_string(body).map(Some).map_err(network)
}

pub async fn get<T: DeserializeOwned>(path: &str) -> ApiResult<T> {
    let response = run(Request::get(&url(path)), None).await?;
    response.json().await.map_err(network)
}

pub async fn post<B: Serialize, T: DeserializeOwned>(path: &str, body: &B) -> ApiResult<T> {
    let response = run(Request::post(&url(path)), encode(body)?).await?;
    response.json().await.map_err(network)
}

pub async fn patch<B: Serialize, T: DeserializeOwned>(path: &str, body: &B) -> ApiResult<T> {
    let response = run(Request::patch(&url(path)), encode(body)?).await?;
    response.json().await.map_err(network)
}

/// For routes that answer "204 No Content".
pub async fn post_empty(path: &str) -> ApiResult<()> {
    run(Request::post(&url(path)), Some("{}".into())).await.map(|_| ())
}

pub async fn patch_empty<B: Serialize>(path: &str, body: &B) -> ApiResult<()> {
    run(Request::patch(&url(path)), encode(body)?).await.map(|_| ())
}

pub async fn delete(path: &str) -> ApiResult<()> {
    run(Request::delete(&url(path)), None).await.map(|_| ())
}
