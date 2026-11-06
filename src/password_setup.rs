//! Password setup mode: `shuttle-rs --setup-password <name> <command> <menu path…>`.
//!
//! For a host with `"password_ref": ""`, the menu runs this in the terminal
//! instead of the host's command. It asks for a Keychain account name (the
//! default comes from the host's menu path), lets `/usr/bin/security
//! add-generic-password` ask for the password itself, writes
//! `"password_ref": "keychain:<account>"` back into the JSON config, and offers
//! to connect. Shuttle never sees the password.
//! See `docs/ADR/0005-keychain-ssh-passwords.md`.

use crate::launcher::password::{PasswordRef, KEYCHAIN_SERVICE};
use std::io::{self, BufRead, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The host being set up, as the menu passed it.
pub struct Host<'a> {
    /// Raw `name` from the config.
    pub name: &'a str,
    /// The host's command, normalized to one line.
    pub command: &'a str,
    /// Menu titles down to the host, ending with its own title.
    pub menu_path: &'a [String],
}

pub struct Env<'a> {
    pub security_bin: &'a str,
    /// Config files that may hold the host, in lookup order.
    pub config_files: &'a [PathBuf],
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The password is saved; connect with it now.
    Connect(PasswordRef),
    /// The password is saved; the user chose not to connect.
    Done,
    /// The user stopped before a password was set up.
    Quit,
}

/// Default Keychain account for a host: its menu path as a slug, for example
/// `EL → Local → Local Server` becomes `el-local-local-server`.
pub fn default_account(menu_path: &[String]) -> String {
    let mut slug = String::new();
    for ch in menu_path.join("/").chars() {
        if ch.is_whitespace() || ch.is_ascii_punctuation() || ch.is_control() {
            if !slug.is_empty() && !slug.ends_with('-') {
                slug.push('-');
            }
        } else {
            slug.extend(ch.to_lowercase());
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "host".to_string()
    } else {
        slug.to_string()
    }
}

fn validate_account(input: &str) -> Result<String, &'static str> {
    let account = input.trim();
    if account.is_empty() {
        Err("The account name cannot be empty.")
    } else if account.chars().any(char::is_control) {
        Err("The account name cannot contain tabs or other control characters.")
    } else {
        Ok(account.to_string())
    }
}

/// Reads a one-letter choice; Enter picks `default`.
fn choice(input: &str, allowed: &str, default: char) -> Option<char> {
    let input = input.trim().to_lowercase();
    match input.chars().next() {
        None => Some(default),
        Some(ch) if allowed.contains(ch) => Some(ch),
        Some(_) => None,
    }
}

/// Runs the interactive setup. Returns `Ok(Outcome::Quit)` on end of input.
pub fn run_with(
    host: &Host,
    env: &Env,
    input: &mut impl BufRead,
    out: &mut impl Write,
) -> io::Result<Outcome> {
    let title = host.menu_path.last().map_or(host.name, String::as_str);
    writeln!(out, "Shuttle: save a password for \"{title}\"")?;
    writeln!(out, "  menu:    {}", host.menu_path.join(" → "))?;
    writeln!(out, "  command: {}", host.command)?;
    match locate(env.config_files, host) {
        Ok(edit) => writeln!(out, "  config:  {}", tilde(&edit.file))?,
        Err(reason) => writeln!(out, "  config:  not updated automatically ({reason})")?,
    }
    writeln!(out)?;

    let default = default_account(host.menu_path);
    let (account, reused) = loop {
        let Some(answer) = ask(input, out, &format!("Keychain account name [{default}]: "))? else {
            return Ok(Outcome::Quit);
        };
        let answer = if answer.trim().is_empty() {
            default.clone()
        } else {
            answer
        };
        let account = match validate_account(&answer) {
            Ok(account) => account,
            Err(problem) => {
                writeln!(out, "{problem}")?;
                continue;
            }
        };

        let replace = if keychain_item_exists(env.security_bin, &account) {
            writeln!(out, "An item for account \"{account}\" already exists.")?;
            let picked = loop {
                let prompt =
                    "[u]se it  [r]eplace its password  [c]hoose another name  [q]uit  (u): ";
                let Some(answer) = ask(input, out, prompt)? else {
                    return Ok(Outcome::Quit);
                };
                if let Some(picked) = choice(&answer, "urcq", 'u') {
                    break picked;
                }
            };
            match picked {
                'u' => break (account, true),
                'c' => continue,
                'q' => return Ok(Outcome::Quit),
                _ => true,
            }
        } else {
            false
        };

        loop {
            out.flush()?;
            if save_keychain_item(env.security_bin, &account, replace) {
                break;
            }
            let Some(answer) = ask(input, out, "✗ Password not saved.  [r]etry  [q]uit  (r): ")?
            else {
                return Ok(Outcome::Quit);
            };
            if choice(&answer, "rq", 'r') == Some('q') {
                return Ok(Outcome::Quit);
            }
        }
        break (account, false);
    };

    if reused {
        writeln!(
            out,
            "✓ Using the saved Keychain item (service {KEYCHAIN_SERVICE}, account {account})"
        )?;
    } else {
        writeln!(
            out,
            "✓ Saved in Keychain (service {KEYCHAIN_SERVICE}, account {account})"
        )?;
    }
    let password = PasswordRef::Keychain {
        account: account.clone(),
    };
    let value = serde_json::to_string(&password.to_config_value()).expect("a string serializes");
    match update_config(env.config_files, host, &password) {
        Ok(()) => writeln!(out, "✓ Updated config: \"password_ref\": {value}")?,
        Err(reason) => {
            writeln!(
                out,
                "! Could not update the config automatically ({reason})."
            )?;
            writeln!(out, "  Set this on the host:")?;
            writeln!(out, "    \"password_ref\": {value}")?;
        }
    }
    writeln!(out)?;

    loop {
        let Some(answer) = ask(input, out, "Connect now? [Y/n]: ")? else {
            return Ok(Outcome::Done);
        };
        match choice(&answer, "yn", 'y') {
            Some('y') => return Ok(Outcome::Connect(password)),
            Some(_) => return Ok(Outcome::Done),
            None => {}
        }
    }
}

fn ask(input: &mut impl BufRead, out: &mut impl Write, prompt: &str) -> io::Result<Option<String>> {
    write!(out, "{prompt}")?;
    out.flush()?;
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        writeln!(out)?;
        return Ok(None);
    }
    Ok(Some(line.trim_end_matches(['\r', '\n']).to_string()))
}

fn keychain_item_exists(security_bin: &str, account: &str) -> bool {
    Command::new(security_bin)
        .args([
            "find-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            account,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// `security` reads the password from the terminal itself (`-w` last).
fn save_keychain_item(security_bin: &str, account: &str, replace: bool) -> bool {
    let mut command = Command::new(security_bin);
    command.arg("add-generic-password");
    if replace {
        command.arg("-U");
    }
    command
        .args(["-s", KEYCHAIN_SERVICE, "-a", account, "-w"])
        .status()
        .is_ok_and(|status| status.success())
}

fn tilde(path: &Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(relative) => format!("~/{}", relative.display()),
        None => path.display().to_string(),
    }
}

// ── Writing password_ref back into the config ─────────────────────────────────

/// The one empty `password_ref` value in a JSON config that belongs to the host.
#[derive(Debug, PartialEq, Eq)]
struct Edit {
    file: PathBuf,
    value: Range<usize>,
}

fn locate(files: &[PathBuf], host: &Host) -> Result<Edit, String> {
    let json_files: Vec<&PathBuf> = files
        .iter()
        .filter(|file| {
            file.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        })
        .collect();
    if json_files.is_empty() {
        return Err("only JSON configs can be updated".into());
    }
    let mut edits = Vec::new();
    for file in json_files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        for value in empty_password_refs(&text, host) {
            edits.push(Edit {
                file: file.clone(),
                value,
            });
        }
    }
    match edits.len() {
        1 => Ok(edits.remove(0)),
        0 => Err(
            "no host with this name and command has an empty password_ref in the JSON config"
                .into(),
        ),
        _ => Err("more than one host with this name and command has an empty password_ref".into()),
    }
}

fn update_config(files: &[PathBuf], host: &Host, password: &PasswordRef) -> Result<(), String> {
    let edit = locate(files, host)?;
    let text = std::fs::read_to_string(&edit.file).map_err(|error| error.to_string())?;
    let value = serde_json::to_string(&password.to_config_value()).expect("a string serializes");
    let mut updated = String::with_capacity(text.len() + value.len());
    updated.push_str(&text[..edit.value.start]);
    updated.push_str(&value);
    updated.push_str(&text[edit.value.end..]);
    write_atomically(&edit.file, &updated).map_err(|error| error.to_string())
}

/// Replaces the file's contents in one step, writing through a symlink (for
/// dotfiles setups) and keeping the file's permissions.
fn write_atomically(file: &Path, contents: &str) -> io::Result<()> {
    let target = std::fs::canonicalize(file)?;
    let permissions = std::fs::metadata(&target)?.permissions();
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = target.with_file_name(format!(".{name}.shuttle-tmp"));
    std::fs::write(&temporary, contents)?;
    std::fs::set_permissions(&temporary, permissions)?;
    std::fs::rename(&temporary, &target)
}

/// Byte ranges of `""` values of `password_ref` keys whose enclosing JSON
/// object has the host's `name` and (normalized) `cmd`.
fn empty_password_refs(text: &str, host: &Host) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut open_objects = Vec::new();
    let mut object_ends = std::collections::HashMap::new();
    let mut candidates = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => open_objects.push(i),
            b'}' => {
                if let Some(start) = open_objects.pop() {
                    object_ends.insert(start, i + 1);
                }
            }
            b'"' => {
                let end = string_end(bytes, i);
                if &text[i..end] == "\"password_ref\"" {
                    if let (Some(&object), Some(value)) =
                        (open_objects.last(), empty_value_after_key(bytes, end))
                    {
                        candidates.push((object, value));
                    }
                }
                i = end;
                continue;
            }
            _ => {}
        }
        i += 1;
    }

    candidates
        .into_iter()
        .filter(|(object, _)| {
            object_ends
                .get(object)
                .and_then(|end| {
                    serde_json::from_str::<serde_json::Value>(&text[*object..*end]).ok()
                })
                .is_some_and(|object| {
                    object["name"].as_str() == Some(host.name)
                        && object["cmd"].as_str().is_some_and(|cmd| {
                            crate::launcher::normalize_command(cmd) == host.command
                        })
                })
        })
        .map(|(_, value)| value)
        .collect()
}

/// Index just past the string literal that starts at `start`.
fn string_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// After a key ending at `key_end`: the range of an empty string value.
fn empty_value_after_key(bytes: &[u8], key_end: usize) -> Option<Range<usize>> {
    let skip = |mut i: usize| {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    };
    let colon = skip(key_end);
    if bytes.get(colon) != Some(&b':') {
        return None;
    }
    let value = skip(colon + 1);
    (bytes.get(value) == Some(&b'"') && bytes.get(value + 1) == Some(&b'"'))
        .then_some(value..value + 2)
}

/// Entry point from `main` for `--setup-password <name> <command> <menu path…>`.
#[cfg(not(test))]
pub fn run(args: &[String]) -> i32 {
    use std::os::unix::process::CommandExt;

    let [name, command, menu_path @ ..] = args else {
        eprintln!("usage: shuttle-rs --setup-password <name> <command> <menu path…>");
        return 2;
    };
    let config_files: Vec<PathBuf> = crate::config::discover_paths()
        .map(|paths| std::iter::once(paths.main).chain(paths.alt).collect())
        .unwrap_or_default();
    let host = Host {
        name,
        command,
        menu_path,
    };
    let env = Env {
        security_bin: "/usr/bin/security",
        config_files: &config_files,
    };
    let outcome = run_with(&host, &env, &mut io::stdin().lock(), &mut io::stdout());
    match outcome {
        Ok(Outcome::Connect(password)) => {
            let shuttle = match std::env::current_exe() {
                Ok(shuttle) => shuttle,
                Err(error) => {
                    eprintln!("Shuttle: cannot locate own executable: {error}");
                    return 1;
                }
            };
            let wrapped = crate::launcher::password::wrap_command(command, &password, &shuttle);
            let error = Command::new("/bin/sh").arg("-c").arg(wrapped).exec();
            eprintln!("Shuttle: cannot start the command: {error}");
            1
        }
        Ok(Outcome::Done) => 0,
        Ok(Outcome::Quit) => 1,
        Err(error) => {
            eprintln!("Shuttle: {error}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::os::unix::fs::PermissionsExt;

    const CONFIG: &str = r#"{
  "terminal": "Terminal.app",
  "hosts": [
    { "[aaa]EL": [
        { "Local": [
            {
              "name": "Local Server",
              "cmd": "ssh user@10.0.0.1",
              "password_ref":   ""
            },
            { "name": "Other", "cmd": "echo \"}{\" ", "password_ref": "" }
        ] }
    ] }
  ]
}
"#;

    struct Fixture {
        dir: tempfile::TempDir,
        security: String,
        config: PathBuf,
    }

    /// A fake `security`: `find-generic-password` succeeds when `exists-<account>`
    /// is present, `add-generic-password` logs its arguments and fails once when
    /// `add-fails-once` is present.
    fn fixture(config: &str) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().display().to_string();
        let security = dir.path().join("security");
        std::fs::write(
            &security,
            format!(
                r#"#!/bin/sh
d='{d}'
case "$1" in
  find-generic-password) [ -e "$d/exists-$5" ] ;;
  add-generic-password)
    echo "$*" >> "$d/log"
    if [ -e "$d/add-fails-once" ]; then rm "$d/add-fails-once"; exit 51; fi ;;
  *) exit 2 ;;
esac
"#
            ),
        )
        .unwrap();
        std::fs::set_permissions(&security, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config_path = dir.path().join("config.json");
        std::fs::write(&config_path, config).unwrap();
        Fixture {
            security: security.to_string_lossy().into_owned(),
            config: config_path,
            dir,
        }
    }

    fn menu_path() -> Vec<String> {
        vec!["EL".into(), "Local".into(), "Local Server".into()]
    }

    fn run(fixture: &Fixture, files: &[PathBuf], typed: &str) -> (Outcome, String) {
        let path = menu_path();
        let host = Host {
            name: "Local Server",
            command: "ssh user@10.0.0.1",
            menu_path: &path,
        };
        let env = Env {
            security_bin: &fixture.security,
            config_files: files,
        };
        let mut out = Vec::new();
        let outcome = run_with(&host, &env, &mut Cursor::new(typed), &mut out).unwrap();
        (outcome, String::from_utf8(out).unwrap())
    }

    fn log(fixture: &Fixture) -> String {
        std::fs::read_to_string(fixture.dir.path().join("log")).unwrap_or_default()
    }

    fn keychain(account: &str) -> PasswordRef {
        PasswordRef::Keychain {
            account: account.into(),
        }
    }

    #[test]
    fn builds_default_account_from_menu_path() {
        let path = |parts: &[&str]| {
            parts
                .iter()
                .map(|part| part.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(default_account(&menu_path()), "el-local-local-server");
        assert_eq!(
            default_account(&path(&["Client Projects", "Emenscr", "Prod", "DB 1"])),
            "client-projects-emenscr-prod-db-1"
        );
        assert_eq!(default_account(&path(&["--a!!b--", "C"])), "a-b-c");
        assert_eq!(
            default_account(&path(&["เซิร์ฟเวอร์ ทดสอบ"])),
            "เซิร์ฟเวอร์-ทดสอบ"
        );
        assert_eq!(default_account(&path(&["!!!"])), "host");
        assert_eq!(default_account(&[]), "host");
    }

    #[test]
    fn saves_password_updates_config_and_connects() {
        let fixture = fixture(CONFIG);
        let (outcome, out) = run(&fixture, std::slice::from_ref(&fixture.config), "\n\n");

        assert_eq!(outcome, Outcome::Connect(keychain("el-local-local-server")));
        assert_eq!(
            log(&fixture),
            "add-generic-password -s Shuttle -a el-local-local-server -w\n"
        );
        assert_eq!(
            std::fs::read_to_string(&fixture.config).unwrap(),
            CONFIG.replacen(
                r#""password_ref":   """#,
                r#""password_ref":   "keychain:el-local-local-server""#,
                1
            ),
            "only the host's value changes; formatting and the other host stay"
        );
        assert!(out.contains("Shuttle: save a password for \"Local Server\""));
        assert!(out.contains("menu:    EL → Local → Local Server"));
        assert!(out.contains("Keychain account name [el-local-local-server]: "));
        assert!(
            out.contains("✓ Saved in Keychain (service Shuttle, account el-local-local-server)")
        );
        assert!(
            out.contains("✓ Updated config: \"password_ref\": \"keychain:el-local-local-server\"")
        );
        assert!(out.ends_with("Connect now? [Y/n]: "));
    }

    #[test]
    fn reuses_replaces_or_renames_existing_items() {
        // [u]se: no new item, then do not connect.
        let fixture = fixture(CONFIG);
        std::fs::write(fixture.dir.path().join("exists-prod"), "").unwrap();
        let (outcome, out) = run(
            &fixture,
            std::slice::from_ref(&fixture.config),
            "prod\nu\nn\n",
        );
        assert_eq!(outcome, Outcome::Done);
        assert_eq!(log(&fixture), "");
        assert!(out.contains("An item for account \"prod\" already exists."));
        assert!(out.contains("✓ Using the saved Keychain item"));
        assert!(std::fs::read_to_string(&fixture.config)
            .unwrap()
            .contains("\"keychain:prod\""));

        // [r]eplace: update the item with -U. An unknown letter asks again.
        let fixture = self::fixture(CONFIG);
        std::fs::write(fixture.dir.path().join("exists-prod"), "").unwrap();
        let (outcome, _) = run(
            &fixture,
            std::slice::from_ref(&fixture.config),
            "prod\nx\nr\ny\n",
        );
        assert_eq!(outcome, Outcome::Connect(keychain("prod")));
        assert_eq!(
            log(&fixture),
            "add-generic-password -U -s Shuttle -a prod -w\n"
        );

        // [c]hoose another name.
        let fixture = self::fixture(CONFIG);
        std::fs::write(fixture.dir.path().join("exists-prod"), "").unwrap();
        let (outcome, _) = run(
            &fixture,
            std::slice::from_ref(&fixture.config),
            "prod\nc\nprod-2\n\n",
        );
        assert_eq!(outcome, Outcome::Connect(keychain("prod-2")));
        assert_eq!(
            log(&fixture),
            "add-generic-password -s Shuttle -a prod-2 -w\n"
        );
    }

    #[test]
    fn retries_or_quits_when_security_fails() {
        let fixture = fixture(CONFIG);
        std::fs::write(fixture.dir.path().join("add-fails-once"), "").unwrap();
        let (outcome, out) = run(&fixture, std::slice::from_ref(&fixture.config), "\n\n\n");
        assert_eq!(outcome, Outcome::Connect(keychain("el-local-local-server")));
        assert!(out.contains("✗ Password not saved."));
        assert_eq!(log(&fixture).lines().count(), 2);

        let fixture = self::fixture(CONFIG);
        std::fs::write(fixture.dir.path().join("add-fails-once"), "").unwrap();
        let (outcome, _) = run(&fixture, std::slice::from_ref(&fixture.config), "\nq\n");
        assert_eq!(outcome, Outcome::Quit);
        assert_eq!(std::fs::read_to_string(&fixture.config).unwrap(), CONFIG);
    }

    #[test]
    fn rejects_bad_names_and_stops_at_end_of_input() {
        let fixture = fixture(CONFIG);
        let (outcome, out) = run(&fixture, std::slice::from_ref(&fixture.config), "a\tb\n");
        assert_eq!(outcome, Outcome::Quit);
        assert!(out.contains("cannot contain tabs"));
        assert_eq!(log(&fixture), "");
        assert_eq!(std::fs::read_to_string(&fixture.config).unwrap(), CONFIG);
    }

    #[test]
    fn explains_when_the_config_cannot_be_updated() {
        let fixture = fixture(CONFIG);
        let yaml = fixture.dir.path().join("config.yaml");
        std::fs::write(&yaml, "hosts: []\n").unwrap();
        let (outcome, out) = run(&fixture, &[yaml], "\n\n");
        assert_eq!(outcome, Outcome::Connect(keychain("el-local-local-server")));
        assert!(
            out.contains("config:  not updated automatically (only JSON configs can be updated)")
        );
        assert!(out.contains(
            "Set this on the host:\n    \"password_ref\": \"keychain:el-local-local-server\""
        ));
    }

    #[test]
    fn finds_only_the_matching_empty_ref() {
        let path = menu_path();
        let host = |name, command| Host {
            name,
            command,
            menu_path: &path,
        };
        let refs =
            |text: &str, name, command| empty_password_refs(text, &host(name, command)).len();

        assert_eq!(refs(CONFIG, "Local Server", "ssh user@10.0.0.1"), 1);
        assert_eq!(
            refs(CONFIG, "Other", "echo \"}{\""),
            1,
            "braces inside strings"
        );
        assert_eq!(refs(CONFIG, "Local Server", "ssh other"), 0);
        assert_eq!(
            refs(
                r#"[{"name":"A","cmd":"ssh a\nls","password_ref":""}]"#,
                "A",
                "ssh a && ls"
            ),
            1,
            "multi-line commands match their normalized form"
        );
        assert_eq!(
            refs(
                r#"[{"name":"A","cmd":"ssh a","password_ref":"keychain:a"}]"#,
                "A",
                "ssh a"
            ),
            0
        );

        let fixture = fixture(&CONFIG.replace(
            "\"Other\", \"cmd\": \"echo \\\"}{\\\" \"",
            "\"Local Server\", \"cmd\": \"ssh user@10.0.0.1\"",
        ));
        assert!(locate(
            std::slice::from_ref(&fixture.config),
            &host("Local Server", "ssh user@10.0.0.1")
        )
        .unwrap_err()
        .contains("more than one"));
    }

    #[test]
    fn writes_through_symlinks() {
        let fixture = fixture(CONFIG);
        let link = fixture.dir.path().join("linked.json");
        std::os::unix::fs::symlink(&fixture.config, &link).unwrap();
        let (outcome, _) = run(&fixture, std::slice::from_ref(&link), "\n\n");
        assert!(matches!(outcome, Outcome::Connect(_)));
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(std::fs::read_to_string(&fixture.config)
            .unwrap()
            .contains("keychain:el-local-local-server"));
    }
}
