//! A sign-in failure is separate from CLIProxy's scheduling and quota exhaustion.
//! Only a successful quota read clears it. `account.changed` invalidates both reads;
//! `quota.updated` remains the invalidation for changed window values.

use std::collections::HashMap;

use poem_openapi::{Enum, Object};
use reqwest::Url;
use schemars::JsonSchema;
use serde::Serialize;

use crate::account::Account;
use crate::database::{Database, DatabaseError};
use crate::id::Id;
use crate::quota::provider::Provider;

/// An observed sign-in failure. Absence does not claim the credential is healthy.
#[derive(Debug, Clone, Copy, Enum, Serialize, JsonSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
pub enum SignInError {
    #[oai(rename = "expired")]
    Expired,
}

#[derive(Debug, Object, Serialize, JsonSchema)]
pub struct Reauthentication {
    /// CLIProxy's management UI OAuth page. Never contains a management key.
    pub url: String,
    /// What to press on that page, in English.
    pub button: String,
}

impl Reauthentication {
    /// Routes and English labels read in Management Center v1.26.7, commit 714de076.
    /// CLIProxyAPI v8.0.35 serves this separately updated UI at `management.html`.
    #[must_use]
    pub fn new(base_url: &str, provider: &str) -> Option<Self> {
        let hint = match provider.parse::<Provider>().ok()? {
            Provider::Claude => "the \"Claude\" tile, then \"Start Anthropic Login\"",
            Provider::Codex => "the \"Codex\" tile, then \"Start Codex Login\"",
            Provider::Antigravity => "the \"Antigravity\" tile, then \"Start Antigravity Login\"",
            // The quota adapter knows one `kimi` provider, so the hint names both Kimi tiles.
            Provider::Kimi => {
                "the \"Kimi China\" or \"Kimi International\" tile, then its login button"
            }
            Provider::Devin => "the \"Devin\" tile, then \"Start Devin Login\"",
            Provider::Xai => "the \"xAI\" tile, then \"Start xAI Login\"",
            Provider::Meta => "the \"Muse (Meta)\" tile, then \"Log in with Muse\"",
        };
        let mut url = Url::parse(base_url).ok()?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return None;
        }
        url.set_path(&format!(
            "{}/management.html",
            url.path().trim_end_matches('/')
        ));
        url.set_query(None);
        url.set_fragment(Some("/oauth"));
        Some(Self {
            url: url.into(),
            button: hint.to_owned(),
        })
    }
}

/// State for `/accounts` without reading windows or computing their predictions.
#[derive(Debug, Default)]
pub struct SignIn {
    pub error: Option<SignInError>,
    pub reauthentication: Option<Reauthentication>,
}

impl SignIn {
    pub async fn list(
        database: &Database,
        only: Option<Id<Account>>,
    ) -> Result<HashMap<Id<Account>, Self>, DatabaseError> {
        let rows = sqlx::query_as::<_, (Id<Account>, Option<SignInError>, String, String)>(
            "SELECT credential_state.account_id, credential_state.sign_in_error, \
             source.base_url, account.provider FROM credential_state \
             JOIN account ON account.account_id = credential_state.account_id \
             JOIN source ON source.source_id = account.source_id \
             WHERE (? IS NULL OR credential_state.account_id = ?)",
        )
        .bind(only)
        .bind(only)
        .fetch_all(&database.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, error, base_url, provider)| {
                let reauthentication = if error.is_some() {
                    Reauthentication::new(&base_url, &provider)
                } else {
                    None
                };
                (
                    id,
                    Self {
                        error,
                        reauthentication,
                    },
                )
            })
            .collect())
    }
    /// The credentials CLIProxy still lists and has not turned off whose last quota read said
    /// the sign-in expired.
    pub async fn expired(database: &Database) -> Result<Vec<Id<Account>>, DatabaseError> {
        Ok(sqlx::query_scalar(
            "SELECT credential_state.account_id FROM credential_state \
             JOIN account ON account.account_id = credential_state.account_id \
             JOIN source ON source.source_id = account.source_id \
             WHERE credential_state.sign_in_error IS NOT NULL AND credential_state.removed_at IS NULL \
             AND credential_state.disabled = 0 AND credential_state.hard_refreshable = 1 \
             AND source.enabled = 1",
        )
        .fetch_all(&database.pool)
        .await?)
    }
}
