use crate::config::model::{CommandHost, HostEntry};

#[derive(Debug, Clone, PartialEq)]
pub enum MenuEntry {
    Menu {
        title: String,
        children: Vec<MenuEntry>,
        separator_after: bool,
    },
    Command {
        title: String,
        command: CommandHost,
        separator_after: bool,
    },
    Separator,
    Disabled {
        title: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayName {
    pub title: String,
    pub separator_after: bool,
}

pub fn build(entries: &[HostEntry]) -> Vec<MenuEntry> {
    let mut menus: Vec<(String, Vec<HostEntry>)> = Vec::new();
    let mut commands: Vec<CommandHost> = Vec::new();

    for entry in entries {
        match entry {
            HostEntry::Command(command) => commands.push(command.clone()),
            HostEntry::Menu(map) => {
                for (name, children) in map {
                    menus.push((name.clone(), children.clone()));
                }
            }
        }
    }

    menus.sort_by(|(left, _), (right, _)| cmp_legacy(left, right));
    commands.sort_by(|left, right| cmp_legacy(&left.name, &right.name));

    let mut normalized = Vec::with_capacity(menus.len() + commands.len());
    for (name, children) in menus {
        let display = display_name(&name);
        normalized.push(MenuEntry::Menu {
            title: display.title,
            children: build(&children),
            separator_after: display.separator_after,
        });
    }
    for command in commands {
        let display = display_name(&command.name);
        normalized.push(MenuEntry::Command {
            title: display.title,
            command,
            separator_after: display.separator_after,
        });
    }
    normalized
}

pub fn display_name(name: &str) -> DisplayName {
    let mut title = name.to_string();
    let mut separator_after = false;
    if title.contains("[---]") {
        title = title.replace("[---]", "");
        separator_after = true;
    }
    title = strip_sort_marker(&title);
    DisplayName {
        title,
        separator_after,
    }
}

fn strip_sort_marker(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut chars = name.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '[' {
            let marker: Vec<char> = chars.clone().take(4).collect();
            if marker.len() == 4
                && marker[3] == ']'
                && marker[..3].iter().all(|ch| ch.is_ascii_lowercase())
            {
                for _ in 0..4 {
                    chars.next();
                }
                continue;
            }
        }
        out.push(ch);
    }

    out
}

fn cmp_legacy(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_lowercase().cmp(&right.to_lowercase())
}

pub fn error_menu(title: impl Into<String>) -> Vec<MenuEntry> {
    vec![MenuEntry::Disabled {
        title: title.into(),
    }]
}

pub fn with_separators(entries: Vec<MenuEntry>) -> Vec<MenuEntry> {
    let mut result = Vec::new();
    for entry in entries {
        let separator_after = match &entry {
            MenuEntry::Menu {
                separator_after, ..
            }
            | MenuEntry::Command {
                separator_after, ..
            } => *separator_after,
            MenuEntry::Separator | MenuEntry::Disabled { .. } => false,
        };
        result.push(entry);
        if separator_after {
            result.push(MenuEntry::Separator);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::CommandHost;
    use std::collections::BTreeMap;

    fn command(name: &str) -> HostEntry {
        HostEntry::Command(CommandHost {
            cmd: format!("echo {name}"),
            name: name.to_string(),
            in_terminal: None,
            theme: None,
            title: None,
            backend: None,
            strategy: None,
            password_ref: None,
            extra: BTreeMap::new(),
        })
    }

    #[test]
    fn strips_sort_and_separator_markers() {
        assert_eq!(
            display_name("[aaa]Prod[---]"),
            DisplayName {
                title: "Prod".into(),
                separator_after: true
            }
        );
    }

    #[test]
    fn strip_sort_marker_preserves_utf8() {
        assert_eq!(display_name("[aaa]Café").title, "Café");
        assert_eq!(display_name("[bbb]日本語").title, "日本語");
    }

    #[test]
    fn sorts_menus_before_commands_independently() {
        let entries = vec![
            command("z leaf"),
            HostEntry::Menu(BTreeMap::from([(
                "b menu".to_string(),
                vec![command("child")],
            )])),
            command("a leaf"),
            HostEntry::Menu(BTreeMap::from([("a menu".to_string(), vec![])])),
        ];
        let titles: Vec<_> = build(&entries)
            .into_iter()
            .map(|entry| match entry {
                MenuEntry::Menu { title, .. } | MenuEntry::Command { title, .. } => title,
                MenuEntry::Separator => "-".into(),
                MenuEntry::Disabled { title } => title,
            })
            .collect();
        assert_eq!(titles, ["a menu", "b menu", "a leaf", "z leaf"]);
    }

    #[test]
    fn creates_disabled_error_menu_item() {
        assert_eq!(
            error_menu("Error parsing config"),
            vec![MenuEntry::Disabled {
                title: "Error parsing config".into()
            }]
        );
    }

    #[test]
    fn inserts_separator_after_marked_items() {
        assert!(matches!(
            &with_separators(build(&[command("[---]leaf")]))[1],
            MenuEntry::Separator
        ));
    }
}
