//! Bounded DNS-over-HTTPS TXT lookups for `/dnsaddr` resolution: `reqwest` on
//! native targets, the browser `fetch` API on wasm32.

use serde::Deserialize;

use super::{
    decode_dnsaddr_txt_value, DnsaddrConfig, DNSADDR_PREFIX, DNS_TXT_RECORD_TYPE,
    MAX_DNSADDR_TXT_BYTES,
};

#[cfg(not(target_arch = "wasm32"))]
pub(super) async fn lookup_dnsaddr_txt(
    query_name: &str,
    dnsaddr: &DnsaddrConfig,
) -> Result<Vec<String>, String> {
    let timeout = dnsaddr.timeout();
    let endpoint = dnsaddr.doh_endpoint.trim();
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|err| err.to_string())?;

    let response = crate::runtime::timeout(
        timeout,
        client
            .get(endpoint)
            .header("accept", "application/dns-json")
            .query(&[("name", query_name.trim_end_matches('.')), ("type", "TXT")])
            .send(),
    )
    .await
    .map_err(|_| format!("TXT lookup timed out for {query_name}"))?
    .map_err(|err| err.to_string())?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "TXT lookup for {query_name} failed with HTTP {status}"
        ));
    }

    let body: DnsJsonResponse = response.json().await.map_err(|err| err.to_string())?;
    dns_json_answers(query_name, body)
}

#[cfg(target_arch = "wasm32")]
pub(super) async fn lookup_dnsaddr_txt(
    query_name: &str,
    dnsaddr: &DnsaddrConfig,
) -> Result<Vec<String>, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Request, RequestInit, RequestMode, Response};

    let mut url = url::Url::parse(dnsaddr.doh_endpoint.trim()).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("name", query_name.trim_end_matches('.'))
        .append_pair("type", "TXT");

    let init = RequestInit::new();
    init.set_method("GET");
    init.set_mode(RequestMode::Cors);
    let request = Request::new_with_str_and_init(url.as_str(), &init).map_err(js_error_string)?;
    request
        .headers()
        .set("accept", "application/dns-json")
        .map_err(js_error_string)?;
    let window = web_sys::window().ok_or_else(|| "browser Window is unavailable".to_string())?;
    let response = crate::runtime::timeout(
        dnsaddr.timeout(),
        JsFuture::from(window.fetch_with_request(&request)),
    )
    .await
    .map_err(|_| format!("TXT lookup timed out for {query_name}"))?
    .map_err(js_error_string)?;
    let response: Response = response
        .dyn_into()
        .map_err(|_| "DoH fetch returned a non-Response value".to_string())?;
    if !response.ok() {
        return Err(format!(
            "TXT lookup for {query_name} failed with HTTP {}",
            response.status()
        ));
    }
    let json = response.json().map_err(js_error_string)?;
    let value = JsFuture::from(json).await.map_err(js_error_string)?;
    let body: DnsJsonResponse = serde_wasm_bindgen::from_value(value)
        .map_err(|e| format!("invalid DoH JSON response: {e}"))?;
    dns_json_answers(query_name, body)
}

fn dns_json_answers(query_name: &str, body: DnsJsonResponse) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for answer in body.answer.unwrap_or_default() {
        if answer.record_type != DNS_TXT_RECORD_TYPE {
            continue;
        }
        if answer.data.len() > MAX_DNSADDR_TXT_BYTES {
            return Err(format!(
                "TXT record exceeded {MAX_DNSADDR_TXT_BYTES} bytes for {query_name}"
            ));
        }
        let text = decode_dnsaddr_txt_value(&answer.data)?;
        if text.starts_with(DNSADDR_PREFIX) {
            out.push(text);
        }
    }
    Ok(out)
}

#[cfg(target_arch = "wasm32")]
fn js_error_string(value: wasm_bindgen::JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| format!("browser JavaScript error: {value:?}"))
}

#[derive(Debug, Deserialize)]
struct DnsJsonResponse {
    #[serde(rename = "Answer")]
    answer: Option<Vec<DnsJsonAnswer>>,
}

#[derive(Debug, Deserialize)]
struct DnsJsonAnswer {
    #[serde(rename = "type")]
    record_type: u32,
    data: String,
}
