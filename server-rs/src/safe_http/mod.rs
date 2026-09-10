//! Bounded outbound HTTP helpers for user-configured sources and actions.

use std::collections::HashSet;
use std::net::{IpAddr, ToSocketAddrs};
use std::time::Duration;
use thiserror::Error;
use url::Url;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum SafeHttpError {
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error("unsafe URL: {0}")]
    UnsafeUrl(String),
    #[error("response too large: {0}")]
    ResponseTooLarge(String),
    #[error("HTTP error: {0}")]
    HttpError(String),
    #[error("network error: {0}")]
    NetworkError(String),
}

pub type UnsafeUrlError = SafeHttpError;

pub fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            let oct = ipv4.octets();
            // Loopback: 127.0.0.0/8
            oct[0] == 127 ||
            // 10.0.0.0/8
            oct[0] == 10 ||
            // 172.16.0.0/12
            (oct[0] == 172 && (16..=31).contains(&oct[1])) ||
            // 192.168.0.0/16
            (oct[0] == 192 && oct[1] == 168) ||
            // Link-local: 169.254.0.0/16
            (oct[0] == 169 && oct[1] == 254) ||
            // Broadcast / current net
            oct[0] == 0 || oct[0] >= 224
        }
        IpAddr::V6(ipv6) => {
            ipv6.is_loopback() || ipv6.is_unspecified()
        }
    }
}

pub fn parse_remote_url(raw: &str, allowed_hosts: &HashSet<String>) -> Result<Url, SafeHttpError> {
    let parsed = Url::parse(raw).map_err(|e| SafeHttpError::InvalidUrl(e.to_string()))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(SafeHttpError::InvalidUrl("only http and https URLs are allowed".into()));
    }
    if parsed.username() != "" || parsed.password().is_some() {
        return Err(SafeHttpError::InvalidUrl("URL credentials are not allowed".into()));
    }
    let host = parsed.host_str().ok_or_else(|| SafeHttpError::InvalidUrl("missing host".into()))?;
    let host_lower = host.to_lowercase();
    if !allowed_hosts.contains(&host_lower) {
        if let Ok(ip) = host.parse::<IpAddr>() {
            if is_private_ip(&ip) {
                return Err(SafeHttpError::UnsafeUrl("private addresses are not allowed".into()));
            }
        }
    }
    Ok(parsed)
}

#[derive(Clone)]
pub struct SafeHttp {
    client: reqwest::Client,
    allowed_hosts: HashSet<String>,
}

impl SafeHttp {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            allowed_hosts: HashSet::new(),
        }
    }

    pub fn with_allowed_hosts(hosts: HashSet<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            allowed_hosts: hosts.into_iter().map(|h| h.to_lowercase()).collect(),
        }
    }

    pub async fn fetch_bytes(
        &self,
        url_str: &str,
        max_bytes: usize,
    ) -> Result<(bytes::Bytes, String), SafeHttpError> {
        self.fetch_bytes_with_method(url_str, max_bytes, "GET", None).await
    }

    pub async fn fetch_bytes_with_method(
        &self,
        url_str: &str,
        max_bytes: usize,
        method: &str,
        body: Option<&str>,
    ) -> Result<(bytes::Bytes, String), SafeHttpError> {
        let parsed = parse_remote_url(url_str, &self.allowed_hosts)?;
        let host = parsed.host_str().unwrap().to_lowercase();

        if !self.allowed_hosts.contains(&host) {
            let host_to_resolve = host.clone();
            let port = parsed.port_or_known_default().unwrap_or(80);
            let is_private = tokio::task::spawn_blocking(move || {
                let addr_str = format!("{}:{}", host_to_resolve, port);
                if let Ok(iter) = addr_str.to_socket_addrs() {
                    for addr in iter {
                        if is_private_ip(&addr.ip()) {
                            return true;
                        }
                    }
                }
                false
            })
            .await
            .unwrap_or(false);

            if is_private {
                return Err(SafeHttpError::UnsafeUrl("private addresses are not allowed".into()));
            }
        }

        let req_method = if method.eq_ignore_ascii_case("POST") {
            reqwest::Method::POST
        } else {
            reqwest::Method::GET
        };

        let mut req = self.client.request(req_method, parsed.as_str());
        if let Some(b) = body {
            if !b.is_empty() {
                req = req.body(b.to_string());
            }
        }

        let resp = req.send().await.map_err(|e| SafeHttpError::HttpError(e.to_string()))?;
        let final_url = resp.url().to_string();

        if !resp.status().is_success() {
            return Err(SafeHttpError::HttpError(format!("HTTP {}", resp.status())));
        }

        if let Some(len) = resp.content_length() {
            if len as usize > max_bytes {
                return Err(SafeHttpError::ResponseTooLarge(format!("exceeds max bytes {max_bytes}")));
            }
        }

        let b = resp.bytes().await.map_err(|e| SafeHttpError::NetworkError(e.to_string()))?;
        if b.len() > max_bytes {
            return Err(SafeHttpError::ResponseTooLarge(format!("exceeds max bytes {max_bytes}")));
        }

        Ok((b, final_url))
    }
}
