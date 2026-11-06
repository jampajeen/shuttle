//! Shared helpers for unit tests.

use std::sync::{Mutex, MutexGuard, PoisonError};

static HOME_ENV_LOCK: Mutex<()> = Mutex::new(());

/// Serializes tests that change `HOME` with tests that depend on it, such as
/// anything calling `config::snapshot`, which reads `~/.ssh/config`.
pub fn lock_home_env() -> MutexGuard<'static, ()> {
    HOME_ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}
