use std::env;
use std::net::IpAddr;

/// Where the MySQL database is. Either one URL, or separate parts (easier when
/// the password has characters like @ or % that a URL would need escaped).
#[derive(Clone, Debug)]
pub enum DbConfig {
    Url(String),
    Parts {
        host: String,
        port: u16,
        database: String,
        username: String,
        password: String,
    },
}

impl DbConfig {
    fn from_env() -> Result<Self, String> {
        let get = |name: &str| env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        if let Some(url) = get("DATABASE_URL") {
            // Accept the JDBC form Pelican shows (jdbc:mysql://...) as well.
            let url = url.strip_prefix("jdbc:").unwrap_or(&url).to_string();
            if !url.starts_with("mysql://") && !url.starts_with("mariadb://") {
                return Err("DATABASE_URL must start with mysql://".into());
            }
            return Ok(Self::Url(url.replacen("mariadb://", "mysql://", 1)));
        }
        let host = get("DB_HOST");
        let database = get("DB_DATABASE");
        let username = get("DB_USERNAME");
        match (host, database, username) {
            (Some(host), Some(database), Some(username)) => Ok(Self::Parts {
                host,
                port: get("DB_PORT").and_then(|p| p.parse().ok()).unwrap_or(3306),
                database,
                username,
                // Not trimmed: passwords may legitimately end in spaces.
                password: env::var("DB_PASSWORD").unwrap_or_default(),
            }),
            _ => Err("No database configured. Set DB_HOST, DB_DATABASE, DB_USERNAME and DB_PASSWORD \
                      (or DATABASE_URL=mysql://user:password@host:3306/database)."
                .into()),
        }
    }

    /// host:port/database, without credentials, for log messages.
    pub fn describe(&self) -> String {
        match self {
            Self::Parts {
                host, port, database, ..
            } => format!("{host}:{port}/{database}"),
            Self::Url(url) => {
                let rest = url.trim_start_matches("mysql://");
                let rest = rest.rsplit_once('@').map(|(_, r)| r).unwrap_or(rest);
                rest.split('?').next().unwrap_or(rest).to_string()
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
    pub bind: IpAddr,
    pub db: DbConfig,
    /// Proxies (such as cloudflared) whose X-Forwarded-* headers are believed.
    pub trusted_proxies: Vec<IpAddr>,
    /// Extra origins allowed to make changes, e.g. https://paracord.example.com
    pub public_origins: Vec<String>,
    /// Force the Secure flag on the session cookie.
    pub secure_cookies: bool,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
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
        Ok(Self {
            port,
            bind,
            db: DbConfig::from_env()?,
            trusted_proxies,
            public_origins,
            secure_cookies,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::DbConfig;

    #[test]
    fn describe_hides_credentials() {
        let cfg = DbConfig::Url("mysql://user:p%40ss@db.example:3306/app?ssl-mode=disabled".into());
        assert_eq!(cfg.describe(), "db.example:3306/app");
    }
}
