#![allow(deprecated, unexpected_cfgs)]

use crate::config::model::Config;
use crate::launcher::{Backend, ITermVersion, LaunchKind, LaunchTarget};
use crate::menu_model::MenuEntry;
#[cfg(not(test))]
use cocoa::appkit::{NSMenu, NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength};
#[cfg(not(test))]
use cocoa::base::{id, nil, NO, YES};
#[cfg(not(test))]
use cocoa::foundation::{NSAutoreleasePool, NSString};
#[cfg(not(test))]
use objc::{class, msg_send, sel, sel_impl};

// ── Public spec type (used in tests without AppKit) ──────────────────────────

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeMenuSpec {
    Menu {
        title: String,
        children: Vec<NativeMenuSpec>,
    },
    Command {
        title: String,
        enabled: bool,
    },
    Separator,
}

#[cfg(test)]
pub fn build_spec(entries: &[MenuEntry]) -> Vec<NativeMenuSpec> {
    entries
        .iter()
        .map(|e| match e {
            MenuEntry::Menu {
                title, children, ..
            } => NativeMenuSpec::Menu {
                title: title.clone(),
                children: build_spec(children),
            },
            MenuEntry::Command { title, .. } => NativeMenuSpec::Command {
                title: title.clone(),
                enabled: true,
            },
            MenuEntry::Disabled { title } => NativeMenuSpec::Command {
                title: title.clone(),
                enabled: false,
            },
            MenuEntry::Separator => NativeMenuSpec::Separator,
        })
        .collect()
}

// ── Native menu construction ──────────────────────────────────────────────────

/// Rebuild and replace the menu on an existing status item (called on hot reload).
#[cfg(all(target_os = "macos", not(test)))]
pub unsafe fn rebuild_menu(status_item: id, entries: &[MenuEntry], config: &Config, delegate: id) {
    use cocoa::appkit::NSStatusItem;
    let _pool = cocoa::foundation::NSAutoreleasePool::new(nil);
    let menu = build_full_menu(entries, config, delegate);
    status_item.setMenu_(menu);
}

#[cfg(all(target_os = "macos", not(test)))]
unsafe fn build_full_menu(entries: &[MenuEntry], config: &Config, delegate: id) -> id {
    let menu = build_ns_menu(entries, config, &[]);

    menu.addItem_(NSMenuItem::separatorItem(nil));

    // ── Configuration submenu ────────────────────────────────────────────────
    let config_submenu = NSMenu::new(nil).autorelease();
    let _: () = msg_send![config_submenu, setAutoenablesItems: NO];

    for (label, action) in [
        ("Configure...", sel!(shuttleConfigure:)),
        ("Import...", sel!(shuttleImport:)),
        ("Export...", sel!(shuttleExport:)),
        ("Check for Updates...", sel!(shuttleCheckForUpdates:)),
    ] {
        let item = titled_action_item(label, action);
        item.setTarget_(delegate);
        config_submenu.addItem_(item);
    }

    config_submenu.addItem_(NSMenuItem::separatorItem(nil));

    let about = titled_action_item("About Shuttle", sel!(orderFrontStandardAboutPanel:));
    config_submenu.addItem_(about);

    let config_header = NSMenuItem::alloc(nil)
        .initWithTitle_action_keyEquivalent_(
            NSString::alloc(nil).init_str("Configuration"),
            sel!(noop:),
            NSString::alloc(nil).init_str(""),
        )
        .autorelease();
    let gear_sym = NSString::alloc(nil).init_str("gearshape");
    let gear_img: id = msg_send![
        class!(NSImage),
        imageWithSystemSymbolName: gear_sym
        accessibilityDescription: nil
    ];
    if gear_img != nil {
        let _: () = msg_send![config_header, setImage: gear_img];
    }
    config_header.setSubmenu_(config_submenu);
    menu.addItem_(config_header);

    // ── Quit ─────────────────────────────────────────────────────────────────
    let quit = NSMenuItem::alloc(nil).initWithTitle_action_keyEquivalent_(
        NSString::alloc(nil).init_str("Quit"),
        sel!(terminate:),
        NSString::alloc(nil).init_str("q"),
    );
    menu.addItem_(quit);

    // Wire the delegate so menuWillOpen: fires for hot reload
    let _: () = msg_send![menu, setDelegate: delegate];
    menu
}

#[cfg(all(target_os = "macos", not(test)))]
pub fn install_status_menu(entries: &[MenuEntry], config: &Config, delegate: id) -> id {
    unsafe {
        let menu = build_full_menu(entries, config, delegate);

        // Build status item
        let status_bar = NSStatusBar::systemStatusBar(nil);
        let item = status_bar.statusItemWithLength_(NSVariableStatusItemLength);
        let button: id = item.button();
        if button != nil {
            let sym = NSString::alloc(nil).init_str("paperplane.fill");
            let image: id = msg_send![
                class!(NSImage),
                imageWithSystemSymbolName: sym
                accessibilityDescription: nil
            ];
            if image != nil {
                let _: () = msg_send![image, setTemplate: YES];
                let _: () = msg_send![button, setImage: image];
            } else {
                let title = NSString::alloc(nil).init_str("🚀");
                let _: () = msg_send![button, setTitle: title];
            }
        }
        item.setMenu_(menu);
        item
    }
}

// ── Recursive menu builder ────────────────────────────────────────────────────

#[cfg(all(target_os = "macos", not(test)))]
unsafe fn build_ns_menu(entries: &[MenuEntry], config: &Config, folders: &[String]) -> id {
    let menu = NSMenu::new(nil).autorelease();
    let _: () = msg_send![menu, setAutoenablesItems: NO];

    for entry in entries {
        match entry {
            MenuEntry::Menu {
                title, children, ..
            } => {
                let item = static_item(title, true);
                let mut path = folders.to_vec();
                path.push(title.clone());
                let submenu = build_ns_menu(children, config, &path);
                item.setSubmenu_(submenu);
                menu.addItem_(item);
            }
            MenuEntry::Command { title, command, .. } => {
                // Resolve the full launch payload so target/title/theme survive to execution.
                let mut path = folders.to_vec();
                path.push(title.clone());
                match resolve_launch_payload(config, command, &path) {
                    Ok(launch_payload) => {
                        let item = command_item(title, &command.cmd, &launch_payload);
                        menu.addItem_(item);
                    }
                    Err(error) => {
                        menu.addItem_(static_item(&format!("{title}: {error}"), false));
                    }
                }
            }
            MenuEntry::Disabled { title } => {
                menu.addItem_(static_item(title, false));
            }
            MenuEntry::Separator => {
                menu.addItem_(NSMenuItem::separatorItem(nil));
            }
        }
    }
    menu
}

/// Build an NSMenuItem that fires ShuttleAction.launch: on click.
#[cfg(all(target_os = "macos", not(test)))]
unsafe fn command_item(title: &str, cmd: &str, backend: &str) -> id {
    let action = crate::macos::action::create_action(cmd, backend);
    let item = NSMenuItem::alloc(nil)
        .initWithTitle_action_keyEquivalent_(
            NSString::alloc(nil).init_str(title),
            sel!(launch:),
            NSString::alloc(nil).init_str(""),
        )
        .autorelease();
    item.setTarget_(action);
    let _: () = msg_send![item, setEnabled: YES];
    item
}

/// Build a non-clickable (or action-less) NSMenuItem.
#[cfg(all(target_os = "macos", not(test)))]
unsafe fn static_item(title: &str, enabled: bool) -> id {
    let item = NSMenuItem::alloc(nil)
        .initWithTitle_action_keyEquivalent_(
            NSString::alloc(nil).init_str(title),
            sel!(noop:),
            NSString::alloc(nil).init_str(""),
        )
        .autorelease();
    let flag = if enabled { YES } else { NO };
    let _: () = msg_send![item, setEnabled: flag];
    item
}

#[cfg(all(target_os = "macos", not(test)))]
unsafe fn titled_action_item(title: &str, action: objc::runtime::Sel) -> id {
    NSMenuItem::alloc(nil).initWithTitle_action_keyEquivalent_(
        NSString::alloc(nil).init_str(title),
        action,
        NSString::alloc(nil).init_str(""),
    )
}

// ── Backend resolution ────────────────────────────────────────────────────────

/// Returns the executor backend string for a given command and config.
// NOTE: The ObjC action target stores this serialized payload on NSMenuItem
// actions, so `executor::execute` must deserialize the same backend/target
// strings below. Keep both sides in sync when adding launcher variants.
/// `menu_path` is the host's place in the menu, ending with its own title.
fn resolve_launch_payload(
    config: &Config,
    host: &crate::config::model::CommandHost,
    menu_path: &[String],
) -> Result<String, crate::launcher::LaunchError> {
    use crate::launcher::password::{self, PasswordSetting};

    match crate::launcher::normalize(config, host, &host.name)? {
        LaunchKind::Url(url) => Ok(format!("url:{url}")),
        LaunchKind::Terminal(req) => {
            let (password_ref, password_setup) = match password::from_host(host)? {
                None => (None, None),
                Some(PasswordSetting::Saved(saved)) => (Some(saved.to_config_value()), None),
                Some(PasswordSetting::NeedsSetup) => (
                    None,
                    Some(serde_json::json!({ "name": host.name, "path": menu_path })),
                ),
            };
            Ok(serde_json::json!({
                "kind": "terminal",
                "backend": backend_to_str(&req.backend),
                "target": target_to_str(&req.target),
                "title": req.title,
                "theme": req.theme_or_profile,
                "password_ref": password_ref,
                "password_setup": password_setup,
            })
            .to_string())
        }
    }
}

fn backend_to_str(backend: &Backend) -> String {
    match backend {
        Backend::TerminalApp => "terminal-app".to_string(),
        Backend::ITerm {
            version: ITermVersion::Stable,
        } => "iterm-stable".to_string(),
        Backend::ITerm {
            version: ITermVersion::Nightly,
        } => "iterm-nightly".to_string(),
        Backend::GhosttyOpen => "ghostty-open".to_string(),
        Backend::GhosttyAppleScript => "ghostty-applescript".to_string(),
        Backend::CmuxCli => "cmux-cli".to_string(),
        Backend::CmuxSocket => "cmux-socket".to_string(),
        Backend::Screen => "screen".to_string(),
    }
}

fn target_to_str(target: &LaunchTarget) -> &'static str {
    match target {
        LaunchTarget::New => "new",
        LaunchTarget::Tab => "tab",
        LaunchTarget::Current => "current",
        LaunchTarget::Virtual => "virtual",
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::CommandHost;
    use std::collections::BTreeMap;

    #[test]
    fn converts_menu_model_to_native_spec() {
        let entries = vec![
            MenuEntry::Disabled {
                title: "Error parsing config".into(),
            },
            MenuEntry::Separator,
            MenuEntry::Command {
                title: "Prod".into(),
                command: CommandHost {
                    cmd: "ssh prod".into(),
                    name: "Prod".into(),
                    in_terminal: None,
                    theme: None,
                    title: None,
                    backend: None,
                    strategy: None,
                    password_ref: None,
                    extra: BTreeMap::new(),
                },
                separator_after: false,
            },
        ];
        assert_eq!(
            build_spec(&entries),
            vec![
                NativeMenuSpec::Command {
                    title: "Error parsing config".into(),
                    enabled: false
                },
                NativeMenuSpec::Separator,
                NativeMenuSpec::Command {
                    title: "Prod".into(),
                    enabled: true
                },
            ]
        );
    }

    #[test]
    fn url_cmds_resolve_to_url_backend() {
        let config = Config::default();
        let host = CommandHost {
            cmd: "https://example.com".into(),
            name: "Website".into(),
            in_terminal: None,
            theme: None,
            title: None,
            backend: None,
            strategy: None,
            password_ref: None,
            extra: BTreeMap::new(),
        };
        assert!(resolve_launch_payload(&config, &host, &[])
            .unwrap()
            .starts_with("url:"));
    }

    #[test]
    fn ghostty_terminal_key_resolves_to_ghostty_applescript() {
        let config = Config {
            terminal: Some("ghostty".into()),
            ..Config::default()
        };
        let host = CommandHost {
            cmd: "ssh prod".into(),
            name: "Prod".into(),
            in_terminal: None,
            theme: None,
            title: None,
            backend: None,
            strategy: None,
            password_ref: None,
            extra: BTreeMap::new(),
        };
        assert!(resolve_launch_payload(&config, &host, &[])
            .unwrap()
            .contains("\"backend\":\"ghostty-applescript\""));
    }

    #[test]
    fn virtual_urls_resolve_to_terminal_payload() {
        let config = Config::default();
        let host = CommandHost {
            cmd: "https://example.com".into(),
            name: "Website".into(),
            in_terminal: Some("virtual".into()),
            theme: None,
            title: None,
            backend: None,
            strategy: None,
            password_ref: None,
            extra: BTreeMap::new(),
        };
        let payload = resolve_launch_payload(&config, &host, &[]).unwrap();
        assert!(!payload.starts_with("url:"));
        assert!(payload.contains("\"backend\":\"screen\""));
        assert!(payload.contains("\"target\":\"virtual\""));
    }

    #[test]
    fn terminal_payload_preserves_title_theme_and_target() {
        let config = Config {
            terminal: Some("iTerm".into()),
            open_in: Some("current".into()),
            ..Config::default()
        };
        let host = CommandHost {
            cmd: "ssh prod".into(),
            name: "Prod".into(),
            in_terminal: None,
            theme: Some("Homebrew".into()),
            title: Some("Production".into()),
            backend: None,
            strategy: None,
            password_ref: None,
            extra: BTreeMap::new(),
        };
        let payload = resolve_launch_payload(&config, &host, &[]).unwrap();
        assert!(payload.contains("\"backend\":\"iterm-stable\""));
        assert!(payload.contains("\"target\":\"current\""));
        assert!(payload.contains("\"title\":\"Production\""));
        assert!(payload.contains("\"theme\":\"Homebrew\""));
    }

    #[test]
    fn terminal_payload_carries_password_ref() {
        let mut host = CommandHost {
            cmd: "ssh prod".into(),
            name: "Prod".into(),
            in_terminal: None,
            theme: None,
            title: None,
            backend: None,
            strategy: None,
            password_ref: None,
            extra: BTreeMap::new(),
        };
        let config = Config::default();
        assert!(resolve_launch_payload(&config, &host, &[])
            .unwrap()
            .contains("\"password_ref\":null"));

        host.password_ref = Some(" keychain: prod ".into());
        assert!(resolve_launch_payload(&config, &host, &[])
            .unwrap()
            .contains("\"password_ref\":\"keychain:prod\""));

        host.password_ref = Some("hunter2".into());
        let error = resolve_launch_payload(&config, &host, &[]).unwrap_err();
        assert_eq!(error, crate::launcher::LaunchError::PasswordRef);
        assert!(!error.to_string().contains("hunter2"));

        host.password_ref = Some(String::new());
        host.name = "[aaa]Prod".into();
        let payload: serde_json::Value = serde_json::from_str(
            &resolve_launch_payload(&config, &host, &["EL".into(), "Prod".into()]).unwrap(),
        )
        .unwrap();
        assert_eq!(payload["password_ref"], serde_json::Value::Null);
        assert_eq!(
            payload["password_setup"],
            serde_json::json!({ "name": "[aaa]Prod", "path": ["EL", "Prod"] })
        );
    }

    #[test]
    fn invalid_launch_config_returns_error() {
        let config = Config {
            backend: Some("nope".into()),
            ..Config::default()
        };
        let host = CommandHost {
            cmd: "ssh prod".into(),
            name: "Prod".into(),
            in_terminal: None,
            theme: None,
            title: None,
            backend: None,
            strategy: None,
            password_ref: None,
            extra: BTreeMap::new(),
        };
        assert!(resolve_launch_payload(&config, &host, &[]).is_err());
    }
}
