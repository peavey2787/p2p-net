//! Bounded JSON-over-HTTPS requests: `reqwest` on native targets, the browser
//! `fetch` API on wasm32. Every request has a timeout and a response size cap,
//! so a slow or hostile endpoint cannot stall or bloat the node.

use std::time::Duration;

use serde::de::DeserializeOwned;

/// Largest response body accepted from any JSON endpoint.
pub(crate) const MAX_JSON_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HttpMethod {
    Get,
    Post,
}

impl HttpMethod {
    fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

/// One JSON request. `url` already carries any query string.
#[derive(Debug, Clone)]
pub(crate) struct JsonRequest {
    pub(crate) method: HttpMethod,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Option<serde_json::Value>,
    pub(crate) timeout: Duration,
}

impl JsonRequest {
    pub(crate) fn get(url: impl Into<String>, timeout: Duration) -> Self {
        Self {
            method: HttpMethod::Get,
            url: url.into(),
            headers: Vec::new(),
            body: None,
            timeout,
        }
    }

    pub(crate) fn post(url: impl Into<String>, body: serde_json::Value, timeout: Duration) -> Self {
        Self {
            method: HttpMethod::Post,
            url: url.into(),
            headers: Vec::new(),
            body: Some(body),
            timeout,
        }
    }

    #[must_use]
    pub(crate) fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_string(), value.into()));
        self
    }

    fn has_header(&self, name: &str) -> bool {
        self.headers
            .iter()
            .any(|(header, _)| header.eq_ignore_ascii_case(name))
    }
}

/// Send `request` and decode a successful JSON response body.
pub(crate) async fn fetch_json<T: DeserializeOwned>(request: JsonRequest) -> Result<T, String> {
    let url = request.url.clone();
    let timeout = request.timeout;
    let body = crate::runtime::timeout(timeout, send(request))
        .await
        .map_err(|_| format!("request to {url} timed out"))??;
    if body.len() > MAX_JSON_RESPONSE_BYTES {
        return Err(format!(
            "response from {url} exceeded {MAX_JSON_RESPONSE_BYTES} bytes"
        ));
    }
    serde_json::from_slice(&body).map_err(|err| format!("invalid JSON from {url}: {err}"))
}

#[cfg(not(target_arch = "wasm32"))]
async fn send(request: JsonRequest) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .timeout(request.timeout)
        .build()
        .map_err(|err| err.to_string())?;
    let method = reqwest::Method::from_bytes(request.method.as_str().as_bytes())
        .map_err(|err| err.to_string())?;
    let mut builder = client.request(method, &request.url);
    if !request.has_header("accept") {
        builder = builder.header("accept", "application/json");
    }
    for (name, value) in &request.headers {
        builder = builder.header(name, value);
    }
    if let Some(body) = &request.body {
        builder = builder.json(body);
    }
    let response = builder.send().await.map_err(|err| err.to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("{} failed with HTTP {status}", request.url));
    }
    if response
        .content_length()
        .is_some_and(|len| len > MAX_JSON_RESPONSE_BYTES as u64)
    {
        return Err(format!(
            "response from {} exceeded {MAX_JSON_RESPONSE_BYTES} bytes",
            request.url
        ));
    }
    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|err| err.to_string())
}

#[cfg(target_arch = "wasm32")]
async fn send(request: JsonRequest) -> Result<Vec<u8>, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Request, RequestInit, RequestMode, Response};

    let init = RequestInit::new();
    init.set_method(request.method.as_str());
    init.set_mode(RequestMode::Cors);
    if let Some(body) = &request.body {
        init.set_body(&wasm_bindgen::JsValue::from_str(&body.to_string()));
    }
    let js_request =
        Request::new_with_str_and_init(&request.url, &init).map_err(js_error_string)?;
    let headers = js_request.headers();
    if !request.has_header("accept") {
        headers
            .set("accept", "application/json")
            .map_err(js_error_string)?;
    }
    if request.body.is_some() {
        headers
            .set("content-type", "application/json")
            .map_err(js_error_string)?;
    }
    for (name, value) in &request.headers {
        headers.set(name, value).map_err(js_error_string)?;
    }
    let window = web_sys::window().ok_or_else(|| "browser Window is unavailable".to_string())?;
    let response = JsFuture::from(window.fetch_with_request(&js_request))
        .await
        .map_err(js_error_string)?;
    let response: Response = response
        .dyn_into()
        .map_err(|_| "fetch returned a non-Response value".to_string())?;
    if !response.ok() {
        return Err(format!(
            "{} failed with HTTP {}",
            request.url,
            response.status()
        ));
    }
    let text = JsFuture::from(response.text().map_err(js_error_string)?)
        .await
        .map_err(js_error_string)?;
    Ok(text.as_string().unwrap_or_default().into_bytes())
}

#[cfg(target_arch = "wasm32")]
fn js_error_string(value: wasm_bindgen::JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| format!("browser JavaScript error: {value:?}"))
}
