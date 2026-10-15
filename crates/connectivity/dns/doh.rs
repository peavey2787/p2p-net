//! Bounded DNS-over-HTTPS TXT lookups for `/dnsaddr` resolution over the
//! shared JSON HTTP client (`reqwest` natively, browser `fetch` on wasm32).

use serde::Deserialize;

use crate::common::http::{fetch_json, JsonRequest};

use super::{
    decode_dnsaddr_txt_value, DnsaddrConfig, DNSADDR_PREFIX, DNS_TXT_RECORD_TYPE,
    MAX_DNSADDR_TXT_BYTES,
};

pub(super) async fn lookup_dnsaddr_txt(
    query_name: &str,
    dnsaddr: &DnsaddrConfig,
) -> Result<Vec<String>, String> {
    let mut url = url::Url::parse(dnsaddr.doh_endpoint.trim()).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("name", query_name.trim_end_matches('.'))
        .append_pair("type", "TXT");
    let request =
        JsonRequest::get(url.as_str(), dnsaddr.timeout()).header("accept", "application/dns-json");
    let body: DnsJsonResponse = fetch_json(request)
        .await
        .map_err(|err| format!("TXT lookup for {query_name} failed: {err}"))?;
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
