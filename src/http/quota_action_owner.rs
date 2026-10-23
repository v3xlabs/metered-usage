//! Live action ownership, independent of caller connections and elapsed wall time.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

/// Requires one service instance per SQLite database. A new boot ID means the
/// previous process was replaced, not another live instance in a rolling deploy.
/// These worker leases do not coordinate multiple processes.
pub struct ActionOwners {
    boot: String,
    leases: Mutex<HashMap<i64, Weak<()>>>,
}

/// Dropping the worker releases ownership even after a persistence error or panic.
pub struct ActionLease {
    _token: Arc<()>,
}

impl Default for ActionOwners {
    fn default() -> Self {
        Self {
            boot: format!("{:032x}", rand::random::<u128>()),
            leases: Mutex::new(HashMap::new()),
        }
    }
}

impl ActionOwners {
    pub fn boot(&self) -> &str {
        &self.boot
    }

    pub fn claim(&self, action_id: i64) -> ActionLease {
        let token = Arc::new(());
        self.leases
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(action_id, Arc::downgrade(&token));
        ActionLease { _token: token }
    }

    pub fn abandoned(&self) -> Vec<i64> {
        self.leases
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter_map(|(id, lease)| (lease.strong_count() == 0).then_some(*id))
            .collect()
    }
    pub fn forget(&self, ids: &[i64]) {
        let mut leases = self
            .leases
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for id in ids {
            leases.remove(id);
        }
    }
}
