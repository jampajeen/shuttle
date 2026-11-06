# ADR 0005: Saved SSH passwords from the macOS Keychain

## Status

Accepted

## Context

Some servers only accept password login. Typing the password each time is tedious, and storing it in the Shuttle config would put a secret in a plain file that people sync and share. The user wants the password kept somewhere safe and typed for them, while sessions still open in their usual terminal, without installing extra tools.

## Decision

- Hosts may set `password_ref: "keychain:<account>"` (or `# shuttle.password_ref keychain:<account>` in SSH config). The account names a generic password in the login Keychain under the fixed service `Shuttle`.
- The menu app validates the reference when it builds the menu and carries it to execution in the launch payload. It never reads the password.
- At launch, the host's command is wrapped in `/bin/sh -c '<fixed script>' shuttle <askpass program> <password_ref> <command>`. The script creates a private state directory with `mktemp -d`, exports `SSH_ASKPASS=<Shuttle executable>`, `SSH_ASKPASS_REQUIRE=force`, `SHUTTLE_PASSWORD_REF` and `SHUTTLE_ASKPASS_STATE` for the command only, runs it, and removes the directory when it ends. All values reach the script as positional arguments, so the script text is a constant.
- ssh (OpenSSH 8.4 or later) then runs the Shuttle executable with the prompt as its argument. `main` detects `SHUTTLE_PASSWORD_REF` and runs askpass mode before any AppKit setup:
  - The first prompt ending in `password:` from a given ssh process is answered with the Keychain item, read with `/usr/bin/security find-generic-password -w` through an argument vector, and written to stdout.
  - A marker file named after the ssh process ID in the state directory limits this to one automatic try per connection. A second password prompt means the saved password was rejected, so the user is asked to type it.
  - Every other prompt (host key confirmation, key passphrases, one-time codes) goes to the user through `/dev/tty`, with echo turned off unless it is a yes/no question. `SSH_ASKPASS_PROMPT=confirm` and `none` are handled as a yes/no exit status and a notice.
  - When no item is saved, the user is told so and asked to type the password.
- The `invalid password_ref` error never repeats the configured value, in case someone put the password itself there.
- `"password_ref": ""` asks for setup. The menu passes the host's raw `name` and menu path in the launch payload, and the terminal runs `shuttle-rs --setup-password <name> <command> <menu path…>` instead of the command:
  - It suggests a Keychain account made from the menu path (`EL → Local → Local Server` gives `el-local-local-server`) and lets the user type another name.
  - If that account already exists, the user can use it, replace its password (`-U`), choose another name, or quit.
  - `/usr/bin/security add-generic-password … -w` runs with the terminal as its input, so `security` asks for the password and Shuttle never sees it.
  - It then sets `"keychain:<account>"` in the config and asks whether to connect, which runs the command with askpass.
- Writing back edits only the `""` value in place. Shuttle scans the JSON text for `"password_ref": ""`, parses each enclosing object, and edits the file only when exactly one object in the main or alternate JSON config has the host's `name` and normalized `cmd`. It writes a temporary file next to the real one, resolving symlinks, keeps its permissions, and renames it over. It does not re-serialize the config, because that would drop the user's formatting and key order. YAML configs and SSH config hosts get the line to add instead.

## Alternatives considered

- **`sshpass`**: the first implementation of this ADR. It needs a separate Homebrew install, is GPL-2.0 (a concern for bundling in a distributed app), passes the password in the `SSHPASS` environment variable, and stops on unknown host keys. Replaced by askpass.
- **Shuttle reads the password and sends it to the terminal**: the secret would pass through the AppleScript or CLI that drives the terminal and could end up typed into the visible session. Rejected.
- **iCloud Keychain**: synchronizable Keychain items need the data protection keychain, which requires a `keychain-access-groups` entitlement and an Apple Developer signing identity. Release builds are ad-hoc signed and notarization is not planned, so this is not available now. The askpass design carries over: a signed build could read synchronizable items in askpass mode.
- **`expect` scripts**: more moving parts and more fragile prompt matching.
- **1Password or Bitwarden CLIs**: they sync across Macs but need an extra account and CLI setup. The `<scheme>:<value>` format of `password_ref` leaves room to add them later as new schemes.

## Consequences

- No extra software is needed; the system `ssh` supports `SSH_ASKPASS_REQUIRE`. An older `ssh` earlier in `PATH` ignores it and asks for the password as usual.
- `password_ref` only helps ssh-based commands, not other programs that ask for passwords.
- The Keychain item must be saved on each Mac.
- Launched commands become a longer `/bin/sh -c …` line in the terminal. It contains the Keychain account name but never the password.
- The askpass path is the Shuttle executable's location at launch time, so moving the app while a session is connecting can break that connection's prompt.
- If a hidden askpass prompt is interrupted, the terminal may be left with echo off until the user runs `stty echo`.
- Password setup needs a visible terminal, so it is not useful for hosts in `virtual` (`screen`) mode.
- Without a controlling terminal, a prompt Shuttle cannot answer fails, and ssh then sends an empty password. Every Shuttle backend runs commands in a terminal or `screen` pty, so this only affects running the wrapped command by hand without one.
