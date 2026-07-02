//! DNS multiaddr resolution for configured and cached peer addresses.
//!
//! p2p-net keeps DNS support enabled by default. `/dns`, `/dns4`, and `/dns6`
//! are resolved through Tokio's OS resolver. `/dnsaddr` is resolved through a
//! bounded DNS-over-HTTPS TXT lookup path with recursion, count, size, and
//! timeout limits. LAN multicast discovery is intentionally not included.

use crate::common::error::config_error;
use std::{net::IpAddr, time::Duration};

use serde::{Deserialize, Serialize};

use libp2p::multiaddr::Protocol;
use libp2p::Multiaddr;

use crate::common::error::NetError;

const DNSADDR_PREFIX: &str = "dnsaddr=";
const DNSADDR_QUERY_PREFIX: &str = "_dnsaddr.";
const MAX_DNSADDR_RECURSION: usize = 8;
const MAX_DNSADDR_RECORDS_PER_LOOKUP: usize = 32;
const MAX_DNSADDR_TOTAL_RECORDS: usize = 128;
const MAX_DNSADDR_TXT_BYTES: usize = 4096;
const DNS_TXT_RECORD_TYPE: u32 = 16;

/// Default DoH endpoint used for `/dnsaddr` TXT lookups.
///
/// Operators can replace this with an internal/self-hosted resolver through
/// [`DnsaddrConfig::doh_endpoint`].
pub const DEFAULT_DNSADDR_DOH_ENDPOINT: &str = "https://cloudflare-dns.com/dns-query";
pub const DEFAULT_DNSADDR_TIMEOUT_SECS: u64 = 5;
const MAX_DNSADDR_TIMEOUT_SECS: u64 = 60;

/// `/dnsaddr` TXT lookup policy.
///
/// The default keeps `/dnsaddr` working out of the box through bounded DoH.
/// Production deployments that do not want a hard-coded third-party dependency
/// should set `doh_endpoint` to an internal or self-hosted HTTPS DoH resolver.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DnsaddrConfig {
    /// Enable `/dnsaddr` TXT lookups. Set to false to reject configured `/dnsaddr`
    /// addresses and ignore cached/discovered `/dnsaddr` addresses.
    pub enabled: bool,
    /// HTTPS DNS-over-HTTPS JSON endpoint for TXT lookups.
    pub doh_endpoint: String,
    /// Per-request timeout for `/dnsaddr` TXT lookups.
    pub timeout_secs: u64,
}

impl Default for DnsaddrConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            doh_endpoint: DEFAULT_DNSADDR_DOH_ENDPOINT.to_string(),
            timeout_secs: DEFAULT_DNSADDR_TIMEOUT_SECS,
        }
    }
}

impl DnsaddrConfig {
    pub fn validate(&self) -> Result<(), NetError> {
        if !self.enabled {
            return Ok(());
        }
        if self.timeout_secs == 0 || self.timeout_secs > MAX_DNSADDR_TIMEOUT_SECS {
            return Err(config_error(format!(
                "dnsaddr.timeout_secs must be between 1 and {MAX_DNSADDR_TIMEOUT_SECS}"
            )));
        }
        let endpoint = self.doh_endpoint.trim();
        if endpoint.is_empty() {
            return Err(config_error(
                "dnsaddr.doh_endpoint must not be empty when dnsaddr is enabled",
            ));
        }
        let url = reqwest::Url::parse(endpoint).map_err(|err| {
            config_error(format!(
                "dnsaddr.doh_endpoint must be a valid HTTPS URL: {err}"
            ))
        })?;
        if url.scheme() != "https" {
            return Err(config_error(
                "dnsaddr.doh_endpoint must use https:// to avoid plaintext DNS queries",
            ));
        }
        if url.host_str().is_none() {
            return Err(config_error("dnsaddr.doh_endpoint must include a host"));
        }
        Ok(())
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }
}

/// Resolve configured DNS multiaddrs. Non-DNS multiaddrs are returned unchanged.
pub async fn resolve_configured_multiaddrs(
    field: &str,
    addrs: Vec<Multiaddr>,
    dnsaddr: &DnsaddrConfig,
) -> Result<Vec<Multiaddr>, NetError> {
    let mut out = Vec::new();
    for addr in addrs {
        match resolve_multiaddr(&addr, dnsaddr).await {
            Ok(mut resolved) => out.append(&mut resolved),
            Err(reason) => {
                return Err(NetError::Config {
                    path: "<config>".to_string(),
                    reason: format!("{field} DNS resolution failed for `{addr}`: {reason}"),
                });
            }
        }
    }
    Ok(dedup_multiaddrs(out))
}

/// Resolve cached/discovered DNS multiaddrs best-effort. Unresolvable DNS entries are ignored.
pub async fn resolve_cached_multiaddrs(
    addrs: Vec<Multiaddr>,
    dnsaddr: &DnsaddrConfig,
) -> Vec<Multiaddr> {
    let mut out = Vec::new();
    for addr in addrs {
        if let Ok(mut resolved) = resolve_multiaddr(&addr, dnsaddr).await {
            out.append(&mut resolved);
        }
    }
    dedup_multiaddrs(out)
}

/// True when the multiaddr contains ordinary DNS name components supported by this resolver.
pub fn has_resolvable_dns(addr: &Multiaddr) -> bool {
    addr.iter()
        .any(|p| matches!(p, Protocol::Dns(_) | Protocol::Dns4(_) | Protocol::Dns6(_)))
}

/// True when the multiaddr contains `/dnsaddr`, which resolves through TXT records.
pub fn has_dnsaddr(addr: &Multiaddr) -> bool {
    addr.iter().any(|p| matches!(p, Protocol::Dnsaddr(_)))
}

/// True when the multiaddr contains any DNS-family component.
pub fn has_any_dns(addr: &Multiaddr) -> bool {
    has_resolvable_dns(addr) || has_dnsaddr(addr)
}

async fn resolve_multiaddr(
    addr: &Multiaddr,
    dnsaddr: &DnsaddrConfig,
) -> Result<Vec<Multiaddr>, String> {
    if has_dnsaddr(addr) {
        if !dnsaddr.enabled {
            return Err("/dnsaddr resolution is disabled by config".to_string());
        }
        return resolve_dnsaddr(addr.clone(), 0, 0, dnsaddr).await;
    }
    resolve_ordinary_dns_multiaddr(addr).await
}

async fn resolve_ordinary_dns_multiaddr(addr: &Multiaddr) -> Result<Vec<Multiaddr>, String> {
    if !has_resolvable_dns(addr) {
        return Ok(vec![addr.clone()]);
    }

    let protocols: Vec<_> = addr.iter().collect();
    let Some((dns_idx, host, family)) = first_dns_component(&protocols) else {
        return Ok(vec![addr.clone()]);
    };

    let lookup = tokio::net::lookup_host((host.as_str(), 0))
        .await
        .map_err(|err| err.to_string())?;

    let mut out = Vec::new();
    for socket in lookup {
        let ip = socket.ip();
        if !family.allows(ip) {
            continue;
        }

        let mut resolved = Multiaddr::empty();
        for (idx, protocol) in protocols.iter().cloned().enumerate() {
            if idx == dns_idx {
                match ip {
                    IpAddr::V4(ip4) => resolved.push(Protocol::Ip4(ip4)),
                    IpAddr::V6(ip6) => resolved.push(Protocol::Ip6(ip6)),
                }
            } else {
                resolved.push(protocol.acquire());
            }
        }
        out.push(resolved);
    }

    if out.is_empty() {
        Err(format!("no matching A/AAAA records for {host}"))
    } else {
        Ok(dedup_multiaddrs(out))
    }
}

async fn resolve_dnsaddr(
    addr: Multiaddr,
    depth: usize,
    total_records: usize,
    dnsaddr: &DnsaddrConfig,
) -> Result<Vec<Multiaddr>, String> {
    if depth > MAX_DNSADDR_RECURSION {
        return Err(format!(
            "dnsaddr recursion exceeded {MAX_DNSADDR_RECURSION} levels"
        ));
    }
    if total_records > MAX_DNSADDR_TOTAL_RECORDS {
        return Err(format!(
            "dnsaddr total records exceeded {MAX_DNSADDR_TOTAL_RECORDS}"
        ));
    }

    let (domain, suffix) = split_first_dnsaddr(&addr)
        .ok_or_else(|| format!("missing /dnsaddr component in {addr}"))?;
    let query_name = dnsaddr_query_name(&domain);
    let records = lookup_dnsaddr_txt(&query_name, dnsaddr).await?;

    if records.is_empty() {
        return Err(format!("no dnsaddr TXT records found for {query_name}"));
    }
    if records.len() > MAX_DNSADDR_RECORDS_PER_LOOKUP {
        return Err(format!(
            "dnsaddr TXT record count {} exceeded {} for {}",
            records.len(),
            MAX_DNSADDR_RECORDS_PER_LOOKUP,
            query_name
        ));
    }

    let mut out = Vec::new();
    for record in records {
        let raw_addr = record
            .strip_prefix(DNSADDR_PREFIX)
            .ok_or_else(|| format!("invalid dnsaddr TXT record `{record}`"))?;
        if raw_addr.len() > MAX_DNSADDR_TXT_BYTES {
            return Err(format!(
                "dnsaddr TXT value exceeded {MAX_DNSADDR_TXT_BYTES} bytes"
            ));
        }

        let candidate: Multiaddr = raw_addr
            .parse()
            .map_err(|err| format!("invalid dnsaddr multiaddr `{raw_addr}`: {err}"))?;

        let mut resolved = if has_dnsaddr(&candidate) {
            Box::pin(resolve_dnsaddr(
                candidate,
                depth + 1,
                total_records + out.len() + 1,
                dnsaddr,
            ))
            .await?
        } else if has_resolvable_dns(&candidate) {
            resolve_ordinary_dns_multiaddr(&candidate).await?
        } else {
            vec![candidate]
        };
        if !suffix.is_empty() {
            resolved.retain(|candidate| multiaddr_ends_with(candidate, &suffix));
        }
        out.append(&mut resolved);
        if total_records + out.len() > MAX_DNSADDR_TOTAL_RECORDS {
            return Err(format!(
                "dnsaddr resolved address count exceeded {MAX_DNSADDR_TOTAL_RECORDS}"
            ));
        }
    }

    Ok(dedup_multiaddrs(out))
}

async fn lookup_dnsaddr_txt(
    query_name: &str,
    dnsaddr: &DnsaddrConfig,
) -> Result<Vec<String>, String> {
    let timeout = dnsaddr.timeout();
    let endpoint = dnsaddr.doh_endpoint.trim();
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|err| err.to_string())?;

    let response = tokio::time::timeout(
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
        let text = decode_dns_json_txt(&answer.data)?;
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

fn decode_dns_json_txt(data: &str) -> Result<String, String> {
    let trimmed = data.trim();
    if trimmed.len() > MAX_DNSADDR_TXT_BYTES {
        return Err(format!("TXT record exceeded {MAX_DNSADDR_TXT_BYTES} bytes"));
    }
    if !trimmed.contains('"') {
        return Ok(trimmed.to_string());
    }

    let mut out = String::new();
    let mut in_quote = false;
    let mut chars = trimmed.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => in_quote = !in_quote,
            '\\' if in_quote => {
                let Some(next) = chars.next() else {
                    return Err("TXT record has trailing escape".to_string());
                };
                out.push(next);
            }
            c if in_quote => out.push(c),
            c if c.is_whitespace() => {}
            other => return Err(format!("unexpected unquoted TXT character `{other}`")),
        }
        if out.len() > MAX_DNSADDR_TXT_BYTES {
            return Err(format!("TXT record exceeded {MAX_DNSADDR_TXT_BYTES} bytes"));
        }
    }
    if in_quote {
        return Err("TXT record has unterminated quote".to_string());
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy)]
enum DnsFamily {
    Any,
    V4,
    V6,
}

impl DnsFamily {
    fn allows(self, ip: IpAddr) -> bool {
        matches!(
            (self, ip),
            (Self::Any, _) | (Self::V4, IpAddr::V4(_)) | (Self::V6, IpAddr::V6(_))
        )
    }
}

fn first_dns_component(protocols: &[Protocol<'_>]) -> Option<(usize, String, DnsFamily)> {
    for (idx, protocol) in protocols.iter().enumerate() {
        match protocol {
            Protocol::Dns(host) => return Some((idx, host.to_string(), DnsFamily::Any)),
            Protocol::Dns4(host) => return Some((idx, host.to_string(), DnsFamily::V4)),
            Protocol::Dns6(host) => return Some((idx, host.to_string(), DnsFamily::V6)),
            _ => {}
        }
    }
    None
}

fn split_first_dnsaddr(addr: &Multiaddr) -> Option<(String, Vec<Protocol<'static>>)> {
    let mut domain = None;
    let mut suffix = Vec::new();

    for protocol in addr.iter() {
        if domain.is_none() {
            if let Protocol::Dnsaddr(name) = protocol {
                domain = Some(name.to_string());
            }
        } else {
            suffix.push(protocol.acquire());
        }
    }

    domain.map(|domain| (domain, suffix))
}

fn dnsaddr_query_name(domain: &str) -> String {
    let trimmed = domain.trim_end_matches('.');
    format!("{DNSADDR_QUERY_PREFIX}{trimmed}.")
}

fn multiaddr_ends_with(addr: &Multiaddr, suffix: &[Protocol<'static>]) -> bool {
    let protocols = addr
        .iter()
        .map(Protocol::acquire)
        .collect::<Vec<Protocol<'static>>>();
    protocols.ends_with(suffix)
}

fn dedup_multiaddrs(addrs: Vec<Multiaddr>) -> Vec<Multiaddr> {
    let mut out = Vec::new();
    for addr in addrs {
        if !out.contains(&addr) {
            out.push(addr);
        }
    }
    out
}

#[cfg(test)]
mod tests;
