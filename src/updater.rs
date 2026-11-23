//! Автообновление лаунчера через GitHub Releases (`yakyzzaa-bot/shaurmeaw-deadlock-launcher`).

use std::env;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use std::process::{self, Command};
use std::thread;
use std::time::Duration;

use serde::Deserialize;

pub const GITHUB_OWNER: &str = "yakyzzaa-bot";
pub const GITHUB_REPO: &str = "shaurmeaw-deadlock-launcher";

const USER_AGENT: &str = "ShaurMeaw-Launcher";
const OLD_BACKUP_NAME: &str = "ShaurMeaw_old.exe.bak";
const UPDATE_TMP_NAME: &str = "ShaurMeaw_update.tmp";
const MIN_VALID_EXE_BYTES: u64 = 64 * 1024;

#[derive(Deserialize)]
struct GitHubRelease {
    #[serde(default)]
    tag_name: String,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    #[serde(default)]
    name: String,
    #[serde(default)]
    browser_download_url: String,
}

/// Удаляет временный бэкап `ShaurMeaw_old.exe.bak` рядом с `.exe`, оставшийся после прошлого обновления.
pub fn cleanup_old_binary() {
    let Ok(cur_exe) = env::current_exe() else {
        return;
    };
    let Some(exe_dir) = cur_exe.parent() else {
        return;
    };
    let bak_path = exe_dir.join(OLD_BACKUP_NAME);
    if !bak_path.exists() {
        return;
    }

    for attempt in 0..10u8 {
        if fs::remove_file(&bak_path).is_ok() || !bak_path.exists() {
            return;
        }
        if attempt < 9 {
            thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Проверяет последний релиз на GitHub. Возвращает `Some((новая_версия, url_на_exe))`,
/// если версия на GitHub строго выше текущей `CARGO_PKG_VERSION`. При любой ошибке возвращает `None`.
pub fn check_update() -> Option<(String, String)> {
    let url = format!("https://api.github.com/repos/{GITHUB_OWNER}/{GITHUB_REPO}/releases/latest");
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(6))
        .timeout(Duration::from_secs(10))
        .build();

    let resp = agent
        .get(&url)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "application/vnd.github+json")
        .call()
        .ok()?;
    let release = resp.into_json::<GitHubRelease>().ok()?;

    let current_ver = semver::Version::parse(env!("CARGO_PKG_VERSION")).ok()?;
    let remote_ver = parse_remote_version(&release.tag_name)?;
    if remote_ver <= current_ver {
        return None;
    }

    let exe_url = pick_exe_asset(&release.assets)?;
    Some((remote_ver.to_string(), exe_url))
}

/// Скачивает новый `.exe`, подменяет текущий исполняемый файл через временный бэкап,
/// перезапускает обновлённый лаунчер и завершает текущий процесс.
pub fn apply_update(download_url: &str) -> Result<(), String> {
    if !download_url.starts_with("https://") {
        return Err("Небезопасная или некорректная ссылка на обновление".to_owned());
    }

    let current_exe = env::current_exe()
        .map_err(|e| format!("Не удалось определить путь к текущему .exe: {e}"))?;
    let exe_dir = current_exe
        .parent()
        .ok_or_else(|| "Не удалось определить папку текущего .exe".to_owned())?;
    let tmp_path = exe_dir.join(UPDATE_TMP_NAME);
    let bak_path = exe_dir.join(OLD_BACKUP_NAME);

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .build();
    let resp = agent
        .get(download_url)
        .set("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| format!("Не удалось скачать обновление: {e}"))?;

    let copy_res: io::Result<u64> = (|| {
        let mut reader = resp.into_reader();
        let mut file = File::create(&tmp_path)?;
        let written = io::copy(&mut reader, &mut file)?;
        file.sync_all()?;
        Ok(written)
    })();
    if let Err(e) = copy_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(format!("Ошибка сохранения файла обновления: {e}"));
    }

    if !is_valid_pe_exe(&tmp_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err("Скачанный файл повреждён или не является исполняемым .exe".to_owned());
    }

    let _ = fs::remove_file(&bak_path);
    if let Err(e) = fs::rename(&current_exe, &bak_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(format!("Не удалось освободить текущий .exe: {e}"));
    }
    if let Err(e) = fs::rename(&tmp_path, &current_exe) {
        let _ = fs::rename(&bak_path, &current_exe);
        let _ = fs::remove_file(&tmp_path);
        return Err(format!("Не удалось установить новый .exe: {e}"));
    }

    if let Err(e) = Command::new(&current_exe).arg("--post-update").spawn() {
        return Err(format!(
            "Обновление установлено, но не удалось перезапустить лаунчер: {e}"
        ));
    }
    process::exit(0);
}

fn parse_remote_version(tag: &str) -> Option<semver::Version> {
    let trimmed = tag.trim().trim_start_matches(['v', 'V']);
    semver::Version::parse(trimmed).ok()
}

fn pick_exe_asset(assets: &[GitHubAsset]) -> Option<String> {
    let mut fallback: Option<&str> = None;
    for asset in assets {
        if !asset.browser_download_url.starts_with("https://") {
            continue;
        }
        let lower = asset.name.to_ascii_lowercase();
        if lower.ends_with(".exe") {
            if lower.contains("shaurmeaw") {
                return Some(asset.browser_download_url.clone());
            }
            if fallback.is_none() {
                fallback = Some(asset.browser_download_url.as_str());
            }
        }
    }
    fallback.map(str::to_owned)
}

fn is_valid_pe_exe(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if meta.len() < MIN_VALID_EXE_BYTES {
        return false;
    }
    let Ok(mut f) = File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 2];
    f.read_exact(&mut magic).is_ok() && magic == *b"MZ"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_remote_version_strips_v_prefix() {
        assert_eq!(
            parse_remote_version("v1.2.3"),
            Some(semver::Version::new(1, 2, 3))
        );
        assert_eq!(
            parse_remote_version(" V2.0.1 "),
            Some(semver::Version::new(2, 0, 1))
        );
        assert!(parse_remote_version("invalid").is_none());
    }

    #[test]
    fn test_pick_exe_asset_prefers_shaurmeaw_https_exe() {
        let assets = vec![
            GitHubAsset {
                name: "SHA256SUMS.txt".to_owned(),
                browser_download_url: "https://example.com/SHA256SUMS.txt".to_owned(),
            },
            GitHubAsset {
                name: "other.exe".to_owned(),
                browser_download_url: "https://example.com/other.exe".to_owned(),
            },
            GitHubAsset {
                name: "ShaurMeaw.exe".to_owned(),
                browser_download_url: "https://example.com/ShaurMeaw.exe".to_owned(),
            },
            GitHubAsset {
                name: "ShaurMeaw_insecure.exe".to_owned(),
                browser_download_url: "http://example.com/ShaurMeaw.exe".to_owned(),
            },
        ];
        assert_eq!(
            pick_exe_asset(&assets).as_deref(),
            Some("https://example.com/ShaurMeaw.exe")
        );
    }
}
