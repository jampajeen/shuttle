#![allow(deprecated)]

use crate::config::{self, ConfigPaths, ReloadSnapshot};
use cocoa::base::id;
use std::sync::Mutex;

pub struct AppState {
    pub paths: ConfigPaths,
    pub snapshot: ReloadSnapshot,
    #[cfg(not(test))]
    pub delegate: id,
    #[cfg(not(test))]
    pub status_item: id,
}

// id is a raw pointer; all access is on the main thread, guarded by Mutex.
unsafe impl Send for AppState {}
unsafe impl Sync for AppState {}

pub static APP_STATE: Mutex<Option<AppState>> = Mutex::new(None);

fn app_state() -> std::sync::MutexGuard<'static, Option<AppState>> {
    APP_STATE.lock().unwrap_or_else(|poisoned| {
        eprintln!("Shuttle: app state mutex was poisoned; continuing with stored state");
        poisoned.into_inner()
    })
}

pub fn init(paths: ConfigPaths, snapshot: ReloadSnapshot, delegate: id, status_item: id) {
    #[cfg(test)]
    let _ = (delegate, status_item);
    let mut guard = app_state();
    *guard = Some(AppState {
        paths,
        snapshot,
        #[cfg(not(test))]
        delegate,
        #[cfg(not(test))]
        status_item,
    });
}

/// Called by the menu delegate's `menuWillOpen:` on the main thread.
/// Reloads config and rebuilds the NSMenu only when a watched file changed.
#[cfg(not(test))]
pub fn reload_if_needed() {
    let needs = reload_needed();

    if !needs {
        return;
    }

    let (delegate, status_item, result) = {
        let guard = app_state();
        let Some(state) = guard.as_ref() else {
            return;
        };
        let delegate = state.delegate;
        let status_item = state.status_item;
        let result = config::load_merged(&state.paths).map(|mut cfg| {
            config::apply_ssh_hosts(&mut cfg);
            let entries = crate::menu_model::with_separators(crate::menu_model::build(&cfg.hosts));
            let new_snap = config::snapshot(&state.paths);
            (entries, cfg, new_snap)
        });
        (delegate, status_item, result)
    };

    match result {
        Ok((entries, cfg, new_snap)) => {
            {
                let mut guard = app_state();
                if let Some(state) = guard.as_mut() {
                    state.snapshot = new_snap;
                }
            }
            #[cfg(not(test))]
            unsafe {
                crate::macos::menu::rebuild_menu(status_item, &entries, &cfg, delegate);
            }
            #[cfg(test)]
            let _ = (status_item, entries, cfg, delegate);
        }
        Err(e) => eprintln!("Shuttle config reload error: {e}"),
    }
}

fn reload_needed() -> bool {
    let guard = app_state();
    guard.as_ref().is_some_and(|state| {
        let new_snap = config::snapshot(&state.paths);
        config::needs_reload(&state.snapshot, &new_snap)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cocoa::base::nil;
    use std::time::{Duration, SystemTime};

    #[test]
    fn init_stores_state_and_no_snapshot_change_skips_reload() {
        // Both tests here share APP_STATE, and snapshots read ~/.ssh/config via HOME.
        let _home = crate::test_support::lock_home_env();
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("config.json");
        std::fs::write(&main, "{}").unwrap();
        let paths = ConfigPaths {
            main: main.clone(),
            alt: None,
            used_main_override: false,
        };
        let snapshot = config::snapshot(&paths);
        init(paths.clone(), snapshot.clone(), nil, nil);

        assert!(!reload_needed());
        let guard = APP_STATE.lock().unwrap();
        let state = guard.as_ref().unwrap();
        assert_eq!(state.paths, paths);
        assert_eq!(state.snapshot, snapshot);
    }

    #[test]
    fn reload_needed_detects_changed_snapshot() {
        let _home = crate::test_support::lock_home_env();
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("config.json");
        std::fs::write(&main, "{}").unwrap();
        let paths = ConfigPaths {
            main,
            alt: None,
            used_main_override: false,
        };
        init(
            paths,
            ReloadSnapshot {
                main_config: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1)),
                ..ReloadSnapshot::default()
            },
            nil,
            nil,
        );

        assert!(reload_needed());
    }
}
