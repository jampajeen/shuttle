#[cfg(not(test))]
use serde::Deserialize;
use std::process::Command;

#[cfg(not(test))]
#[derive(Debug, Deserialize)]
struct TerminalPayload {
    kind: String,
    backend: String,
    target: String,
    title: String,
    theme: String,
    #[serde(default)]
    password_ref: Option<String>,
    #[serde(default)]
    password_setup: Option<PasswordSetupPayload>,
}

/// A host with an empty `password_ref`: run password setup instead.
#[cfg(not(test))]
#[derive(Debug, Deserialize)]
struct PasswordSetupPayload {
    /// Raw `name` from the config, used to find the host when saving.
    name: String,
    /// Menu titles down to the host, used for the default Keychain account.
    path: Vec<String>,
}

/// Top-level dispatch. `backend` is a resolved backend string (e.g. "ghostty-open",
/// "terminal-app", "iterm-stable"). `cmd` is the raw command from the config.
#[cfg(not(test))]
pub fn execute(cmd: &str, backend: &str) {
    let cmd = normalize_cmd(cmd);
    if let Ok(payload) = serde_json::from_str::<TerminalPayload>(backend) {
        if payload.kind == "terminal" {
            match terminal_command(&cmd, &payload) {
                Ok(cmd) => execute_terminal_payload(&cmd, &payload),
                Err(error) => eprintln!("Shuttle: {error}"),
            }
            return;
        }
    }

    match backend {
        "ghostty-open" => launch_ghostty_open(&cmd, "shuttle"),
        "ghostty-applescript" => launch_ghostty_applescript(&cmd, "new"),
        "iterm-stable" => launch_iterm(&cmd, false, "new", "default profile"),
        "iterm-nightly" => launch_iterm(&cmd, true, "new", "default profile"),
        "cmux-cli" => launch_cmux_cli(&cmd, "new", "shuttle"),
        "cmux-socket" => launch_cmux_socket(&cmd, "current", "shuttle"),
        "screen" => launch_screen(&cmd, "shuttle"),
        // url: prefix injected by menu builder for URL commands
        url if url.starts_with("url:") => launch_url(&url["url:".len()..]),
        // terminal-app and anything unknown fall through to Terminal.app
        _ => launch_terminal_app(&cmd, "new", "basic", "shuttle"),
    }
}

/// The command line to type into the terminal for `cmd`, wrapped for a saved
/// password or replaced by password setup. Both re-run this executable.
#[cfg(not(test))]
fn terminal_command(cmd: &str, payload: &TerminalPayload) -> Result<String, String> {
    use crate::launcher::password;

    if payload.password_ref.is_none() && payload.password_setup.is_none() {
        return Ok(cmd.to_string());
    }
    let shuttle = std::env::current_exe()
        .map_err(|error| format!("cannot locate own executable for passwords: {error}"))?;
    if let Some(setup) = &payload.password_setup {
        return Ok(password::setup_command(
            &shuttle,
            &setup.name,
            cmd,
            &setup.path,
        ));
    }
    let saved = payload.password_ref.as_deref().unwrap_or_default();
    let saved = password::PasswordRef::parse(saved).map_err(|error| error.to_string())?;
    // ssh runs this executable again in askpass mode.
    Ok(password::wrap_command(cmd, &saved, &shuttle))
}

#[cfg(not(test))]
fn execute_terminal_payload(cmd: &str, payload: &TerminalPayload) {
    match payload.backend.as_str() {
        "ghostty-open" => launch_ghostty_open(cmd, &payload.title),
        "ghostty-applescript" => launch_ghostty_applescript(cmd, &payload.target),
        "iterm-stable" => launch_iterm(cmd, false, &payload.target, &payload.theme),
        "iterm-nightly" => launch_iterm(cmd, true, &payload.target, &payload.theme),
        "cmux-cli" => launch_cmux_cli(cmd, &payload.target, &payload.title),
        "cmux-socket" => launch_cmux_socket(cmd, &payload.target, &payload.title),
        "screen" => launch_screen(cmd, &payload.title),
        _ => launch_terminal_app(cmd, &payload.target, &payload.theme, &payload.title),
    }
}

// ── Command normalisation ─────────────────────────────────────────────────────

/// Collapse multi-line commands (JSON \n) into a single shell-safe command.
fn normalize_cmd(cmd: &str) -> String {
    crate::launcher::normalize_command(cmd)
}

// ── URL ───────────────────────────────────────────────────────────────────────

#[cfg(not(test))]
fn launch_url(url: &str) {
    open_url_command(url).spawn().ok();
}

fn open_url_command(url: &str) -> Command {
    let mut command = Command::new("open");
    command.arg(url);
    command
}

// ── Ghostty ───────────────────────────────────────────────────────────────────

#[cfg(not(test))]
fn launch_ghostty_open(cmd: &str, title: &str) {
    ghostty_open_command(cmd, title).spawn().ok();
}

fn ghostty_open_command(cmd: &str, title: &str) -> Command {
    // open -a Ghostty.app --args -e sh -c "cmd"
    // Do not pass -n: that forces macOS to launch a separate Ghostty instance.
    // Using sh -c so multi-part commands and quoting work correctly.
    let mut command = Command::new("open");
    command.args([
        "-a",
        "Ghostty.app",
        "--args",
        "--title",
        title,
        "-e",
        "sh",
        "-c",
        cmd,
    ]);
    command
}

#[cfg(not(test))]
fn launch_ghostty_applescript(cmd: &str, target: &str) {
    run_osascript(&ghostty_applescript(cmd, target));
}

fn ghostty_applescript(cmd: &str, target: &str) -> String {
    let escaped = escape_for_applescript(cmd);
    match target {
        "tab" => format!(
            r#"tell application "Ghostty"
    activate
    set cfg to new surface configuration
    set command of cfg to "{escaped}"
    if (count of windows) = 0 then
        set win to new window with configuration cfg
    else
        set win to front window
        set newTab to new tab in win with configuration cfg
        select tab newTab
    end if
end tell"#
        ),
        "current" => format!(
            r#"tell application "Ghostty"
    activate
    if (count of windows) = 0 then
        set cfg to new surface configuration
        set command of cfg to "{escaped}"
        set win to new window with configuration cfg
    else
        set term to focused terminal of selected tab of front window
        input text "{escaped}" to term
        send key "enter" to term
    end if
end tell"#
        ),
        _ => format!(
            r#"tell application "Ghostty"
    activate
    set cfg to new surface configuration
    set command of cfg to "{escaped}"
    set win to new window with configuration cfg
end tell"#
        ),
    }
}

// ── Terminal.app ──────────────────────────────────────────────────────────────

#[cfg(not(test))]
fn launch_terminal_app(cmd: &str, target: &str, theme: &str, title: &str) {
    run_osascript(&terminal_app_script(cmd, target, theme, title));
}

fn terminal_app_script(cmd: &str, target: &str, theme: &str, title: &str) -> String {
    let escaped = escape_for_applescript(cmd);
    let theme = escape_for_applescript(theme);
    let title = escape_for_applescript(title);
    let target_script = match target {
        "current" => format!("do script \"{escaped}\" in selected tab of front window"),
        // Terminal's AppleScript has no "new tab" command (`do script ""` opens a
        // window), so press ⌘T through System Events, which needs Accessibility
        // permission. Without it, fall back to a new window. If Terminal was just
        // launched, use the window it opens at startup instead of adding a tab.
        "tab" => format!(
            r#"if not terminalWasRunning then
        repeat 20 times
            if (count of windows) > 0 then exit repeat
            delay 0.1
        end repeat
    end if
    if (count of windows) is 0 then
        do script "{escaped}"
    else if not terminalWasRunning then
        do script "{escaped}" in selected tab of front window
    else
        set openedTab to false
        try
            tell application "System Events"
                set frontmost of process "Terminal" to true
                keystroke "t" using {{command down}}
            end tell
            set openedTab to true
        end try
        if openedTab then
            delay 0.3
            do script "{escaped}" in selected tab of front window
        else
            do script "{escaped}"
        end if
    end if"#
        ),
        _ => format!("do script \"{escaped}\""),
    };
    // Applying a settings set resizes the window to that profile's size. A new
    // window may take it, but an existing window the user sized must keep its size.
    let apply_theme = if target == "new" {
        format!(r#"set current settings of selected tab of front window to settings set "{theme}""#)
    } else {
        format!(
            r#"set windowBounds to bounds of front window
        set current settings of selected tab of front window to settings set "{theme}"
        set bounds of front window to windowBounds"#
        )
    };
    format!(
        r#"set terminalWasRunning to application "Terminal" is running
tell application "Terminal"
    activate
    {target_script}
    try
        {apply_theme}
    end try
    try
        set custom title of selected tab of front window to "{title}"
    end try
end tell"#
    )
}

// ── iTerm ─────────────────────────────────────────────────────────────────────

#[cfg(not(test))]
fn launch_iterm(cmd: &str, nightly: bool, target: &str, profile: &str) {
    run_osascript(&iterm_script(cmd, nightly, target, profile));
}

fn iterm_script(cmd: &str, nightly: bool, target: &str, profile: &str) -> String {
    let escaped = escape_for_applescript(cmd);
    let profile = escape_for_applescript(profile);
    let app = if nightly { "iTerm" } else { "iTerm2" };
    let launch = match target {
        "current" => format!("tell current session of current window to write text \"{escaped}\""),
        "tab" => format!(
            "tell current window to create tab with profile \"{profile}\" command \"{escaped}\""
        ),
        _ => format!("create window with profile \"{profile}\" command \"{escaped}\""),
    };
    format!(
        r#"tell application "{app}"
    activate
    {launch}
end tell"#
    )
}

// ── cmux ──────────────────────────────────────────────────────────────────────

#[cfg(not(test))]
fn launch_cmux_cli(cmd: &str, target: &str, title: &str) {
    let binary = crate::launcher::cmux::default_binary()
        .unwrap_or_else(|_| std::path::PathBuf::from("cmux"));
    match target {
        "current" => Command::new(binary).args(["send", cmd]).spawn().ok(),
        "virtual" => Command::new(binary)
            .args(["run", "--background", cmd])
            .spawn()
            .ok(),
        _ => Command::new(binary)
            .args(["workspace", "send", title, cmd])
            .spawn()
            .ok(),
    };
}

#[cfg(not(test))]
fn launch_cmux_socket(cmd: &str, target: &str, title: &str) {
    if let Ok(path) = crate::launcher::cmux::socket_path() {
        let (method, params) = match target {
            "current" => ("surface.send", serde_json::json!({ "text": cmd })),
            "virtual" => (
                "command.run",
                serde_json::json!({ "command": cmd, "background": true }),
            ),
            _ => (
                "workspace.send",
                serde_json::json!({ "workspace": title, "text": cmd }),
            ),
        };
        let payload = crate::launcher::cmux::socket_request(1, method, params);
        crate::launcher::cmux::send_socket_request(&path, &payload).ok();
    }
}

// ── Virtual / screen ─────────────────────────────────────────────────────────

#[cfg(not(test))]
fn launch_screen(cmd: &str, title: &str) {
    screen_command(cmd, title).spawn().ok();
}

fn screen_command(cmd: &str, title: &str) -> Command {
    let mut command = Command::new("screen");
    command.args(["-d", "-m", "-S", title, "sh", "-lc", cmd]);
    command
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Escape a string for embedding inside an AppleScript double-quoted string.
fn escape_for_applescript(s: &str) -> String {
    crate::macos::util::escape_for_applescript(s)
}

#[cfg(not(test))]
fn run_osascript(script: &str) {
    Command::new("osascript").arg("-e").arg(script).spawn().ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_multiline_commands() {
        assert_eq!(normalize_cmd("ssh host\ncd /var"), "ssh host && cd /var");
        assert_eq!(normalize_cmd("  ssh host  "), "ssh host");
    }

    #[test]
    fn escapes_quotes_and_backslashes() {
        assert_eq!(escape_for_applescript(r#"echo "hi""#), r#"echo \"hi\""#);
        assert_eq!(escape_for_applescript(r#"C:\tmp"#), r#"C:\\tmp"#);
    }

    #[test]
    fn builds_open_and_screen_commands() {
        let open = open_url_command("https://example.com");
        assert_eq!(open.get_program(), "open");
        assert_eq!(open.get_args().collect::<Vec<_>>(), ["https://example.com"]);

        let screen = screen_command("ssh prod", "Prod");
        assert_eq!(screen.get_program(), "screen");
        assert_eq!(
            screen.get_args().collect::<Vec<_>>(),
            ["-d", "-m", "-S", "Prod", "sh", "-lc", "ssh prod"]
        );
    }

    #[test]
    fn builds_ghostty_open_command() {
        let command = ghostty_open_command("ssh prod", "Prod");
        assert_eq!(command.get_program(), "open");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "-a",
                "Ghostty.app",
                "--args",
                "--title",
                "Prod",
                "-e",
                "sh",
                "-c",
                "ssh prod",
            ]
        );
    }

    #[test]
    fn builds_applescripts_for_targets() {
        assert!(ghostty_applescript("ssh prod", "tab")
            .contains("new tab in win with configuration cfg"));
        assert!(ghostty_applescript("ssh prod", "current").contains("input text"));
        assert!(
            ghostty_applescript("ssh prod", "new").contains("new window with configuration cfg")
        );

        assert!(terminal_app_script("ssh prod", "current", "Basic", "Prod")
            .contains("selected tab of front window"));
        let tab = terminal_app_script("ssh prod", "tab", "Basic", "Prod");
        assert!(!tab.contains("in (do script \"\")"), "that opens a window");
        assert!(tab.contains("keystroke \"t\" using {command down}"));
        assert!(tab.contains("do script \"ssh prod\" in selected tab of front window"));
        assert!(tab.starts_with("set terminalWasRunning to application \"Terminal\" is running"));

        let keeps_size = "set bounds of front window to windowBounds";
        assert!(tab.contains(keeps_size));
        assert!(terminal_app_script("ssh prod", "current", "Basic", "Prod").contains(keeps_size));
        assert!(!terminal_app_script("ssh prod", "new", "Basic", "Prod").contains(keeps_size));
    }

    #[test]
    fn terminal_app_scripts_compile() {
        // osacompile only checks syntax against Terminal's dictionary; nothing runs.
        let dir = tempfile::tempdir().unwrap();
        for target in ["new", "tab", "current"] {
            let script = terminal_app_script(r#"echo "it's" \ ok"#, target, "Basic", "T");
            let output = Command::new("/usr/bin/osacompile")
                .args(["-o"])
                .arg(dir.path().join(format!("{target}.scpt")))
                .args(["-e", &script])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{target}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        assert!(terminal_app_script("ssh prod", "new", "Basic", "Prod")
            .contains("settings set \"Basic\""));

        assert!(iterm_script("ssh prod", false, "new", "Default").contains("iTerm2"));
        assert!(iterm_script("ssh prod", true, "tab", "Default").contains("create tab"));
        assert!(iterm_script("ssh prod", true, "current", "Default").contains("write text"));
    }
}
