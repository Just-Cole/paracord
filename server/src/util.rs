use std::net::{IpAddr, SocketAddr};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::HeaderMap;
use base64::Engine;
use sha2::{Digest, Sha256};

use crate::config::Config;

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).expect("system random source");
    buf
}

/// Time-ordered id: 12 hex chars of milliseconds plus 10 random hex chars.
/// Sorting ids as text sorts them by creation time.
pub fn new_id() -> String {
    format!("{:012x}{}", now_ms(), hex::encode(random_bytes::<5>()))
}

pub fn new_token() -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes::<32>())
}

pub fn sha256_hex(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

pub fn random_hue() -> i64 {
    let [a, b] = random_bytes::<2>();
    (u16::from_le_bytes([a, b]) % 360) as i64
}

fn from_trusted_proxy(cfg: &Config, peer: &SocketAddr) -> bool {
    let ip = peer.ip().to_canonical();
    cfg.trusted_proxies.iter().any(|p| p.to_canonical() == ip)
}

/// The caller's IP. Forwarded headers are only believed from a trusted proxy.
pub fn client_ip(cfg: &Config, headers: &HeaderMap, peer: &SocketAddr) -> IpAddr {
    if from_trusted_proxy(cfg, peer) {
        let forwarded = headers
            .get("cf-connecting-ip")
            .or_else(|| headers.get("x-forwarded-for"))
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .and_then(|v| v.trim().parse::<IpAddr>().ok());
        if let Some(ip) = forwarded {
            return ip;
        }
    }
    peer.ip().to_canonical()
}

pub fn is_https(cfg: &Config, headers: &HeaderMap, peer: &SocketAddr) -> bool {
    if cfg.secure_cookies {
        return true;
    }
    from_trusted_proxy(cfg, peer)
        && headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("https"))
}
