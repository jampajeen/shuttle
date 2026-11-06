use serde::Deserialize;
use std::cmp::Ordering;
#[cfg(not(test))]
use std::process::Command;

#[cfg(not(test))]
const LATEST_RELEASE_API: &str = "https://api.github.com/repos/jampajeen/shuttle/releases/latest";
const RELEASES_URL: &str = "https://github.com/jampajeen/shuttle/releases/latest";

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: Option<String>,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    Available {
        latest: String,
        page_url: String,
        download_url: String,
    },
    Current {
        latest: String,
    },
}

#[cfg(not(test))]
pub fn check_latest_release(current_version: &str) -> Result<UpdateStatus, String> {
    let output = Command::new("/usr/bin/curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--connect-timeout",
            "10",
            "--max-time",
            "20",
            "--user-agent",
            concat!("Shuttle/", env!("CARGO_PKG_VERSION")),
            LATEST_RELEASE_API,
        ])
        .output()
        .map_err(|error| format!("Could not start curl: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("GitHub request failed with status {}", output.status)
        } else {
            stderr
        });
    }

    status_from_release_json(current_version, &output.stdout)
}

fn status_from_release_json(current_version: &str, json: &[u8]) -> Result<UpdateStatus, String> {
    let release: GitHubRelease = serde_json::from_slice(json)
        .map_err(|error| format!("GitHub returned an unexpected response: {error}"))?;
    status_from_release(current_version, release)
}

fn status_from_release(
    current_version: &str,
    release: GitHubRelease,
) -> Result<UpdateStatus, String> {
    let latest = release.tag_name.trim_start_matches('v').to_string();
    let page_url = release.html_url.unwrap_or_else(|| RELEASES_URL.to_string());
    let download_url = release
        .assets
        .iter()
        .find(|asset| asset.name == "Shuttle.zip")
        .map(|asset| asset.browser_download_url.clone())
        .ok_or_else(|| "Latest GitHub release has no Shuttle.zip asset".to_string())?;

    if compare_versions(&latest, current_version) == Ordering::Greater {
        Ok(UpdateStatus::Available {
            latest,
            page_url,
            download_url,
        })
    } else {
        Ok(UpdateStatus::Current { latest })
    }
}

#[cfg(all(target_os = "macos", not(test)))]
pub fn check_for_updates_async() {
    std::thread::spawn(|| {
        let current = env!("CARGO_PKG_VERSION");
        let script = match check_latest_release(current) {
            Ok(UpdateStatus::Available {
                latest,
                page_url,
                download_url,
            }) => match current_app_bundle() {
                Some(app_path) => {
                    let choice = ask_update_choice(&latest, current).unwrap_or_default();
                    match choice.as_str() {
                        "Install" => match install_update(&download_url, &app_path) {
                            Ok(()) => return,
                            Err(error) => update_error_script(&error),
                        },
                        "Open GitHub" => open_url_script(&page_url),
                        _ => return,
                    }
                }
                None => {
                    let message = applescript_string(&format!(
                        "Shuttle {latest} is available, but this build is not running from an app bundle."
                    ));
                    let page_url = applescript_string(&page_url);
                    format!(
                        "display dialog {message} buttons {{\"OK\", \"Open GitHub\"}} default button \"Open GitHub\" with title \"Shuttle Updates\"\nif button returned of result is \"Open GitHub\" then open location {page_url}"
                    )
                }
            },
            Ok(UpdateStatus::Current { latest }) => {
                let message = applescript_string(&format!(
                    "Shuttle is up to date. Latest version: {latest}."
                ));
                format!(
                    "display dialog {message} buttons {{\"OK\"}} default button \"OK\" with title \"Shuttle Updates\""
                )
            }
            Err(error) => update_error_script(&format!("Could not check for updates:\n{error}")),
        };
        let _ = Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(script)
            .status();
    });
}

#[cfg(all(target_os = "macos", not(test)))]
fn ask_update_choice(latest: &str, current: &str) -> Result<String, String> {
    let message = applescript_string(&format!(
        "Shuttle {latest} is available. You are running {current}. Install it now?"
    ));
    let script = format!(
        "button returned of (display dialog {message} buttons {{\"Later\", \"Open GitHub\", \"Install\"}} default button \"Install\" cancel button \"Later\" with title \"Shuttle Updates\")"
    );
    let output = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|error| format!("Could not show update prompt: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Ok("Later".to_string())
    }
}

#[cfg(all(target_os = "macos", not(test)))]
fn install_update(download_url: &str, app_path: &std::path::Path) -> Result<(), String> {
    let tmp = std::env::temp_dir().join(format!("shuttle-update-{}", std::process::id()));
    let zip = tmp.join("Shuttle.zip");
    let script = tmp.join("install.sh");
    std::fs::create_dir_all(&tmp).map_err(|error| format!("Could not prepare update: {error}"))?;

    let status = Command::new("/usr/bin/curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--output",
        ])
        .arg(&zip)
        .arg(download_url)
        .status()
        .map_err(|error| format!("Could not download update: {error}"))?;
    if !status.success() {
        return Err(format!("Update download failed with status {status}"));
    }

    let status = Command::new("/usr/bin/ditto")
        .args(["-x", "-k"])
        .arg(&zip)
        .arg(&tmp)
        .status()
        .map_err(|error| format!("Could not unpack update: {error}"))?;
    if !status.success() {
        return Err(format!("Update unpack failed with status {status}"));
    }

    let new_app = tmp.join("Shuttle.app");
    if !new_app.is_dir() {
        return Err("Update archive did not contain Shuttle.app".to_string());
    }

    let installer = format!(
        "#!/bin/sh\nset -eu\nsleep 1\nrm -rf {app}\n/usr/bin/ditto {new_app} {app}\n/usr/bin/open {app}\nrm -rf {tmp}\n",
        app = shell_quote(app_path),
        new_app = shell_quote(&new_app),
        tmp = shell_quote(&tmp),
    );
    std::fs::write(&script, installer)
        .map_err(|error| format!("Could not write installer: {error}"))?;
    Command::new("/bin/chmod")
        .args(["+x"])
        .arg(&script)
        .status()
        .map_err(|error| format!("Could not prepare installer: {error}"))?;
    Command::new("/bin/sh")
        .arg(&script)
        .spawn()
        .map_err(|error| format!("Could not start installer: {error}"))?;
    Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg("tell application \"Shuttle\" to quit")
        .spawn()
        .map_err(|error| format!("Could not quit Shuttle: {error}"))?;
    Ok(())
}

#[cfg(all(target_os = "macos", not(test)))]
fn current_app_bundle() -> Option<std::path::PathBuf> {
    let mut path = std::env::current_exe().ok()?;
    while path.pop() {
        if path.extension().is_some_and(|extension| extension == "app") {
            return Some(path);
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn update_error_script(error: &str) -> String {
    let message = applescript_string(error);
    format!(
        "display dialog {message} buttons {{\"OK\"}} default button \"OK\" with title \"Shuttle Updates\" with icon caution"
    )
}

#[cfg(target_os = "macos")]
fn open_url_script(url: &str) -> String {
    let url = applescript_string(url);
    format!("open location {url}")
}

#[cfg(target_os = "macos")]
fn applescript_string(value: &str) -> String {
    format!("{:?}", value)
}

#[cfg(target_os = "macos")]
fn shell_quote(path: &std::path::Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn compare_versions(left: &str, right: &str) -> Ordering {
    let left = version_parts(left);
    let right = version_parts(right);
    for index in 0..left.len().max(right.len()) {
        match left
            .get(index)
            .unwrap_or(&0)
            .cmp(right.get(index).unwrap_or(&0))
        {
            Ordering::Equal => continue,
            ordering => return ordering,
        }
    }
    Ordering::Equal
}

fn version_parts(version: &str) -> Vec<u64> {
    version
        .split(['.', '-'])
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_semver_like_versions() {
        assert_eq!(compare_versions("0.3.2", "0.3.1"), Ordering::Greater);
        assert_eq!(compare_versions("0.3", "0.3.0"), Ordering::Equal);
        assert_eq!(compare_versions("0.2.9", "0.3.0"), Ordering::Less);
        assert_eq!(compare_versions("0.3-beta", "0.3.0"), Ordering::Equal);
        assert_eq!(version_parts("1.2-alpha"), vec![1, 2, 0]);
    }

    #[test]
    fn parses_available_release_response() {
        let json = br#"{
            "tag_name":"v9.9.9",
            "html_url":"https://example.com/release",
            "assets":[
                {"name":"notes.txt","browser_download_url":"https://example.com/notes"},
                {"name":"Shuttle.zip","browser_download_url":"https://example.com/Shuttle.zip"}
            ]
        }"#;
        assert_eq!(
            status_from_release_json("0.1.0", json).unwrap(),
            UpdateStatus::Available {
                latest: "9.9.9".into(),
                page_url: "https://example.com/release".into(),
                download_url: "https://example.com/Shuttle.zip".into(),
            }
        );
    }

    #[test]
    fn parses_current_release_response_with_default_page() {
        let json = br#"{
            "tag_name":"v0.5.1",
            "html_url":null,
            "assets":[{"name":"Shuttle.zip","browser_download_url":"https://example.com/Shuttle.zip"}]
        }"#;
        assert_eq!(
            status_from_release_json("0.5.1", json).unwrap(),
            UpdateStatus::Current {
                latest: "0.5.1".into()
            }
        );
    }

    #[test]
    fn rejects_bad_release_responses() {
        assert!(status_from_release_json("0.1.0", b"not json").is_err());
        let json = br#"{"tag_name":"v1.0.0","html_url":null,"assets":[]}"#;
        assert_eq!(
            status_from_release_json("0.1.0", json).unwrap_err(),
            "Latest GitHub release has no Shuttle.zip asset"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn builds_update_scripts_safely() {
        assert_eq!(applescript_string("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(
            open_url_script("https://example.com"),
            "open location \"https://example.com\""
        );
        assert!(update_error_script("bad").contains("with icon caution"));
        assert_eq!(
            shell_quote(std::path::Path::new("/tmp/it's.app")),
            "'/tmp/it'\\''s.app'"
        );
    }
}
