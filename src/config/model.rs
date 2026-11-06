use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
pub struct Config {
    #[serde(default)]
    pub editor: Option<String>,
    #[serde(default)]
    pub launch_at_login: bool,
    #[serde(default)]
    pub terminal: Option<String>,
    #[serde(rename = "iTerm_version", default)]
    pub iterm_version: Option<String>,
    #[serde(default)]
    pub default_theme: Option<String>,
    #[serde(default)]
    pub open_in: Option<String>,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub strategy: Option<String>,
    #[serde(default)]
    pub show_ssh_config_hosts: Option<bool>,
    #[serde(default)]
    pub ssh_config_ignore_hosts: Vec<String>,
    #[serde(default)]
    pub ssh_config_ignore_keywords: Vec<String>,
    #[serde(default)]
    pub hosts: Vec<HostEntry>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum HostEntry {
    Command(CommandHost),
    Menu(std::collections::BTreeMap<String, Vec<HostEntry>>),
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct CommandHost {
    pub cmd: String,
    pub name: String,
    #[serde(rename = "inTerminal", default)]
    pub in_terminal: Option<String>,
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub strategy: Option<String>,
    /// Saved password to type for the command, e.g. `keychain:prod`.
    #[serde(default)]
    pub password_ref: Option<String>,
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, Value>,
}
