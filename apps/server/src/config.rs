//! Typed configuration for the `terichat-server` binary.
//!
//! [`Config::from_env`] loads `.env` files (via `dotenvy`) and then reads the
//! process environment. [`Config::from_pairs`] offers the same semantics over
//! an explicit set of pairs so unit tests never touch shared process state.

use std::collections::HashMap;
use std::fmt;
use std::net::IpAddr;

/// Default TCP port the server listens on when `PORT` is unset.
const DEFAULT_PORT: u16 = 3001;

/// Default bind address when `BIND_ADDR` is unset: loopback only. Container
/// and staging deployments set `BIND_ADDR=0.0.0.0` explicitly.
const DEFAULT_BIND: IpAddr = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

/// Default `RUST_LOG` filter directive when `RUST_LOG` is unset.
const DEFAULT_RUST_LOG: &str = "info";

/// Typed server configuration.
///
/// Defaults (each also documented on its field):
/// - `PORT` defaults to `3001`.
/// - `BIND_ADDR` defaults to `127.0.0.1` (loopback only).
/// - `RUST_LOG` defaults to `"info"`.
/// - `DATABASE_URL`, when missing or empty, defaults to [`None`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// TCP port the server binds. Defaults to `3001` when `PORT` is unset.
    pub port: u16,
    /// Local address the server binds. Defaults to loopback when `BIND_ADDR`
    /// is unset; set `0.0.0.0` for containerized deployments.
    pub bind_addr: IpAddr,
    /// Tracing/log filter directive. Defaults to `"info"` when `RUST_LOG` is unset.
    pub rust_log: String,
    /// Postgres connection URL. Defaults to [`None`] when `DATABASE_URL` is
    /// missing or empty (the server boots without a database).
    pub database_url: Option<String>,
}

/// Typed error for malformed server configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// `PORT` was set but is not a valid `u16` port number.
    InvalidPort {
        /// Raw `PORT` value that failed to parse.
        raw: String,
    },
    /// `BIND_ADDR` was set but is not a valid IP address.
    InvalidBindAddr {
        /// Raw `BIND_ADDR` value that failed to parse.
        raw: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPort { raw } => {
                write!(
                    f,
                    "invalid PORT value {raw:?}: expected a port number 0-65535"
                )
            }
            Self::InvalidBindAddr { raw } => {
                write!(f, "invalid BIND_ADDR value {raw:?}: expected an IP address")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Load configuration from the process environment.
    ///
    /// Loads `.env` first (a missing file is fine), then reads `PORT`,
    /// `BIND_ADDR`, `RUST_LOG`, and `DATABASE_URL` with the defaults
    /// documented on [`Config`].
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::InvalidPort`] when `PORT` is set but does not
    /// parse as a `u16` port number, or [`ConfigError::InvalidBindAddr`] when
    /// `BIND_ADDR` is set but does not parse as an IP address.
    pub fn from_env() -> Result<Self, ConfigError> {
        let _ = dotenvy::dotenv();
        Self::from_pairs(std::env::vars())
    }

    /// Build configuration from explicit `(key, value)` pairs.
    ///
    /// Shares the exact semantics of [`Config::from_env`] without touching
    /// process state, so tests stay hermetic. Unknown keys are ignored.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::InvalidPort`] when `PORT` is present but does
    /// not parse as a `u16` port number, or [`ConfigError::InvalidBindAddr`]
    /// when `BIND_ADDR` is present but does not parse as an IP address.
    pub fn from_pairs(
        pairs: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self, ConfigError> {
        let vars: HashMap<String, String> = pairs.into_iter().collect();

        let port = match vars.get("PORT") {
            None => DEFAULT_PORT,
            Some(raw) => raw
                .parse::<u16>()
                .map_err(|_| ConfigError::InvalidPort { raw: raw.clone() })?,
        };

        let bind_addr = match vars.get("BIND_ADDR") {
            None => DEFAULT_BIND,
            Some(raw) => raw
                .parse::<IpAddr>()
                .map_err(|_| ConfigError::InvalidBindAddr { raw: raw.clone() })?,
        };

        let rust_log = vars
            .get("RUST_LOG")
            .cloned()
            .unwrap_or_else(|| DEFAULT_RUST_LOG.to_owned());

        let database_url = vars
            .get("DATABASE_URL")
            .cloned()
            .filter(|url| !url.is_empty());

        Ok(Self {
            port,
            bind_addr,
            rust_log,
            database_url,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn defaults_when_nothing_is_set() {
        let config = Config::from_pairs(Vec::new()).unwrap();
        assert_eq!(config.port, 3001);
        assert_eq!(
            config.bind_addr,
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        );
        assert_eq!(config.rust_log, "info");
        assert_eq!(config.database_url, None);
    }

    #[test]
    fn bind_addr_parses_and_rejects_garbage() {
        let config = Config::from_pairs(pairs(&[("BIND_ADDR", "0.0.0.0")])).unwrap();
        assert_eq!(
            config.bind_addr,
            std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
        );
        assert!(matches!(
            Config::from_pairs(pairs(&[("BIND_ADDR", "not-an-ip")])),
            Err(ConfigError::InvalidBindAddr { .. })
        ));
    }

    #[test]
    fn custom_values_are_applied() {
        let config = Config::from_pairs(pairs(&[
            ("PORT", "8080"),
            ("RUST_LOG", "debug"),
            ("DATABASE_URL", "postgres://localhost/terichat"),
        ]))
        .unwrap();
        assert_eq!(config.port, 8080);
        assert_eq!(config.rust_log, "debug");
        assert_eq!(
            config.database_url,
            Some("postgres://localhost/terichat".to_owned())
        );
    }

    #[test]
    fn invalid_port_is_an_error() {
        let err = Config::from_pairs(pairs(&[("PORT", "not-a-port")])).unwrap_err();
        assert_eq!(
            err,
            ConfigError::InvalidPort {
                raw: "not-a-port".to_owned()
            }
        );
        assert!(err.to_string().contains("not-a-port"));
    }

    #[test]
    fn empty_database_url_maps_to_none() {
        let config = Config::from_pairs(pairs(&[("DATABASE_URL", "")])).unwrap();
        assert_eq!(config.database_url, None);
    }
}
