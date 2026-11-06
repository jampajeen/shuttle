//! Shuttle - macOS menu bar launcher for SSH sessions, commands, and URLs.
#[cfg(unix)]
mod askpass;
mod config;
mod launcher;
#[cfg(target_os = "macos")]
mod macos;
mod menu_model;
mod update;

#[cfg(not(test))]
fn main() {
    // ssh starts this executable as its SSH_ASKPASS helper for `password_ref`
    // hosts; answer it and exit before any AppKit setup.
    #[cfg(unix)]
    if let Some(code) = askpass::run_if_requested() {
        std::process::exit(code);
    }

    #[cfg(target_os = "macos")]
    macos::app::run();

    #[cfg(not(target_os = "macos"))]
    {
        if let Err(error) = build_menu_entries() {
            let _menu = menu_model::error_menu("Error parsing config");
            eprintln!("Shuttle config error: {error}");
        } else {
            println!("Config loaded OK (macOS app not running on this platform)");
        }
    }
}

/// Builds menu entries from the active config, returning them alongside the parsed config.
pub fn build_menu_entries(
) -> Result<(Vec<menu_model::MenuEntry>, config::model::Config), config::ConfigError> {
    build_menu_entries_from_paths(config::discover_paths()?)
}

fn build_menu_entries_from_paths(
    paths: config::ConfigPaths,
) -> Result<(Vec<menu_model::MenuEntry>, config::model::Config), config::ConfigError> {
    config::ensure_default_config(&paths)?;
    let before = config::snapshot(&paths);
    let mut config = config::load_merged(&paths)?;
    config::apply_ssh_hosts(&mut config);
    #[cfg(all(target_os = "macos", not(test)))]
    let _ = macos::login_item::set_launch_at_login(config.launch_at_login);
    let menu = menu_model::with_separators(menu_model::build(&config.hosts));
    let after = config::snapshot(&paths);
    let _needs_reload = config::needs_reload(&before, &after);
    Ok((menu, config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static HOME_ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn builds_menu_entries_from_discovered_paths() {
        let temp = tempfile::tempdir().unwrap();
        let paths = config::ConfigPaths {
            main: temp.path().join("config.json"),
            alt: None,
            used_main_override: false,
        };

        let (menu, config) = build_menu_entries_from_paths(paths).unwrap();

        assert!(!menu.is_empty());
        assert_eq!(config.terminal.as_deref(), Some("Terminal.app"));
    }

    #[test]
    fn builds_menu_entries_from_home_discovery() {
        let _guard = HOME_ENV_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        unsafe {
            std::env::set_var("HOME", temp.path());
        }

        let result = build_menu_entries();

        unsafe {
            if let Some(old_home) = old_home {
                std::env::set_var("HOME", old_home);
            } else {
                std::env::remove_var("HOME");
            }
        }

        let (menu, config) = result.unwrap();
        assert!(!menu.is_empty());
        assert_eq!(config.terminal.as_deref(), Some("Terminal.app"));
        assert!(temp.path().join(".config/shuttle/config.json").exists());
    }
}
