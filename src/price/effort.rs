//! Splits a provider reasoning-level suffix off a model name when the base model is known.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::database::DatabaseError;
use crate::usage::UsageEvent;

const EFFORT_SUFFIXES: [&str; 6] = ["minimal", "low", "medium", "high", "xhigh", "max"];

/// Gateway provider names mapped to their upstream `LiteLLM` provider prefix for models listed
/// under multiple resellers (for example `gemini/`, `vertex_ai/`, `github_copilot/`).
const PROVIDER_HINTS: [(&str, &str); 4] = [
    ("antigravity", "gemini"),
    ("gemini-cli", "gemini"),
    ("aistudio", "gemini"),
    ("vertex", "vertex_ai"),
];

/// Cached model price keys and derived `(provider, model) -> base` cuts, refreshed on startup
/// and price updates and extended in memory on ingest.
#[derive(Debug)]
pub struct ModelCache(RwLock<ModelNames>);

#[derive(Debug, Default)]
struct ModelNames {
    direct: HashSet<String>,
    any_bare: HashSet<String>,
    unique_bare: HashMap<String, String>,
    priced_as: BTreeMap<String, String>,
    observed: HashSet<(String, String)>,
    cuts_json: String,
}

impl ModelCache {
    pub async fn load(pool: &SqlitePool) -> Result<Self, DatabaseError> {
        let price_models = load_price_models(pool).await?;
        let observed: Vec<(String, String)> =
            sqlx::query_as("SELECT DISTINCT provider, model FROM usage_event")
                .fetch_all(pool)
                .await?;
        Ok(Self(RwLock::new(ModelNames::new(
            &price_models,
            BTreeMap::new(),
            observed.into_iter().collect(),
        ))))
    }

    pub fn set_priced_as(&self, priced_as: BTreeMap<String, String>) {
        let mut names = self.write();
        names.priced_as = priced_as;
        names.rebuild_cuts();
    }

    /// Reloads price keys from `model_price` while preserving any models observed in memory.
    pub async fn refresh(&self, pool: &SqlitePool) -> Result<(), DatabaseError> {
        let price_models = load_price_models(pool).await?;
        let mut names = self.write();
        names.set_price_models(&price_models);
        names.rebuild_cuts();
        Ok(())
    }

    pub fn observe<'a>(&self, pairs: impl IntoIterator<Item = (&'a str, &'a str)>) {
        let mut names = self.write();
        let mut changed = false;
        for (provider, model) in pairs {
            if names
                .observed
                .insert((provider.to_owned(), model.to_owned()))
                && names.split_for(provider, model).is_some()
            {
                changed = true;
            }
        }
        if changed {
            names.rebuild_cuts();
        }
    }

    #[must_use]
    pub fn cuts_json(&self) -> String {
        self.read().cuts_json.clone()
    }

    /// Resolves `unpriced` `(provider, model)` pairs to `{provider, model, price_model}` entries
    /// only for names that `split_effort_suffix` cut.
    #[must_use]
    pub fn resolved_as_json(&self, unpriced: &[(String, String)]) -> String {
        let names = self.read();
        Value::Array(
            unpriced
                .iter()
                .filter_map(|(provider, model)| {
                    let price_model = names.cut_price_key(provider, model)?;
                    Some(json!({
                        "provider": provider,
                        "model": model,
                        "price_model": price_model,
                    }))
                })
                .collect(),
        )
        .to_string()
    }

    /// Applies the suffix rule to an event read from storage without altering the stored row.
    /// A stored `reasoning_effort` wins; `model_alias` stays as stored; `raw_model` carries the
    /// stored upstream name when cut.
    pub fn normalize_event(&self, event: &mut UsageEvent) {
        let names = self.read();
        if let Some((base, effort)) = names.split_for(&event.provider, &event.model) {
            let base = base.to_owned();
            let effort = effort.to_owned();
            event.raw_model = Some(std::mem::replace(&mut event.model, base));
            if event.reasoning_effort.is_none() {
                event.reasoning_effort = Some(effort);
            }
        }
    }

    fn read(&self) -> RwLockReadGuard<'_, ModelNames> {
        self.0
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, ModelNames> {
        self.0
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl ModelNames {
    fn new(
        price_models: &[String],
        priced_as: BTreeMap<String, String>,
        observed: HashSet<(String, String)>,
    ) -> Self {
        let mut names = Self {
            priced_as,
            observed,
            cuts_json: "[]".to_owned(),
            ..Self::default()
        };
        names.set_price_models(price_models);
        names.rebuild_cuts();
        names
    }

    fn set_price_models(&mut self, price_models: &[String]) {
        self.direct.clear();
        self.any_bare.clear();
        self.unique_bare.clear();
        for model in price_models {
            self.direct.insert(model.clone());
        }
        let mut bare: HashMap<&str, Option<&str>> = HashMap::new();
        for model in price_models {
            if let Some((_, leaf)) = model.rsplit_once('/') {
                self.any_bare.insert(leaf.to_owned());
            }
            let Some((_, suffix)) = model.split_once('/') else {
                continue;
            };
            if !suffix.contains('/') && !self.direct.contains(suffix) {
                bare.entry(suffix)
                    .and_modify(|only| *only = None)
                    .or_insert(Some(model.as_str()));
            }
        }
        for (suffix, only) in bare {
            if let Some(key) = only {
                self.unique_bare.insert(suffix.to_owned(), key.to_owned());
            }
        }
    }

    fn is_exact_known(&self, model: &str) -> bool {
        self.direct.contains(model)
            || self.priced_as.contains_key(model)
            || self.any_bare.contains(model)
    }

    /// Resolves `base` (as given, without stripping prefixes) to its `model_price.model` key:
    /// `priced_as` configured target, exact bare key, own/hinted `provider/<base>`, then unique
    /// `provider/<base>`.
    fn base_price_key(&self, provider: &str, base: &str) -> Option<String> {
        if let Some(configured) = self.priced_as.get(base) {
            return Some(configured.clone());
        }
        if self.direct.contains(base) {
            return Some(base.to_owned());
        }
        if !base.contains('/') {
            let own = format!("{provider}/{base}");
            if self.direct.contains(&own) {
                return Some(own);
            }
            if let Some(hint) = provider_hint(provider) {
                let hinted = format!("{hint}/{base}");
                if self.direct.contains(&hinted) {
                    return Some(hinted);
                }
            }
            if let Some(unique) = self.unique_bare.get(base) {
                return Some(unique.clone());
            }
        }
        None
    }

    fn split_for<'a>(&self, provider: &str, model: &'a str) -> Option<(&'a str, &'a str)> {
        split_effort_suffix(
            model,
            |exact| self.is_exact_known(exact),
            |base| self.base_price_key(provider, base).is_some(),
        )
    }

    fn cut_price_key(&self, provider: &str, model: &str) -> Option<String> {
        let (base, _) = self.split_for(provider, model)?;
        self.base_price_key(provider, base)
    }

    fn rebuild_cuts(&mut self) {
        let mut entries: Vec<(&str, &str, &str)> = self
            .observed
            .iter()
            .filter_map(|(provider, model)| {
                let (base, _) = self.split_for(provider, model)?;
                Some((provider.as_str(), model.as_str(), base))
            })
            .collect();
        entries.sort_unstable();
        self.cuts_json = Value::Array(
            entries
                .into_iter()
                .map(|(provider, model, base)| {
                    json!({ "provider": provider, "model": model, "base": base })
                })
                .collect(),
        )
        .to_string();
    }
}

async fn load_price_models(pool: &SqlitePool) -> Result<Vec<String>, DatabaseError> {
    Ok(sqlx::query_scalar("SELECT DISTINCT model FROM model_price")
        .fetch_all(pool)
        .await?)
}

fn provider_hint(provider: &str) -> Option<&'static str> {
    PROVIDER_HINTS
        .iter()
        .find_map(|&(gateway, upstream)| (gateway == provider).then_some(upstream))
}

/// Splits a trailing `-<level>` suffix from `model` when `is_exact(model)` is false
/// and `has_base(base)` is true.
#[must_use]
pub fn split_effort_suffix(
    model: &str,
    is_exact: impl Fn(&str) -> bool,
    has_base: impl Fn(&str) -> bool,
) -> Option<(&str, &str)> {
    let (base, effort) = model.rsplit_once('-')?;
    if base.is_empty() || base.ends_with('/') || !EFFORT_SUFFIXES.contains(&effort) {
        return None;
    }
    if is_exact(model) || !has_base(base) {
        return None;
    }
    Some((base, effort))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashSet};

    use super::{ModelNames, split_effort_suffix};

    #[test]
    fn shared_effort_suffix_table() {
        type Case<'a> = (&'a str, &'a [&'a str], Option<(&'a str, &'a str)>);
        let direct_cases: &[Case<'_>] = &[
            (
                "gemini-4-neon-xhigh",
                &["gemini-4-neon"],
                Some(("gemini-4-neon", "xhigh")),
            ),
            (
                "gemini-4-argon-high",
                &["gemini-4-argon"],
                Some(("gemini-4-argon", "high")),
            ),
            (
                "gemini-4-carbon-high",
                &["gemini-4-carbon"],
                Some(("gemini-4-carbon", "high")),
            ),
            (
                "gemini-4-hydrogen-xhigh",
                &["gemini-4-hydrogen"],
                Some(("gemini-4-hydrogen", "xhigh")),
            ),
            (
                "google/gemini-4-neon-xhigh",
                &["google/gemini-4-neon"],
                Some(("google/gemini-4-neon", "xhigh")),
            ),
            (
                "gemini-4-neon-minimal",
                &["gemini-4-neon"],
                Some(("gemini-4-neon", "minimal")),
            ),
            (
                "gemini-4-neon-low",
                &["gemini-4-neon"],
                Some(("gemini-4-neon", "low")),
            ),
            (
                "gemini-4-neon-medium",
                &["gemini-4-neon"],
                Some(("gemini-4-neon", "medium")),
            ),
            (
                "gemini-4-neon-max",
                &["gemini-4-neon"],
                Some(("gemini-4-neon", "max")),
            ),
            ("kimi-k3.1-pro-highspeed", &["kimi-k3.1-pro"], None),
            ("gpt-4o-mini", &["gpt-4o"], None),
            ("mistral-medium", &["mistral", "mistral-medium"], None),
            ("qwen-max", &["qwen", "qwen-max"], None),
            ("foo-high", &["foo", "foo-high"], None),
            ("foo-high", &["foo"], Some(("foo", "high"))),
            ("something-high", &[], None),
            ("-high", &[""], None),
            ("google/-high", &["google/"], None),
        ];
        for &(model, known, expected) in direct_cases {
            let actual = split_effort_suffix(
                model,
                |exact| known.contains(&exact),
                |base| known.contains(&base),
            );
            assert_eq!(actual, expected, "model={model:?} known={known:?}");
        }
    }

    #[test]
    fn provider_hints_and_ambiguity() {
        let map_keys: Vec<String> = [
            "gemini/gemini-4-neon",
            "vertex_ai/gemini-4-neon",
            "github_copilot/gemini-4-neon",
            "azure_ai/gpt-oss-120b",
            "cerebras/gpt-oss-120b",
            "azure_ai/grok-4",
            "groq/llama-high",
            "cerebras/llama-high",
            "llama",
            "novita/qwen/qwen3-max",
            "qwen3",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let priced_as = BTreeMap::from([
            (
                "kimi-k3-high".to_owned(),
                "moonshot/kimi-k3-turbo".to_owned(),
            ),
            ("custom-base".to_owned(), "cerebras/gpt-oss-120b".to_owned()),
        ]);
        let names = ModelNames::new(&map_keys, priced_as, HashSet::new());

        let provider_cases: &[(&str, &str, Option<&str>)] = &[
            (
                "antigravity",
                "gemini-4-neon-xhigh",
                Some("gemini/gemini-4-neon"),
            ),
            (
                "gemini-cli",
                "gemini-4-neon-high",
                Some("gemini/gemini-4-neon"),
            ),
            (
                "aistudio",
                "gemini-4-neon-medium",
                Some("gemini/gemini-4-neon"),
            ),
            (
                "vertex",
                "gemini-4-neon-low",
                Some("vertex_ai/gemini-4-neon"),
            ),
            ("antigravity", "gemini-4-neon", None),
            ("openrouter", "gemini-4-neon-xhigh", None),
            ("antigravity", "azure/gemini-4-neon-high", None),
            ("antigravity", "gpt-oss-120b-medium", None),
            ("xai", "grok-4-high", Some("azure_ai/grok-4")),
            ("antigravity", "llama-high", None),
            ("antigravity", "qwen3-max", None),
            ("moonshot", "kimi-k3-high", None),
            (
                "antigravity",
                "custom-base-high",
                Some("cerebras/gpt-oss-120b"),
            ),
        ];
        for &(provider, model, expected) in provider_cases {
            assert_eq!(
                names.cut_price_key(provider, model).as_deref(),
                expected,
                "provider={provider:?} model={model:?}"
            );
        }
    }
}
