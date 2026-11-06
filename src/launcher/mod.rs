pub mod cmux;
pub mod ghostty;
pub mod iterm;
pub mod password;
pub mod terminal_app;
pub mod url;
pub mod virtual_screen;

use crate::config::model::{CommandHost, Config};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchTarget {
    New,
    Tab,
    Current,
    Virtual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ITermVersion {
    Stable,
    Nightly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    TerminalApp,
    ITerm { version: ITermVersion },
    GhosttyOpen,
    GhosttyAppleScript,
    CmuxCli,
    CmuxSocket,
    Screen,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchStrategy {
    Default,
    Workspace,
    Socket,
    AppleScript,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchKind {
    Url(String),
    Terminal(LaunchRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest {
    pub command: String,
    pub title: String,
    pub theme_or_profile: String,
    pub target: LaunchTarget,
    pub backend: Backend,
    pub strategy: LaunchStrategy,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LaunchError {
    #[error(
        "'{0}' is not a valid value for inTerminal. Use 'new', 'tab', 'current', or 'virtual'."
    )]
    Target(String),
    #[error("'{0}' is not a valid value for iTerm_version. Use 'stable' or 'nightly'.")]
    ITermVersion(String),
    #[error("'{0}' is not a supported backend. Use terminal-app, iterm-stable, iterm-nightly, ghostty-open, ghostty-applescript, cmux-cli, cmux-socket, or screen.")]
    Backend(String),
    #[error("'{0}' is not a supported strategy. Use default, workspace, socket, or applescript.")]
    Strategy(String),
    // Never echo the value: someone may have put the password itself here.
    #[error("invalid password_ref. Use 'keychain:<account>' with the Keychain account name, not the password.")]
    PasswordRef,
}

pub fn normalize(
    config: &Config,
    host: &CommandHost,
    menu_title: &str,
) -> Result<LaunchKind, LaunchError> {
    let target = normalize_target(host.in_terminal.as_deref(), config.open_in.as_deref())?;
    if target != LaunchTarget::Virtual && is_url(&host.cmd) {
        return Ok(LaunchKind::Url(host.cmd.clone()));
    }

    let backend = resolve_backend(config, host, &target)?;
    let strategy = resolve_strategy(host.strategy.as_deref().or(config.strategy.as_deref()))?;

    Ok(LaunchKind::Terminal(LaunchRequest {
        command: host.cmd.clone(),
        title: host.title.clone().unwrap_or_else(|| menu_title.to_string()),
        theme_or_profile: host
            .theme
            .clone()
            .unwrap_or_else(|| default_theme(config, &backend)),
        target,
        backend,
        strategy,
    }))
}

fn normalize_target(
    host_target: Option<&str>,
    config_target: Option<&str>,
) -> Result<LaunchTarget, LaunchError> {
    match host_target
        .or(config_target)
        .unwrap_or("tab")
        .to_lowercase()
        .as_str()
    {
        "new" => Ok(LaunchTarget::New),
        "current" => Ok(LaunchTarget::Current),
        "tab" => Ok(LaunchTarget::Tab),
        "virtual" => Ok(LaunchTarget::Virtual),
        invalid if host_target.is_some() => Err(LaunchError::Target(invalid.to_string())),
        _ => Ok(LaunchTarget::Tab),
    }
}

fn normalize_iterm_version(version: Option<&str>) -> Result<ITermVersion, LaunchError> {
    match version.unwrap_or("stable").to_lowercase().as_str() {
        "stable" => Ok(ITermVersion::Stable),
        "nightly" => Ok(ITermVersion::Nightly),
        invalid => Err(LaunchError::ITermVersion(invalid.to_string())),
    }
}

fn resolve_backend(
    config: &Config,
    host: &CommandHost,
    target: &LaunchTarget,
) -> Result<Backend, LaunchError> {
    if *target == LaunchTarget::Virtual {
        return Ok(Backend::Screen);
    }

    if let Some(backend) = host.backend.as_deref().or(config.backend.as_deref()) {
        return match backend.to_lowercase().as_str() {
            "terminal" | "terminal.app" | "terminal-app" => Ok(Backend::TerminalApp),
            "iterm" | "iterm-stable" => Ok(Backend::ITerm {
                version: ITermVersion::Stable,
            }),
            "iterm-nightly" => Ok(Backend::ITerm {
                version: ITermVersion::Nightly,
            }),
            "ghostty" | "ghostty-applescript" => Ok(Backend::GhosttyAppleScript),
            "ghostty-open" => Ok(Backend::GhosttyOpen),
            "cmux" | "cmux-cli" => Ok(Backend::CmuxCli),
            "cmux-socket" => Ok(Backend::CmuxSocket),
            "screen" | "virtual" => Ok(Backend::Screen),
            invalid => Err(LaunchError::Backend(invalid.to_string())),
        };
    }

    let terminal = config
        .terminal
        .as_deref()
        .unwrap_or("Terminal.app")
        .to_lowercase();
    if terminal.contains("ghostty") {
        Ok(Backend::GhosttyAppleScript)
    } else if terminal.contains("cmux") {
        Ok(Backend::CmuxCli)
    } else if terminal.contains("iterm") {
        Ok(Backend::ITerm {
            version: normalize_iterm_version(config.iterm_version.as_deref())?,
        })
    } else {
        Ok(Backend::TerminalApp)
    }
}

fn resolve_strategy(strategy: Option<&str>) -> Result<LaunchStrategy, LaunchError> {
    match strategy.unwrap_or("default").to_lowercase().as_str() {
        "default" => Ok(LaunchStrategy::Default),
        "workspace" => Ok(LaunchStrategy::Workspace),
        "socket" => Ok(LaunchStrategy::Socket),
        "applescript" | "apple-script" => Ok(LaunchStrategy::AppleScript),
        invalid => Err(LaunchError::Strategy(invalid.to_string())),
    }
}

fn default_theme(config: &Config, backend: &Backend) -> String {
    config
        .default_theme
        .clone()
        .unwrap_or_else(|| match backend {
            Backend::ITerm { .. } => "Default".into(),
            Backend::TerminalApp
            | Backend::Screen
            | Backend::GhosttyOpen
            | Backend::GhosttyAppleScript
            | Backend::CmuxCli
            | Backend::CmuxSocket => "basic".into(),
        })
}

fn is_url(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("ssh://")
        || lower.starts_with("file://")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn host() -> CommandHost {
        CommandHost {
            cmd: "echo hi".into(),
            name: "Hi".into(),
            in_terminal: None,
            theme: None,
            title: None,
            backend: None,
            strategy: None,
            password_ref: None,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn maps_legacy_terminal_app_defaults() {
        let config = Config {
            terminal: Some("Terminal.app".into()),
            open_in: Some("new".into()),
            ..Config::default()
        };
        assert_eq!(
            normalize(&config, &host(), "Menu").unwrap(),
            LaunchKind::Terminal(LaunchRequest {
                command: "echo hi".into(),
                title: "Menu".into(),
                theme_or_profile: "basic".into(),
                target: LaunchTarget::New,
                backend: Backend::TerminalApp,
                strategy: LaunchStrategy::Default
            })
        );
    }

    #[test]
    fn maps_iterm_nightly_and_theme() {
        let config = Config {
            terminal: Some("iTerm".into()),
            iterm_version: Some("nightly".into()),
            default_theme: Some("Homebrew".into()),
            ..Config::default()
        };
        let LaunchKind::Terminal(request) = normalize(&config, &host(), "Menu").unwrap() else {
            panic!("terminal expected")
        };
        assert_eq!(
            request.backend,
            Backend::ITerm {
                version: ITermVersion::Nightly
            }
        );
        assert_eq!(request.theme_or_profile, "Homebrew");
    }

    #[test]
    fn detects_urls_unless_virtual() {
        let mut host = host();
        host.cmd = "https://example.com".into();
        assert_eq!(
            normalize(&Config::default(), &host, "Web").unwrap(),
            LaunchKind::Url("https://example.com".into())
        );
        host.cmd = "HTTPS://example.com".into();
        assert_eq!(
            normalize(&Config::default(), &host, "Web").unwrap(),
            LaunchKind::Url("HTTPS://example.com".into())
        );
        host.in_terminal = Some("virtual".into());
        assert!(matches!(
            normalize(&Config::default(), &host, "Web").unwrap(),
            LaunchKind::Terminal(_)
        ));
    }

    #[test]
    fn resolves_top_level_and_host_backend_precedence() {
        let config = Config {
            backend: Some("ghostty-open".into()),
            ..Config::default()
        };
        let LaunchKind::Terminal(request) = normalize(&config, &host(), "Menu").unwrap() else {
            panic!("terminal expected")
        };
        assert_eq!(request.backend, Backend::GhosttyOpen);

        let mut host = host();
        host.backend = Some("cmux-cli".into());
        let LaunchKind::Terminal(request) = normalize(&config, &host, "Menu").unwrap() else {
            panic!("terminal expected")
        };
        assert_eq!(request.backend, Backend::CmuxCli);
    }

    #[test]
    fn rejects_invalid_backend_and_strategy() {
        let config = Config {
            backend: Some("nope".into()),
            ..Config::default()
        };
        assert_eq!(
            normalize(&config, &host(), "Menu"),
            Err(LaunchError::Backend("nope".into()))
        );

        let config = Config {
            strategy: Some("sideways".into()),
            ..Config::default()
        };
        assert_eq!(
            normalize(&config, &host(), "Menu"),
            Err(LaunchError::Strategy("sideways".into()))
        );
    }

    #[test]
    fn validates_bad_host_target() {
        let mut host = host();
        host.in_terminal = Some("sideways".into());
        assert_eq!(
            normalize(&Config::default(), &host, "Menu"),
            Err(LaunchError::Target("sideways".into()))
        );
    }

    #[test]
    fn terminal_key_ghostty_maps_to_ghostty_applescript() {
        let config = Config {
            terminal: Some("Ghostty".into()),
            ..Config::default()
        };
        let LaunchKind::Terminal(request) = normalize(&config, &host(), "Menu").unwrap() else {
            panic!("terminal expected")
        };
        assert_eq!(request.backend, Backend::GhosttyAppleScript);
    }

    #[test]
    fn terminal_key_cmux_maps_to_cmux_cli() {
        let config = Config {
            terminal: Some("cmux".into()),
            ..Config::default()
        };
        let LaunchKind::Terminal(request) = normalize(&config, &host(), "Menu").unwrap() else {
            panic!("terminal expected")
        };
        assert_eq!(request.backend, Backend::CmuxCli);
    }
}
