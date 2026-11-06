//! `SSH_ASKPASS` helper mode of the Shuttle executable.
//!
//! For hosts with `password_ref`, the launched command runs with
//! `SSH_ASKPASS` pointing at this executable and `SSH_ASKPASS_REQUIRE=force`
//! (see `launcher::password`). ssh then runs `shuttle-rs "<prompt>"` for each
//! question it would otherwise ask on the terminal. Shuttle answers the first
//! password prompt of each ssh process with the password saved in the macOS
//! login Keychain, and hands every other question (host key confirmation, key
//! passphrases, a retry after a rejected password) to the user on the
//! terminal. The password travels from `security` to ssh over pipes only.
//! See `docs/ADR/0005-keychain-ssh-passwords.md`.

use crate::launcher::password::{PasswordRef, KEYCHAIN_SERVICE};
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, PartialEq, Eq)]
pub enum Response {
    /// Print this line for ssh to read.
    Answer(String),
    /// Ask the user on the terminal, showing `notice` first.
    AskUser {
        hidden: bool,
        notice: Option<String>,
    },
    /// ssh wants a yes/no decision as the exit status (`SSH_ASKPASS_PROMPT=confirm`).
    Confirm,
    /// ssh only shows a message and does not wait (`SSH_ASKPASS_PROMPT=none`).
    Notify,
}

pub struct Request<'a> {
    pub prompt: &'a str,
    /// Value of `SSH_ASKPASS_PROMPT`, if ssh set it.
    pub prompt_kind: Option<&'a str>,
    pub password_ref: &'a str,
    /// Per-launch directory that remembers which ssh processes were answered.
    pub state_dir: Option<&'a Path>,
    /// The ssh process that asked, i.e. this helper's parent.
    pub ssh_pid: u32,
    pub security_bin: &'a str,
}

pub fn respond(request: &Request) -> Response {
    match request.prompt_kind {
        Some("confirm") => return Response::Confirm,
        Some("none") => return Response::Notify,
        _ => {}
    }
    if !is_password_prompt(request.prompt) {
        return Response::AskUser {
            hidden: !is_yes_no_question(request.prompt),
            notice: None,
        };
    }

    let type_it = |notice: String| Response::AskUser {
        hidden: true,
        notice: Some(format!("Shuttle: {notice}; type the password instead.")),
    };
    let Ok(PasswordRef::Keychain { account }) = PasswordRef::parse(request.password_ref) else {
        return type_it("invalid password_ref".into());
    };
    let marker = request
        .state_dir
        .map(|dir| dir.join(format!("answered-{}", request.ssh_pid)));
    if marker.as_deref().is_some_and(Path::exists) {
        // Only one automatic try per ssh process, so a wrong saved password
        // is not sent again and again (servers may ban repeated failures).
        return type_it("the saved password was not accepted".into());
    }
    match keychain_password(request.security_bin, &account) {
        Some(password) => {
            if let Some(marker) = marker {
                let _ = std::fs::write(marker, b"");
            }
            Response::Answer(password)
        }
        None => type_it(format!(
            "no password saved in Keychain for account {account} (service {KEYCHAIN_SERVICE})"
        )),
    }
}

fn is_password_prompt(prompt: &str) -> bool {
    prompt
        .lines()
        .last()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_lowercase()
        .ends_with("password:")
}

fn is_yes_no_question(prompt: &str) -> bool {
    prompt.to_ascii_lowercase().contains("(yes/no")
}

fn keychain_password(security_bin: &str, account: &str) -> Option<String> {
    let output = Command::new(security_bin)
        .args([
            "find-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            account,
            "-w",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let mut password = String::from_utf8(output.stdout).ok()?;
    if password.ends_with('\n') {
        password.pop();
    }
    Some(password)
}

/// Runs askpass mode when ssh started this executable for a `password_ref`
/// host, returning the exit code. Returns `None` for a normal app launch.
#[cfg(not(test))]
pub fn run_if_requested() -> Option<i32> {
    use crate::launcher::password::{REF_ENV, STATE_ENV};

    let password_ref = std::env::var(REF_ENV).ok()?;
    let prompt = std::env::args().nth(1).unwrap_or_default();
    let prompt_kind = std::env::var("SSH_ASKPASS_PROMPT").ok();
    let state_dir = std::env::var_os(STATE_ENV).map(std::path::PathBuf::from);
    let response = respond(&Request {
        prompt: &prompt,
        prompt_kind: prompt_kind.as_deref(),
        password_ref: &password_ref,
        state_dir: state_dir.as_deref(),
        ssh_pid: std::os::unix::process::parent_id(),
        security_bin: "/usr/bin/security",
    });
    Some(match response {
        Response::Answer(answer) => {
            println!("{answer}");
            0
        }
        Response::AskUser { hidden, notice } => {
            match tty::ask(&prompt, hidden, notice.as_deref()) {
                Ok(answer) => {
                    println!("{answer}");
                    0
                }
                Err(_) => 1,
            }
        }
        Response::Confirm => match tty::ask(&format!("{prompt} (yes/no) "), false, None) {
            Ok(answer) if answer.trim().eq_ignore_ascii_case("yes") => 0,
            _ => 1,
        },
        Response::Notify => {
            let _ = tty::show(&prompt);
            0
        }
    })
}

/// Talks to the user through the controlling terminal that ssh runs in.
#[cfg(not(test))]
mod tty {
    use std::fs::{File, OpenOptions};
    use std::io::{BufRead, BufReader, Result, Write};
    use std::process::Command;

    fn open() -> Result<File> {
        OpenOptions::new().read(true).write(true).open("/dev/tty")
    }

    pub fn show(message: &str) -> Result<()> {
        writeln!(open()?, "{message}")
    }

    pub fn ask(prompt: &str, hidden: bool, notice: Option<&str>) -> Result<String> {
        let mut tty = open()?;
        if let Some(notice) = notice {
            writeln!(tty, "{notice}")?;
        }
        write!(tty, "{prompt}")?;
        tty.flush()?;
        if hidden {
            set_echo(&tty, false);
        }
        let mut line = String::new();
        let read = BufReader::new(tty.try_clone()?).read_line(&mut line);
        if hidden {
            set_echo(&tty, true);
            let _ = writeln!(tty);
        }
        read?;
        Ok(line.trim_end_matches(['\r', '\n']).to_string())
    }

    fn set_echo(tty: &File, on: bool) {
        if let Ok(stdin) = tty.try_clone() {
            let _ = Command::new("/bin/stty")
                .arg(if on { "echo" } else { "-echo" })
                .stdin(stdin)
                .status();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A fake `security` that only knows the password of account `prod`.
    fn fake_security(dir: &Path) -> String {
        let path = dir.join("security");
        std::fs::write(
            &path,
            "#!/bin/sh\n[ \"$*\" = \"find-generic-password -s Shuttle -a prod -w\" ] || exit 44\nprintf 'pa ss$\\n\\n'\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn request<'a>(
        prompt: &'a str,
        password_ref: &'a str,
        state_dir: &'a Path,
        security_bin: &'a str,
    ) -> Request<'a> {
        Request {
            prompt,
            prompt_kind: None,
            password_ref,
            state_dir: Some(state_dir),
            ssh_pid: 4242,
            security_bin,
        }
    }

    #[test]
    fn recognises_password_prompts() {
        for prompt in [
            "shuttletest@127.0.0.1's password: ",
            "(ops@db-1) Password:",
            "Warning: something\nroot@host's password: ",
        ] {
            assert!(is_password_prompt(prompt), "{prompt:?}");
        }
        for prompt in [
            "Enter passphrase for key '/Users/me/.ssh/id_ed25519': ",
            "Are you sure you want to continue connecting (yes/no/[fingerprint])? ",
            "Verification code: ",
        ] {
            assert!(!is_password_prompt(prompt), "{prompt:?}");
        }
    }

    #[test]
    fn answers_first_password_prompt_from_keychain() {
        let dir = tempfile::tempdir().unwrap();
        let security = fake_security(dir.path());
        let first = respond(&request(
            "u@h's password: ",
            "keychain:prod",
            dir.path(),
            &security,
        ));
        // `security -w` adds one newline; only that one is removed.
        assert_eq!(first, Response::Answer("pa ss$\n".into()));
    }

    #[test]
    fn asks_user_when_saved_password_was_not_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let security = fake_security(dir.path());
        let ask = || {
            respond(&request(
                "u@h's password: ",
                "keychain:prod",
                dir.path(),
                &security,
            ))
        };
        assert!(matches!(ask(), Response::Answer(_)));
        let Response::AskUser { hidden, notice } = ask() else {
            panic!("retry must go to the user");
        };
        assert!(hidden);
        assert!(notice.unwrap().contains("not accepted"));

        // Another ssh process in the same launch gets its own automatic try.
        let other = Request {
            ssh_pid: 4343,
            ..request("u@h's password: ", "keychain:prod", dir.path(), &security)
        };
        assert!(matches!(respond(&other), Response::Answer(_)));
    }

    #[test]
    fn asks_user_when_nothing_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let security = fake_security(dir.path());
        let Response::AskUser { hidden, notice } = respond(&request(
            "u@h's password: ",
            "keychain:other",
            dir.path(),
            &security,
        )) else {
            panic!("missing password must go to the user");
        };
        assert!(hidden);
        assert!(notice
            .unwrap()
            .contains("no password saved in Keychain for account other (service Shuttle)"));
        assert!(!dir.path().join("answered-4242").exists());
    }

    #[test]
    fn asks_user_for_invalid_ref() {
        let dir = tempfile::tempdir().unwrap();
        let response = respond(&request(
            "u@h's password: ",
            "hunter2",
            dir.path(),
            "/nonexistent",
        ));
        let Response::AskUser { notice, .. } = response else {
            panic!("invalid ref must go to the user");
        };
        assert!(!notice.unwrap().contains("hunter2"));
    }

    #[test]
    fn passes_other_questions_to_the_user() {
        let dir = tempfile::tempdir().unwrap();
        let ask = |prompt| {
            respond(&request(
                prompt,
                "keychain:prod",
                dir.path(),
                "/nonexistent",
            ))
        };
        assert_eq!(
            ask("Are you sure you want to continue connecting (yes/no/[fingerprint])? "),
            Response::AskUser {
                hidden: false,
                notice: None
            }
        );
        assert_eq!(
            ask("Enter passphrase for key 'id_ed25519': "),
            Response::AskUser {
                hidden: true,
                notice: None
            }
        );
        for (kind, expected) in [("confirm", Response::Confirm), ("none", Response::Notify)] {
            let request = Request {
                prompt_kind: Some(kind),
                ..request(
                    "u@h's password: ",
                    "keychain:prod",
                    dir.path(),
                    "/nonexistent",
                )
            };
            assert_eq!(respond(&request), expected);
        }
    }
}
