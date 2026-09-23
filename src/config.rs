//! The file that declares which gateways this service collects from and where prices come
//! from. Secrets are never written in it: a source names the environment variable that holds
//! its key, and the key is read once, at load, so a missing variable stops startup.

use std::fmt;
use std::path::Path;

use serde::Deserialize;

use crate::source::SourceKind;

/// The price map `LiteLLM` itself defaults to, `litellm/__init__.py:421-424`.
pub const PUBLIC_PRICE_MAP_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";

const DEFAULT_PRICE_REFRESH_HOURS: u64 = 24;

#[derive(Debug, Default)]
pub struct Config {
    pub sources: Vec<SourceConfig>,
    pub pricing: PricingConfig,
}

#[derive(Debug)]
pub struct SourceConfig {
    /// Stable across restarts and renames; the database finds the source by it.
    pub key: String,
    pub name: String,
    pub kind: SourceKind,
    pub base_url: String,
    /// The `CLIProxy` management key or the `LiteLLM` master key.
    pub secret: Secret,
}

#[derive(Debug)]
pub struct PricingConfig {
    /// A `LiteLLM` source whose live price map outranks the public one.
    pub litellm_source: Option<String>,
    pub public_map_url: String,
    pub refresh_hours: u64,
}

impl Default for PricingConfig {
    fn default() -> Self {
        Self {
            litellm_source: None,
            public_map_url: PUBLIC_PRICE_MAP_URL.to_owned(),
            refresh_hours: DEFAULT_PRICE_REFRESH_HOURS,
        }
    }
}

/// A credential read from the environment. It prints as a placeholder so that no `Debug`
/// or log line can carry it.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret(..)")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not read {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("{0}")]
    Parse(#[from] toml::de::Error),
    #[error("source {key:?}: environment variable {variable} is not set")]
    MissingSecret { key: String, variable: String },
    #[error("source {key:?}: environment variable {variable} is empty")]
    EmptySecret { key: String, variable: String },
    #[error("source key {0:?} appears more than once")]
    DuplicateKey(String),
    #[error("pricing.litellm_source {0:?} is not a litellm source in this file")]
    UnknownPriceSource(String),
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.display().to_string(),
            source,
        })?;
        Self::parse(&text, |variable| std::env::var(variable).ok())
    }

    /// `environment` is how a variable is looked up, so a test can supply its own.
    pub fn parse(
        text: &str,
        environment: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ConfigError> {
        let file: ConfigFile = toml::from_str(text)?;
        let mut sources = Vec::with_capacity(file.source.len());
        for source in file.source {
            if sources
                .iter()
                .any(|known: &SourceConfig| known.key == source.key)
            {
                return Err(ConfigError::DuplicateKey(source.key));
            }
            let secret =
                environment(&source.key_env).ok_or_else(|| ConfigError::MissingSecret {
                    key: source.key.clone(),
                    variable: source.key_env.clone(),
                })?;
            if secret.trim().is_empty() {
                return Err(ConfigError::EmptySecret {
                    key: source.key,
                    variable: source.key_env,
                });
            }
            sources.push(SourceConfig {
                name: source.name.unwrap_or_else(|| source.key.clone()),
                key: source.key,
                kind: source.kind,
                base_url: source.base_url.trim_end_matches('/').to_owned(),
                secret: Secret(secret),
            });
        }
        let pricing = file.pricing.unwrap_or_default();
        if let Some(key) = &pricing.litellm_source
            && !sources
                .iter()
                .any(|source| &source.key == key && source.kind == SourceKind::LiteLlm)
        {
            return Err(ConfigError::UnknownPriceSource(key.clone()));
        }

        Ok(Self {
            sources,
            pricing: PricingConfig {
                litellm_source: pricing.litellm_source,
                public_map_url: pricing
                    .public_map_url
                    .unwrap_or_else(|| PUBLIC_PRICE_MAP_URL.to_owned()),
                refresh_hours: pricing.refresh_hours.unwrap_or(DEFAULT_PRICE_REFRESH_HOURS),
            },
        })
    }

    #[must_use]
    pub fn source(&self, key: &str) -> Option<&SourceConfig> {
        self.sources.iter().find(|source| source.key == key)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    #[serde(default)]
    source: Vec<SourceFile>,
    pricing: Option<PricingFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceFile {
    key: String,
    name: Option<String>,
    kind: SourceKind,
    base_url: String,
    key_env: String,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct PricingFile {
    litellm_source: Option<String>,
    public_map_url: Option<String>,
    refresh_hours: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"
        [[source]]
        key = "watch"
        name = "watch cliproxy"
        kind = "cliproxy"
        base_url = "http://watch:8317/"
        key_env = "CLIPROXY_KEY"

        [[source]]
        key = "home"
        kind = "litellm"
        base_url = "http://litellm:4000"
        key_env = "LITELLM_KEY"

        [pricing]
        litellm_source = "home"
    "#;

    fn environment(variable: &str) -> Option<String> {
        match variable {
            "CLIPROXY_KEY" => Some("management".to_owned()),
            "LITELLM_KEY" => Some("master".to_owned()),
            _ => None,
        }
    }

    #[test]
    fn reads_sources_and_their_secrets() {
        let config = Config::parse(FILE, environment).expect("parses");
        let watch = config.source("watch").expect("watch");
        assert_eq!(watch.base_url, "http://watch:8317");
        assert_eq!(watch.secret.expose(), "management");
        assert_eq!(config.source("home").expect("home").name, "home");
        assert_eq!(config.pricing.public_map_url, PUBLIC_PRICE_MAP_URL);
    }

    #[test]
    fn a_missing_secret_names_its_variable() {
        let error = Config::parse(FILE, |_| None).expect_err("no secrets");
        assert!(error.to_string().contains("CLIPROXY_KEY"), "{error}");
    }

    #[test]
    fn a_secret_never_prints() {
        let config = Config::parse(FILE, environment).expect("parses");
        assert!(!format!("{config:?}").contains("management"));
    }

    #[test]
    fn prices_can_only_come_from_a_declared_litellm() {
        let file = FILE.replace("litellm_source = \"home\"", "litellm_source = \"watch\"");
        assert!(matches!(
            Config::parse(&file, environment),
            Err(ConfigError::UnknownPriceSource(_))
        ));
    }
}
