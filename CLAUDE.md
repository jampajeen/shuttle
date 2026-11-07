@AGENTS.md

# Fork notes: jampajeen/shuttle

This is a personal fork of [philipbrembeck/shuttle](https://github.com/philipbrembeck/shuttle), the Rust rewrite of fitztrev/shuttle. `AGENTS.md`, imported above, is upstream's rulebook and applies here unchanged. This file only adds what is specific to the fork. It is fork-only: never include it in an upstream PR.

## Remotes and branches

- `origin` is `git@github.com:jampajeen/shuttle.git`, the fork, with default branch `main`. `upstream` is `https://github.com/philipbrembeck/shuttle`, branch `main`.
- Fork-only commits must never go upstream:
  - `1e3fce6` "chore: point updater and docs to personal fork": the updater URL points at `jampajeen/shuttle`, the README has a fork note, the upstream `request-review.yml` workflow is removed, and `.claude/settings.local.json` is gitignored.
  - The commit that adds this `CLAUDE.md`.
- To take upstream changes, merge `upstream/main` on a branch, or use **Sync fork** on GitHub. The fork was re-created from philipbrembeck, so Sync fork pulls from the right repo.

## Sending work upstream

1. Branch from `upstream/main` and cherry-pick only the commits that upstream should have.
2. Make sure no fork-specific text slipped in: `git grep jampajeen` must find nothing.
3. Run the checks, push the branch to `origin`, and open the PR with base `philipbrembeck/shuttle:main`.

Keep fixes that upstream also needs in their own commits, separate from fork features, so they can be cherry-picked. A whole-file split is not required: picking hunks into the index with `git apply --cached` works.

## Current state (2026-11-07)

- `feat/keychain-ssh-passwords` is pushed but not merged into `main`:
  - `b5af630`: `password_ref: "keychain:<account>"`, with ssh answered through `SSH_ASKPASS` by the Shuttle binary itself.
  - `9357219`: Terminal.app `tab` mode opens a real tab, and the window keeps its size when a theme is applied.
  - `7806499`: `"password_ref": ""` starts password setup from the menu; also adds the HOME test lock in `src/test_support.rs`.
- `feat/keychain-ssh-passwords-upstream` is pushed: `4248ae0` (same change as `b5af630`) and `58aa71d` (same as `9357219`), for a PR to upstream that is **not opened yet**.
- Upstream's `macos::state` tests are flaky, failing about 1 run in 5 on a Mac with `~/.ssh/config`, because they read HOME while a `main.rs` test changes it. The fix is in `7806499`; the user chose not to send it upstream.
- Later: `cocoa`/`objc` are deprecated, and `block v0.1.6` already prints a future-incompatibility warning. Migrate to `objc2` when they break or before any App Store work.
- Design and alternatives for passwords are in `docs/ADR/0005-keychain-ssh-passwords.md`.

## Working with the user

- The user writes in Thai; answer in Thai unless they switch.
- **Ask before copying a build into `/Applications`.** Every ad-hoc signed install counts as a new app to macOS, so Accessibility, which Terminal.app tab mode needs, must be granted again. Building to `target/release/Shuttle.app` and testing from there needs no permission.
- Commit and push only when asked. When asked to commit, keep upstream-worthy fixes in separate commits.
- The user's real Shuttle config is `~/.config/shuttle/config.json` (folders `EL` and `Client Projects`; `EL → Local → Local Server` uses `keychain:el-local`). Don't change it without asking, except for temporary test hosts that you back up first and remove afterwards.

## Environment

- **Real verification needs the Mac.** `src/macos/` (menu, executor, AppleScript) only compiles on macOS, and Keychain, `/usr/bin/security`, Terminal.app and app bundles exist only there. Cloud or Linux sessions can work on config, SSH import, launcher, `askpass` and `password_setup` logic, and docs. They must say plainly that the macOS paths are unverified, and `./scripts/check-rust.sh` may not pass there unchanged.
- Rust on the Mac is Homebrew's keg-only `rustup`, so prefix commands with `PATH="/opt/homebrew/opt/rustup/bin:$PATH"`.
- To build the app bundle, run `./scripts/build-rust-app.sh`, then `codesign --force --deep --sign - target/release/Shuttle.app` as the release workflow does.

## Testing password features end to end

- **Throwaway password-only SSH server:**
  ```dockerfile
  FROM alpine:3.20
  RUN apk add --no-cache openssh && ssh-keygen -A \
   && adduser -D -s /bin/sh shuttletest \
   && printf 'PasswordAuthentication yes\nKbdInteractiveAuthentication no\nPubkeyAuthentication no\nPermitRootLogin no\n' >> /etc/ssh/sshd_config
  CMD ["/usr/sbin/sshd", "-D", "-e"]
  ```
  Run it with `docker run -d -p 127.0.0.1:2222:22`, set a random password through `chpasswd`, and save the same password under a test account with `security add-generic-password -U -s Shuttle -a <test-account> -w`. Never print passwords; to let the user type one, put it on the clipboard with `pbcopy` and clear it afterwards.
- Point ssh at a scratch `UserKnownHostsFile`, never the user's `~/.ssh/known_hosts`.
- Drive terminal prompts with a pseudo-terminal (Python `pty`) and redact secrets from the output. Without a tty, askpass cannot ask the user, and ssh then sends empty passwords.
- Count `Accepted password` and `Failed password` in `docker logs` to confirm what reached the server.
- Clean up everything afterwards: the container, both images (`shuttle-sshd-test` and `alpine:3.20`), test Keychain items, and test hosts in the config.
