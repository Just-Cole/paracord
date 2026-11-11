use std::env;
use std::net::IpAddr;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
    pub bind: IpAddr,
    pub data_dir: PathBuf,
    /// Proxies (such as cloudflared) whose X-Forwarded-* headers are believed.
    pub trusted_proxies: Vec<IpAddr>,
    /// Extra origins allowed to make changes, e.g. https://paracord.example.com
    pub public_origins: Vec<String>,
    /// Force the Secure flag on the session cookie.
    pub secure_cookies: bool,
}

impl Config {
    pub fn from_env() -> Self {
        // PORT wins; Pelican sets SERVER_PORT to the allocated port.
        let port = env::var("PORT")
            .ok()
            .or_else(|| env::var("SERVER_PORT").ok())
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(3000);
        let bind = env::var("BIND")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(IpAddr::from([0, 0, 0, 0]));
        let data_dir = env::var("DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("data"));
        let trusted_proxies = env::var("TRUST_PROXY")
            .unwrap_or_else(|_| "127.0.0.1,::1".to_string())
            .split(',')
            .filter_map(|v| v.trim().parse().ok())
            .collect();
        let public_origins = env::var("PUBLIC_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(|v| v.trim().trim_end_matches('/').to_string())
            .filter(|v| !v.is_empty())
            .collect();
        let secure_cookies = matches!(
            env::var("SECURE_COOKIES").unwrap_or_default().trim(),
            "1" | "true" | "yes"
        );
        Self {
            port,
            bind,
            data_dir,
            trusted_proxies,
            public_origins,
            secure_cookies,
        }
    }
}
