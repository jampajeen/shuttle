use crate::config::model::{CommandHost, HostEntry};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SshHost {
    pub alias: String,
    pub metadata: BTreeMap<String, String>,
}

pub fn parse_file(path: &Path) -> std::io::Result<BTreeMap<String, SshHost>> {
    let contents = fs::read_to_string(path)?;
    Ok(parse_str(
        &contents,
        path.parent().unwrap_or_else(|| Path::new(".")),
    ))
}

pub fn parse_str(contents: &str, base_dir: &Path) -> BTreeMap<String, SshHost> {
    let mut hosts: BTreeMap<String, SshHost> = BTreeMap::new();
    let mut current_alias: Option<String> = None;

    for line in contents.lines() {
        let trimmed = line.trim();
        let Some((is_comment, key, value)) = parse_line(trimmed) else {
            continue;
        };

        if is_comment {
            if let (Some(alias), Some(metadata_key)) =
                (&current_alias, key.strip_prefix("shuttle."))
            {
                if let Some(host) = hosts.get_mut(alias) {
                    host.metadata
                        .insert(metadata_key.to_string(), value.to_string());
                }
            }
            continue;
        }

        if key.eq_ignore_ascii_case("include") {
            for include in expand_include(base_dir, value) {
                if let Ok(included) = parse_file(&include) {
                    hosts.extend(included);
                }
            }
            continue;
        }

        if key.eq_ignore_ascii_case("host") {
            if let Some(alias) = value.split_whitespace().next() {
                current_alias = Some(alias.to_string());
                hosts.entry(alias.to_string()).or_insert_with(|| SshHost {
                    alias: alias.to_string(),
                    metadata: BTreeMap::new(),
                });
            }
        }
    }

    hosts
}

pub fn should_include(name: &str, ignored_hosts: &[String], ignored_keywords: &[String]) -> bool {
    !name.contains('*')
        && !name.starts_with('.')
        && !ignored_hosts.iter().any(|ignored| ignored == name)
        && !ignored_keywords
            .iter()
            .any(|keyword| name.contains(keyword))
}

pub fn merge_hosts(
    root: &mut Vec<HostEntry>,
    hosts: &BTreeMap<String, SshHost>,
    ignored_hosts: &[String],
    ignored_keywords: &[String],
) {
    for (alias, host) in hosts {
        let name = host
            .metadata
            .get("name")
            .map_or(alias.as_str(), String::as_str);
        if !should_include(name, ignored_hosts, ignored_keywords) {
            continue;
        }
        let password_ref = host.metadata.get("password_ref").cloned();
        insert_path(root, name, alias, password_ref);
    }
}

fn insert_path(root: &mut Vec<HostEntry>, name: &str, alias: &str, password_ref: Option<String>) {
    let mut parts: Vec<&str> = name.split('/').collect();
    let Some(leaf) = parts.pop() else {
        return;
    };
    let mut current = root;
    for part in parts {
        current = child_menu(current, part);
    }
    current.push(HostEntry::Command(CommandHost {
        cmd: format!("ssh {alias}"),
        name: leaf.to_string(),
        in_terminal: None,
        theme: None,
        title: None,
        backend: None,
        strategy: None,
        password_ref,
        extra: BTreeMap::new(),
    }));
}

fn child_menu<'a>(entries: &'a mut Vec<HostEntry>, name: &str) -> &'a mut Vec<HostEntry> {
    let index = entries
        .iter()
        .position(|entry| matches!(entry, HostEntry::Menu(map) if map.contains_key(name)))
        .unwrap_or_else(|| {
            entries.push(HostEntry::Menu(BTreeMap::from([(
                name.to_string(),
                Vec::new(),
            )])));
            entries.len() - 1
        });

    match &mut entries[index] {
        HostEntry::Menu(map) => map.get_mut(name).expect("menu exists"),
        HostEntry::Command(_) => unreachable!(),
    }
}

fn parse_line(trimmed: &str) -> Option<(bool, &str, &str)> {
    if trimmed.is_empty() {
        return None;
    }
    let (is_comment, body) = trimmed
        .strip_prefix('#')
        .map_or((false, trimmed), |body| (true, body.trim_start()));
    let split = body.find(char::is_whitespace).or_else(|| body.find('='))?;
    let key = &body[..split];
    let value = body[split..].trim_start_matches(|ch: char| ch.is_whitespace() || ch == '=');
    if key.is_empty() || value.is_empty() {
        None
    } else {
        Some((is_comment, key, value))
    }
}

fn expand_include(base_dir: &Path, value: &str) -> Vec<PathBuf> {
    value
        .split_whitespace()
        .flat_map(|path| {
            let path = expand_tilde(path);
            let path = if path.is_absolute() {
                path
            } else {
                base_dir.join(path)
            };
            expand_glob(path)
        })
        .collect()
}

fn expand_glob(path: PathBuf) -> Vec<PathBuf> {
    let pattern = path.to_string_lossy();
    if !pattern.contains(['*', '?', '[']) {
        return vec![path];
    }

    let Ok(paths) = glob::glob(&pattern) else {
        return vec![path];
    };
    let mut paths: Vec<PathBuf> = paths.filter_map(Result::ok).collect();
    paths.sort();
    paths
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        dirs::home_dir().map_or_else(|| PathBuf::from(path), |home| home.join(rest))
    } else {
        PathBuf::from(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_host_first_alias_and_shuttle_comments() {
        let parsed = parse_str(
            "Host prod prod.example\n  # shuttle.name Servers/Prod\n",
            Path::new("."),
        );
        assert_eq!(parsed["prod"].metadata["name"], "Servers/Prod");
        assert!(!parsed.contains_key("prod.example"));
    }

    #[test]
    fn filters_legacy_ignored_hosts() {
        assert!(!should_include("*.example", &[], &[]));
        assert!(!should_include(".hidden", &[], &[]));
        assert!(!should_include("prod", &["prod".into()], &[]));
        assert!(!should_include("prod-old", &[], &["old".into()]));
        assert!(should_include("prod", &[], &[]));
    }

    #[test]
    fn merges_slash_paths_into_menu_tree() {
        let mut root = Vec::new();
        let hosts = BTreeMap::from([(
            "prod".to_string(),
            SshHost {
                alias: "prod".into(),
                metadata: BTreeMap::from([("name".into(), "Servers/Prod".into())]),
            },
        )]);
        merge_hosts(&mut root, &hosts, &[], &[]);
        assert!(matches!(&root[0], HostEntry::Menu(map) if map.contains_key("Servers")));
    }

    #[test]
    fn imports_password_ref_metadata() {
        let parsed = parse_str(
            "Host prod\n  # shuttle.password_ref keychain:prod\nHost dev\n",
            Path::new("."),
        );
        let mut root = Vec::new();
        merge_hosts(&mut root, &parsed, &[], &[]);
        let refs: Vec<_> = root
            .iter()
            .map(|entry| match entry {
                HostEntry::Command(host) => (host.cmd.as_str(), host.password_ref.as_deref()),
                HostEntry::Menu(_) => panic!("command expected"),
            })
            .collect();
        assert_eq!(
            refs,
            [("ssh dev", None), ("ssh prod", Some("keychain:prod"))]
        );
    }

    #[test]
    fn expands_include_globs_in_lexical_order() {
        let temp = tempfile::tempdir().unwrap();
        let include_dir = temp.path().join("conf.d");
        fs::create_dir_all(&include_dir).unwrap();
        fs::write(include_dir.join("b.conf"), "Host beta\n").unwrap();
        fs::write(include_dir.join("a.conf"), "Host alpha\n").unwrap();

        let parsed = parse_str("Include conf.d/*.conf\n", temp.path());
        let aliases: Vec<_> = parsed.keys().cloned().collect();
        assert_eq!(aliases, ["alpha", "beta"]);
    }
}
