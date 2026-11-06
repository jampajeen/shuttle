//! Saved passwords for launched commands (`password_ref`).
//!
//! Shuttle's menu app never reads a saved password. When a host sets
//! `password_ref`, the launched shell command is wrapped so ssh runs with
//! `SSH_ASKPASS` set to the Shuttle executable and `SSH_ASKPASS_REQUIRE=force`.
//! When ssh needs the password it starts Shuttle in askpass mode (see
//! `crate::askpass`), which reads it from the macOS login Keychain and writes
//! it to ssh over a pipe. The password never appears on a command line, in an
//! environment variable, in terminal scrollback, or in shell history.
//! See `docs/ADR/0005-keychain-ssh-passwords.md`.

use super::LaunchError;
use crate::config::model::CommandHost;
use std::path::Path;

/// Keychain service name of the generic password items Shuttle looks up.
pub const KEYCHAIN_SERVICE: &str = "Shuttle";
/// Tells the Shuttle executable to run as ssh's askpass helper for this ref.
pub const REF_ENV: &str = "SHUTTLE_PASSWORD_REF";
/// Per-launch directory where askpass remembers which ssh processes it answered.
pub const STATE_ENV: &str = "SHUTTLE_ASKPASS_STATE";

const KEYCHAIN_SCHEME: &str = "keychain:";

/// Runs inside `/bin/sh -c` with `$1` = askpass program (the Shuttle
/// executable), `$2` = canonical `password_ref`, `$3` = the host's command.
/// It exports the askpass settings for the command only, keeps a private
/// state directory for the launch, and removes it when the command ends.
/// It is one line so every terminal backend can type it as one command.
const WRAPPER_SCRIPT: &str = concat!(
    "d=$(mktemp -d \"${TMPDIR:-/tmp}/shuttle-askpass.XXXXXX\") || exit 1; ",
    "trap 'rm -rf \"$d\"' EXIT; trap 'exit 129' HUP; trap 'exit 130' INT; trap 'exit 143' TERM; ",
    "SSH_ASKPASS=$1 SSH_ASKPASS_REQUIRE=force SHUTTLE_PASSWORD_REF=$2 SHUTTLE_ASKPASS_STATE=$d; ",
    "export SSH_ASKPASS SSH_ASKPASS_REQUIRE SHUTTLE_PASSWORD_REF SHUTTLE_ASKPASS_STATE; ",
    "/bin/sh -c \"$3\"",
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasswordRef {
    /// A generic password in the login Keychain with service
    /// [`KEYCHAIN_SERVICE`] and this account name.
    Keychain { account: String },
}

impl PasswordRef {
    /// Parses a config value such as `keychain:prod`.
    pub fn parse(value: &str) -> Result<Self, LaunchError> {
        let account = value
            .trim()
            .strip_prefix(KEYCHAIN_SCHEME)
            .ok_or(LaunchError::PasswordRef)?
            .trim();
        if account.is_empty() || account.chars().any(char::is_control) {
            return Err(LaunchError::PasswordRef);
        }
        Ok(Self::Keychain {
            account: account.to_string(),
        })
    }

    /// The canonical config value, used to carry the reference to execution.
    pub fn to_config_value(&self) -> String {
        match self {
            Self::Keychain { account } => format!("{KEYCHAIN_SCHEME}{account}"),
        }
    }
}

/// What a host's `password_ref` asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasswordSetting {
    /// Use this saved password.
    Saved(PasswordRef),
    /// `password_ref` is empty: save a password first (see `crate::password_setup`).
    NeedsSetup,
}

/// Validates the host's `password_ref`, if any.
pub fn from_host(host: &CommandHost) -> Result<Option<PasswordSetting>, LaunchError> {
    match host.password_ref.as_deref() {
        None => Ok(None),
        Some(value) if value.trim().is_empty() => Ok(Some(PasswordSetting::NeedsSetup)),
        Some(value) => {
            PasswordRef::parse(value).map(|password| Some(PasswordSetting::Saved(password)))
        }
    }
}

/// The command that runs Shuttle's password setup in the terminal for a host
/// with an empty `password_ref`. `menu_path` is the host's place in the menu,
/// ending with its own title; `host_name` is its raw `name` from the config.
pub fn setup_command(
    shuttle_program: &Path,
    host_name: &str,
    command: &str,
    menu_path: &[String],
) -> String {
    let mut parts = vec![
        shell_quote(&shuttle_program.to_string_lossy()),
        SETUP_FLAG.to_string(),
        shell_quote(host_name),
        shell_quote(command),
    ];
    parts.extend(menu_path.iter().map(|part| shell_quote(part)));
    parts.join(" ")
}

/// First argument that starts the Shuttle executable in password setup mode.
pub const SETUP_FLAG: &str = "--setup-password";

/// Wraps `command` so ssh asks `askpass_program` (the Shuttle executable)
/// for the saved password.
pub fn wrap_command(command: &str, password: &PasswordRef, askpass_program: &Path) -> String {
    format!(
        "/bin/sh -c {} shuttle {} {} {}",
        shell_quote(WRAPPER_SCRIPT),
        shell_quote(&askpass_program.to_string_lossy()),
        shell_quote(&password.to_config_value()),
        shell_quote(command),
    )
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn host(password_ref: Option<&str>) -> CommandHost {
        CommandHost {
            cmd: "ssh prod".into(),
            name: "Prod".into(),
            in_terminal: None,
            theme: None,
            title: None,
            backend: None,
            strategy: None,
            password_ref: password_ref.map(Into::into),
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn parses_keychain_refs() {
        assert_eq!(
            PasswordRef::parse(" keychain: prod ").unwrap(),
            PasswordRef::Keychain {
                account: "prod".into()
            }
        );
        assert_eq!(
            PasswordRef::parse("keychain:ops@db-1")
                .unwrap()
                .to_config_value(),
            "keychain:ops@db-1"
        );
    }

    #[test]
    fn rejects_invalid_refs() {
        for value in [
            "",
            "prod",
            "keychain:",
            "keychain:  ",
            "op://Servers/prod",
            "keychain:a\nb",
        ] {
            assert_eq!(
                PasswordRef::parse(value).unwrap_err(),
                LaunchError::PasswordRef,
                "{value:?} should be rejected"
            );
        }
    }

    #[test]
    fn reads_optional_host_ref() {
        assert_eq!(from_host(&host(None)).unwrap(), None);
        assert!(matches!(
            from_host(&host(Some("keychain:prod"))).unwrap(),
            Some(PasswordSetting::Saved(_))
        ));
        for empty in ["", "  "] {
            assert_eq!(
                from_host(&host(Some(empty))).unwrap(),
                Some(PasswordSetting::NeedsSetup)
            );
        }
        assert!(from_host(&host(Some("nope"))).is_err());
    }

    #[test]
    fn setup_command_quotes_every_argument() {
        let command = setup_command(
            Path::new("/Apps/My Shuttle.app/Contents/MacOS/shuttle-rs"),
            "[aaa]Local's Server",
            "ssh user@10.0.0.1",
            &["EL".into(), "Local".into(), "Local's Server".into()],
        );
        assert_eq!(
            command,
            "'/Apps/My Shuttle.app/Contents/MacOS/shuttle-rs' --setup-password '[aaa]Local'\\''s Server' 'ssh user@10.0.0.1' 'EL' 'Local' 'Local'\\''s Server'"
        );
    }

    #[test]
    fn wrapped_command_is_one_line_and_names_no_secret() {
        let wrapped = wrap_command(
            "ssh prod\necho done",
            &PasswordRef::Keychain {
                account: "prod".into(),
            },
            Path::new("/Applications/My Apps/Shuttle.app/Contents/MacOS/shuttle-rs"),
        );
        assert!(wrapped.starts_with("/bin/sh -c '"));
        assert!(!WRAPPER_SCRIPT.contains('\n'));
        assert!(WRAPPER_SCRIPT.contains(REF_ENV) && WRAPPER_SCRIPT.contains(STATE_ENV));
        assert!(wrapped.ends_with(
            " '/Applications/My Apps/Shuttle.app/Contents/MacOS/shuttle-rs' 'keychain:prod' 'ssh prod\necho done'"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn wrapper_exports_askpass_settings_for_the_command_only() {
        let dir = tempfile::tempdir().unwrap();
        let state_file = dir.path().join("state-path");
        let command = format!(
            r#"printf '%s|%s|%s|%s\n' "$SSH_ASKPASS" "$SSH_ASKPASS_REQUIRE" "$SHUTTLE_PASSWORD_REF" "it's"; test -d "$SHUTTLE_ASKPASS_STATE" && printf '%s' "$SHUTTLE_ASKPASS_STATE" > '{}'; exit 3"#,
            state_file.display()
        );
        let wrapped = wrap_command(
            &command,
            &PasswordRef::Keychain {
                account: "o'brien".into(),
            },
            Path::new("/opt/Shuttle's/shuttle-rs"),
        );
        let output = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(&wrapped)
            .env("TMPDIR", dir.path())
            .output()
            .unwrap();

        assert_eq!(output.status.code(), Some(3), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "/opt/Shuttle's/shuttle-rs|force|keychain:o'brien|it's\n"
        );
        let state_dir = std::fs::read_to_string(&state_file).unwrap();
        assert!(state_dir.contains("shuttle-askpass."));
        assert!(
            !Path::new(&state_dir).exists(),
            "state directory must be removed"
        );
    }
}
