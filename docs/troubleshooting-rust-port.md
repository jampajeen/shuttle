# Rust Port Troubleshooting

## Config parse errors

If the menu shows or logs an invalid JSON error, validate your config:

```sh
python3 -m json.tool ~/.shuttle.json >/dev/null
```

If you use `~/.shuttle.path`, validate the file named in that path instead.

## Automation permission

AppleScript backends require macOS Automation permission. If Terminal.app, iTerm, or Ghostty AppleScript actions fail:

1. Open System Settings → Privacy & Security → Automation.
2. Find Shuttle.
3. Enable the target terminal application.
4. Restart Shuttle and retry.

### Terminal.app tabs open as new windows

Terminal.app has no AppleScript command for a new tab, so `tab` mode presses ⌘T through System Events. That needs two more permissions for Shuttle:

1. System Settings → Privacy & Security → **Accessibility**: turn on Shuttle.
2. System Settings → Privacy & Security → **Automation**: under Shuttle, turn on **System Events**.

Without them, Shuttle opens the command in a new window instead. A locally rebuilt, ad-hoc signed Shuttle counts as a new app, so grant Accessibility again after each rebuild (remove the old Shuttle entry first).

## Missing terminal apps

- `ghostty-open` and `ghostty-applescript` require `Ghostty.app` in `/Applications`.
- iTerm backends require iTerm installed and the matching `iTerm_version` / backend value.
- `screen` mode requires `/usr/bin/screen` or a compatible `screen` in PATH.

## Saved SSH passwords (`password_ref`)

- **`no password saved in Keychain for account …; type the password instead`**: save one with `security add-generic-password -s Shuttle -a <account> -w`. The account must match the part after `keychain:` exactly, and the service must be `Shuttle`.
- **`the saved password was not accepted; type the password instead`**: the server rejected the saved password. Type it this time, then update the saved one with `security add-generic-password -U -s Shuttle -a <account> -w`.
- **A Keychain dialog asks whether `security` may use the item**: the item was created in Keychain Access or by another app. Choose **Always Allow**, or delete it and re-create it with the `security` command above.
- **ssh still asks for the password every time**: the command does not run OpenSSH 8.4 or later (for example an old `ssh` earlier in `PATH`), so `SSH_ASKPASS_REQUIRE=force` is ignored. Check with `ssh -V`.
- **The terminal stops showing what you type**: an askpass prompt was interrupted while it had echo turned off. Run `stty echo` or `reset`.
- **The menu shows `invalid password_ref`**: the value must be `keychain:<account>`, with the account name, not the password.

## cmux socket access

`cmux-socket` uses a Unix socket, defaulting to `/tmp/cmux.sock`. Override it with:

```sh
export CMUX_SOCKET_PATH=/path/to/cmux.sock
```

If cmux blocks external local clients, start/configure cmux with:

```sh
CMUX_SOCKET_MODE=allowAll
```

Use `cmux-cli` if you cannot enable socket access.
