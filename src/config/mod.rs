pub mod model;
pub mod ssh;

use model::Config;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use thiserror::Error;

const DEFAULT_CONFIG: &str = include_str!("../../resources/shuttle.default.json");

/// XDG-style default config directory: ~/.config/shuttle/
pub const CONFIG_DIR_NAME: &str = "shuttle";
/// Default config file name inside the config dir.
pub const CONFIG_FILE_NAME: &str = "config.json";
/// Legacy home-root config path kept for migration.
pub const LEGACY_CONFIG_FILE: &str = ".shuttle.json";

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not determine the current home directory")]
    MissingHome,
    #[error("I/O error for {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("invalid JSON in {path}: {source}. Validate with `python3 -m json.tool {path}` and fix the syntax before reloading Shuttle.")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("invalid YAML in {path}: {source}. Validate with `ruby -e 'require \"yaml\"; YAML.load_file(ARGV[0])' {path}` or another YAML linter before reloading Shuttle.")]
    Yaml {
        path: PathBuf,
        source: yaml_serde::Error,
    },
    #[error("unsupported config extension for {path}. Use .json, .yaml, or .yml.")]
    UnsupportedExtension { path: PathBuf },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigFormat {
    Json,
    Yaml,
}

impl ConfigFormat {
    fn from_path(path: &Path) -> Result<Self, ConfigError> {
        match path.extension().and_then(|extension| extension.to_str()) {
            Some(extension) if extension.eq_ignore_ascii_case("json") => Ok(Self::Json),
            Some(extension)
                if extension.eq_ignore_ascii_case("yaml")
                    || extension.eq_ignore_ascii_case("yml") =>
            {
                Ok(Self::Yaml)
            }
            _ => Err(ConfigError::UnsupportedExtension {
                path: path.to_path_buf(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPaths {
    pub main: PathBuf,
    pub alt: Option<PathBuf>,
    pub used_main_override: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReloadSnapshot {
    pub main_config: Option<SystemTime>,
    pub alt_config: Option<SystemTime>,
    pub ssh_system: Option<SystemTime>,
    pub ssh_user: Option<SystemTime>,
}

pub fn discover_paths() -> Result<ConfigPaths, ConfigError> {
    discover_paths_in(home_dir()?)
}

pub fn discover_paths_in(home: PathBuf) -> Result<ConfigPaths, ConfigError> {
    // Explicit override via ~/.shuttle.path wins over everything.
    let override_file = home.join(".shuttle.path");
    if override_file.exists() {
        let main = read_config_path_file(&override_file)?;
        let alt = resolve_alt_path(&home)?;
        return Ok(ConfigPaths {
            main,
            alt,
            used_main_override: true,
        });
    }

    let config_dir = home.join(".config").join(CONFIG_DIR_NAME);
    let preferred = config_dir.join(CONFIG_FILE_NAME);

    // Legacy location: ~/.shuttle.json (migrate automatically if found)
    let legacy = home.join(LEGACY_CONFIG_FILE);

    let main = if let Some(existing) = first_existing(&standard_main_candidates(&config_dir)) {
        existing
    } else if legacy.exists() {
        // Migrate: copy legacy → preferred location and keep using it
        if let Ok(()) = fs::create_dir_all(&config_dir) {
            let _ = fs::copy(&legacy, &preferred);
        }
        if preferred.exists() {
            preferred
        } else {
            legacy
        }
    } else {
        // Neither exists — first run, use JSON default location
        preferred
    };

    let alt = resolve_alt_path(&home)?;
    Ok(ConfigPaths {
        main,
        alt,
        used_main_override: false,
    })
}

/// Resolve the alternate config path.
fn resolve_alt_path(home: &Path) -> Result<Option<PathBuf>, ConfigError> {
    let alt_override = home.join(".shuttle-alt.path");
    if alt_override.exists() {
        read_config_path_file(&alt_override).map(Some)
    } else {
        let config_dir = home.join(".config").join(CONFIG_DIR_NAME);
        let legacy_alt = home.join(".shuttle-alt.json");
        Ok(first_existing(&standard_alt_candidates(&config_dir))
            .or_else(|| legacy_alt.exists().then_some(legacy_alt)))
    }
}

fn standard_main_candidates(config_dir: &Path) -> [PathBuf; 3] {
    [
        config_dir.join("config.yaml"),
        config_dir.join("config.yml"),
        config_dir.join(CONFIG_FILE_NAME),
    ]
}

fn standard_alt_candidates(config_dir: &Path) -> [PathBuf; 3] {
    [
        config_dir.join("alt.yaml"),
        config_dir.join("alt.yml"),
        config_dir.join("alt.json"),
    ]
}

fn first_existing(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|path| path.exists()).cloned()
}

/// Write the bundled default config on first run.
pub fn ensure_default_config(paths: &ConfigPaths) -> Result<(), ConfigError> {
    if paths.used_main_override || paths.main.exists() {
        return Ok(());
    }
    // Make sure the directory exists.
    if let Some(parent) = paths.main.parent() {
        fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(&paths.main, DEFAULT_CONFIG).map_err(|source| ConfigError::Io {
        path: paths.main.clone(),
        source,
    })
}

pub fn load_config(path: &Path) -> Result<Config, ConfigError> {
    let bytes = fs::read(path).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    match ConfigFormat::from_path(path)? {
        ConfigFormat::Json => serde_json::from_slice(&bytes).map_err(|source| ConfigError::Json {
            path: path.to_path_buf(),
            source,
        }),
        ConfigFormat::Yaml => yaml_serde::from_slice(&bytes).map_err(|source| ConfigError::Yaml {
            path: path.to_path_buf(),
            source,
        }),
    }
}

pub fn load_merged(paths: &ConfigPaths) -> Result<Config, ConfigError> {
    let mut config = load_config(&paths.main)?;
    if let Some(alt) = &paths.alt {
        let mut alt_config = load_config(alt)?;
        config.hosts.append(&mut alt_config.hosts);
    }
    Ok(config)
}

pub fn snapshot(paths: &ConfigPaths) -> ReloadSnapshot {
    ReloadSnapshot {
        main_config: modified(&paths.main),
        alt_config: paths.alt.as_deref().and_then(modified),
        ssh_system: modified(Path::new("/etc/ssh/ssh_config")),
        ssh_user: home_dir()
            .ok()
            .and_then(|home| modified(&home.join(".ssh/config"))),
    }
}

pub fn needs_reload(old: &ReloadSnapshot, new: &ReloadSnapshot) -> bool {
    new.main_config != old.main_config
        || new.alt_config != old.alt_config
        || new.ssh_system != old.ssh_system
        || new.ssh_user != old.ssh_user
}

fn read_config_path_file(path: &Path) -> Result<PathBuf, ConfigError> {
    let config_path = read_path_file(path)?;
    ConfigFormat::from_path(&config_path)?;
    Ok(config_path)
}

fn read_path_file(path: &Path) -> Result<PathBuf, ConfigError> {
    let contents = fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(PathBuf::from(contents.trim()))
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Merge SSH config hosts into `config.hosts` only when `show_ssh_config_hosts` is enabled.
/// This is the single authoritative place for the check.
pub fn apply_ssh_hosts(config: &mut Config) {
    if !config.show_ssh_config_hosts.unwrap_or(true) {
        return;
    }
    let mut ssh_hosts = std::collections::BTreeMap::new();
    if let Ok(hosts) = ssh::parse_file(Path::new("/etc/ssh/ssh_config")) {
        ssh_hosts.extend(hosts);
    }
    if let Ok(home) = home_dir() {
        if let Ok(hosts) = ssh::parse_file(&home.join(".ssh/config")) {
            ssh_hosts.extend(hosts);
        }
    }
    let ignore_hosts = config.ssh_config_ignore_hosts.clone();
    let ignore_keywords = config.ssh_config_ignore_keywords.clone();
    ssh::merge_hosts(
        &mut config.hosts,
        &ssh_hosts,
        &ignore_hosts,
        &ignore_keywords,
    );
}

fn home_dir() -> Result<PathBuf, ConfigError> {
    dirs::home_dir().ok_or(ConfigError::MissingHome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::HostEntry;

    const MINIMAL_JSON: &str = r#"{"hosts":[{"cmd":"ssh prod","name":"Prod"}]}"#;
    const MINIMAL_YAML: &str = "hosts:\n  - cmd: ssh prod\n    name: Prod\n";

    #[test]
    fn prefers_xdg_config_path_on_first_run() {
        let temp = tempfile::tempdir().unwrap();
        let paths = discover_paths_in(temp.path().to_path_buf()).unwrap();
        assert_eq!(
            paths.main,
            temp.path().join(".config/shuttle/config.json"),
            "default should be ~/.config/shuttle/config.json"
        );
        assert!(!paths.used_main_override);
    }

    #[test]
    fn override_path_file_wins_over_everything() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join(".shuttle.path"), "/tmp/custom.json\n").unwrap();
        let paths = discover_paths_in(temp.path().to_path_buf()).unwrap();
        assert_eq!(paths.main, PathBuf::from("/tmp/custom.json"));
        assert!(paths.used_main_override);
    }

    #[test]
    fn migrates_legacy_config_to_xdg_on_first_run() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join(".shuttle.json");
        fs::write(&legacy, DEFAULT_CONFIG).unwrap();
        let paths = discover_paths_in(temp.path().to_path_buf()).unwrap();
        let preferred = temp.path().join(".config/shuttle/config.json");
        // Should point to the preferred location after migration
        assert_eq!(paths.main, preferred);
        assert!(
            preferred.exists(),
            "config should be copied to XDG location"
        );
    }

    #[test]
    fn uses_xdg_when_it_already_exists() {
        let temp = tempfile::tempdir().unwrap();
        let preferred = temp.path().join(".config/shuttle/config.json");
        fs::create_dir_all(preferred.parent().unwrap()).unwrap();
        fs::write(&preferred, DEFAULT_CONFIG).unwrap();
        let paths = discover_paths_in(temp.path().to_path_buf()).unwrap();
        assert_eq!(paths.main, preferred);
    }

    #[test]
    fn creates_config_dir_on_first_run() {
        let temp = tempfile::tempdir().unwrap();
        let paths = discover_paths_in(temp.path().to_path_buf()).unwrap();
        ensure_default_config(&paths).unwrap();
        assert!(paths.main.exists());
        load_config(&paths.main).unwrap();
    }

    #[test]
    fn alt_config_found_at_xdg_location() {
        let temp = tempfile::tempdir().unwrap();
        let alt = temp.path().join(".config/shuttle/alt.json");
        fs::create_dir_all(alt.parent().unwrap()).unwrap();
        fs::write(&alt, "{}").unwrap();
        let paths = discover_paths_in(temp.path().to_path_buf()).unwrap();
        assert_eq!(paths.alt, Some(alt));
    }

    #[test]
    fn loads_default_config_json() {
        let config: Config = serde_json::from_str(DEFAULT_CONFIG).unwrap();
        assert_eq!(config.terminal.as_deref(), Some("Terminal.app"));
        assert!(!config.hosts.is_empty());
    }

    #[test]
    fn load_config_loads_valid_json() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        fs::write(&path, MINIMAL_JSON).unwrap();
        let config = load_config(&path).unwrap();
        assert_eq!(config.hosts.len(), 1);
    }

    #[test]
    fn load_config_loads_valid_yaml() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.yaml");
        fs::write(&path, MINIMAL_YAML).unwrap();
        let config = load_config(&path).unwrap();
        assert_eq!(config.hosts.len(), 1);
    }

    #[test]
    fn command_hosts_do_not_require_title() {
        let config: Config = serde_json::from_str(
            r#"{
                "hosts": [
                    {
                        "IONOS": [
                            {
                                "K8S": [
                                    {
                                        "cmd": "ssh root@85.215.50.134",
                                        "inTerminal": "new",
                                        "name": "K8S Worker Node 1 (Server VPS L Linux)",
                                        "theme": "snazzy"
                                    }
                                ]
                            }
                        ]
                    }
                ]
            }"#,
        )
        .unwrap();
        let HostEntry::Menu(ionos) = &config.hosts[0] else {
            panic!("expected IONOS menu");
        };
        let HostEntry::Menu(k8s) = &ionos["IONOS"][0] else {
            panic!("expected K8S menu");
        };
        let HostEntry::Command(host) = &k8s["K8S"][0] else {
            panic!("expected command host");
        };
        assert_eq!(host.title, None);
    }

    #[test]
    fn command_hosts_read_password_ref_from_json_and_yaml() {
        let json: Config = serde_json::from_str(
            r#"{"hosts": [{"cmd": "ssh prod", "name": "Prod", "password_ref": "keychain:prod"}]}"#,
        )
        .unwrap();
        let yaml: Config = yaml_serde::from_str(
            "hosts:\n  - cmd: ssh prod\n    name: Prod\n    password_ref: keychain:prod\n",
        )
        .unwrap();
        for config in [json, yaml] {
            let HostEntry::Command(host) = &config.hosts[0] else {
                panic!("expected command host");
            };
            assert_eq!(host.password_ref.as_deref(), Some("keychain:prod"));
            assert!(host.extra.is_empty());
        }
    }

    #[test]
    fn load_config_loads_yml_as_yaml() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.yml");
        fs::write(&path, MINIMAL_YAML).unwrap();
        let config = load_config(&path).unwrap();
        assert_eq!(config.hosts.len(), 1);
    }

    #[test]
    fn load_config_loads_yaml_example_resource() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.yaml");
        fs::write(&path, include_str!("../../resources/shuttle.example.yaml")).unwrap();
        let config = load_config(&path).unwrap();
        assert_eq!(config.terminal.as_deref(), Some("Ghostty"));
        assert!(!config.hosts.is_empty());
    }

    #[test]
    fn invalid_json_returns_structured_error() {
        let temp = tempfile::tempdir().unwrap();
        let bad = temp.path().join("bad.json");
        fs::write(&bad, "{ nope").unwrap();
        assert!(matches!(load_config(&bad), Err(ConfigError::Json { .. })));
    }

    #[test]
    fn invalid_yaml_returns_structured_error() {
        let temp = tempfile::tempdir().unwrap();
        let bad = temp.path().join("bad.yaml");
        fs::write(&bad, "hosts:\n  - [nope\n").unwrap();
        assert!(matches!(load_config(&bad), Err(ConfigError::Yaml { .. })));
    }

    #[test]
    fn extensionless_path_returns_unsupported_extension() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config");
        fs::write(&path, MINIMAL_JSON).unwrap();
        assert!(matches!(
            load_config(&path),
            Err(ConfigError::UnsupportedExtension { .. })
        ));
    }

    #[test]
    fn unknown_extension_returns_unsupported_extension() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        fs::write(&path, MINIMAL_JSON).unwrap();
        assert!(matches!(
            load_config(&path),
            Err(ConfigError::UnsupportedExtension { .. })
        ));
    }

    #[test]
    fn config_yaml_beats_config_json() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join(".config/shuttle");
        fs::create_dir_all(&dir).unwrap();
        let yaml = dir.join("config.yaml");
        let json = dir.join("config.json");
        fs::write(&yaml, MINIMAL_YAML).unwrap();
        fs::write(&json, MINIMAL_JSON).unwrap();
        let paths = discover_paths_in(temp.path().to_path_buf()).unwrap();
        assert_eq!(paths.main, yaml);
    }

    #[test]
    fn config_yml_beats_config_json() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join(".config/shuttle");
        fs::create_dir_all(&dir).unwrap();
        let yml = dir.join("config.yml");
        let json = dir.join("config.json");
        fs::write(&yml, MINIMAL_YAML).unwrap();
        fs::write(&json, MINIMAL_JSON).unwrap();
        let paths = discover_paths_in(temp.path().to_path_buf()).unwrap();
        assert_eq!(paths.main, yml);
    }

    #[test]
    fn config_yaml_beats_config_yml() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join(".config/shuttle");
        fs::create_dir_all(&dir).unwrap();
        let yaml = dir.join("config.yaml");
        let yml = dir.join("config.yml");
        fs::write(&yaml, MINIMAL_YAML).unwrap();
        fs::write(&yml, MINIMAL_YAML).unwrap();
        let paths = discover_paths_in(temp.path().to_path_buf()).unwrap();
        assert_eq!(paths.main, yaml);
    }

    #[test]
    fn alt_yaml_beats_alt_json() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join(".config/shuttle");
        fs::create_dir_all(&dir).unwrap();
        let yaml = dir.join("alt.yaml");
        let json = dir.join("alt.json");
        fs::write(&yaml, MINIMAL_YAML).unwrap();
        fs::write(&json, MINIMAL_JSON).unwrap();
        let paths = discover_paths_in(temp.path().to_path_buf()).unwrap();
        assert_eq!(paths.alt, Some(yaml));
    }

    #[test]
    fn alt_yml_beats_alt_json() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join(".config/shuttle");
        fs::create_dir_all(&dir).unwrap();
        let yml = dir.join("alt.yml");
        let json = dir.join("alt.json");
        fs::write(&yml, MINIMAL_YAML).unwrap();
        fs::write(&json, MINIMAL_JSON).unwrap();
        let paths = discover_paths_in(temp.path().to_path_buf()).unwrap();
        assert_eq!(paths.alt, Some(yml));
    }

    #[test]
    fn invalid_main_override_extension_errors() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join(".shuttle.path"), "/tmp/custom.toml\n").unwrap();
        assert!(matches!(
            discover_paths_in(temp.path().to_path_buf()),
            Err(ConfigError::UnsupportedExtension { .. })
        ));
    }

    #[test]
    fn invalid_alt_override_extension_errors() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join(".shuttle-alt.path"), "/tmp/custom.toml\n").unwrap();
        assert!(matches!(
            discover_paths_in(temp.path().to_path_buf()),
            Err(ConfigError::UnsupportedExtension { .. })
        ));
    }

    #[test]
    fn load_merged_surfaces_alt_errors() {
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("config.json");
        let alt = temp.path().join("alt.json");
        fs::write(&main, MINIMAL_JSON).unwrap();
        fs::write(&alt, "{ nope").unwrap();
        let paths = ConfigPaths {
            main,
            alt: Some(alt),
            used_main_override: false,
        };
        assert!(matches!(load_merged(&paths), Err(ConfigError::Json { .. })));
    }

    #[test]
    fn reload_detects_any_snapshot_change() {
        let old = ReloadSnapshot {
            main_config: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(20)),
            alt_config: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(10)),
            ssh_system: None,
            ssh_user: None,
        };
        let deleted = ReloadSnapshot {
            main_config: None,
            ..old.clone()
        };
        assert!(needs_reload(&old, &deleted));

        let older = ReloadSnapshot {
            alt_config: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(5)),
            ..old.clone()
        };
        assert!(needs_reload(&old, &older));
    }
}
