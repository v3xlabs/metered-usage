//! Clears gateway scheduling cooldowns without spending a provider quota reset.

use crate::app::AppState;
use crate::prelude::*;
use crate::quota::management::Management;
use crate::quota::{Refresh, RefreshRequestError, SourceFailure, sources, store};

/// An explicit human action. No provider call or automatic retry is made here.
pub async fn reset(state: &AppState, only: Id<Account>) -> Result<Refresh, RefreshRequestError> {
    let requested_at = jiff::Timestamp::now();
    let source_id = Account::load(&state.database, only)
        .await?
        .ok_or(RefreshRequestError::UnknownAccount)?
        .summary
        .source_id;
    let _guard = state.manual_reset_lock.lock().await;
    let mut outcome = Refresh::default();
    let mut found = false;
    for (source, config) in sources(state, Some(source_id)).await? {
        let management = Management::new(&state.http, config);
        let listing = match management.auth_files().await {
            Ok(listing) => listing,
            Err(error) => {
                outcome.unreachable.push(SourceFailure {
                    source: source.name,
                    error,
                });
                continue;
            }
        };
        let targets = store(state, source.id, &listing).await?;
        for (account_id, file, _) in targets {
            if only != account_id || file.entry.disabled {
                continue;
            }
            found = true;
            if let Some(reused) = crate::quota::reuse::Reuse::recent(
                &state.database,
                "cooldown_reset",
                account_id,
                requested_at,
            )
            .await?
            {
                if reused.failed {
                    outcome.failed += 1;
                } else {
                    outcome.refreshed += 1;
                }
                outcome.shared_actions.push(reused.action);
                continue;
            }
            let auth_index = file.entry.auth_index.as_deref().unwrap_or_default();
            match management.reset_cooldown(auth_index).await {
                Ok(()) => outcome.refreshed += 1,
                Err(error) => {
                    tracing::warn!(%account_id, %error, "gateway cooldown reset was not confirmed");
                    outcome.failed += 1;
                }
            }
        }
        // Read the gateway again rather than assuming a reset cleared every status field.
        // Provider quota windows are kept exactly as last reported.
        match management.auth_files().await {
            Ok(listing) => {
                store(state, source.id, &listing).await?;
            }
            Err(error) => outcome.unreachable.push(SourceFailure {
                source: source.name,
                error,
            }),
        }
    }
    if !found && outcome.unreachable.is_empty() {
        return Err(RefreshRequestError::NotListed);
    }
    Ok(outcome)
}
