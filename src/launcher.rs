//! Запись autoexec.cfg на диск и запуск игры.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::autoexec_presets::{self, MODE_BUILDER, PRESET_SHAURMEAW, PRESET_VALVE};
use crate::config_gen::{build_autoexec, build_launch_args, MARKER, OLD_MARKER};
use crate::fs_safe::{atomic_write_with_retry, read_bytes_with_retry};
use crate::settings::{LaunchMode, Settings};
use crate::steam;

/// Возвращает итоговое содержимое `autoexec.cfg` согласно выбранному пресету.
pub fn resolve_autoexec_content(s: &Settings) -> String {
    resolve_autoexec_content_in(s, &Settings::autoexec_presets_dir())
}

pub fn resolve_autoexec_content_in(s: &Settings, presets_dir: &Path) -> String {
    let preset = s.autoexec_preset.trim();
    if preset == MODE_BUILDER {
        return build_autoexec(s);
    }
    if preset == PRESET_VALVE {
        return format!(
            "{MARKER}\n// Официальный режим Valve — пользовательские консольные команды отключены\n"
        );
    }

    let raw = autoexec_presets::read_preset_in(presets_dir, preset).unwrap_or_else(|_| {
        if preset.is_empty() || preset == PRESET_SHAURMEAW {
            crate::embedded::shaurmeaw_autoexec().to_owned()
        } else {
            build_autoexec(s)
        }
    });

    let trimmed = raw.trim_start_matches('\u{feff}');
    if trimmed.starts_with(MARKER) || trimmed.starts_with(OLD_MARKER) {
        trimmed.to_owned()
    } else {
        format!("{MARKER}\n{trimmed}")
    }
}

/// Записывает autoexec.cfg в папку игры. Если там лежал «чужой» autoexec —
/// сначала делает резервную копию autoexec.cfg.bak (снимая атрибут Read-Only).
pub fn write_autoexec(s: &Settings) -> Result<PathBuf, String> {
    write_autoexec_with_content(s, &resolve_autoexec_content(s))
}

pub fn write_autoexec_with_content(s: &Settings, content: &str) -> Result<PathBuf, String> {
    let game = PathBuf::from(&s.game_path);
    if !steam::is_valid_game_dir(&game) {
        return Err("Папка Deadlock не найдена — укажите её во вкладке «Пути»".into());
    }

    let dir = steam::cfg_dir(&game);
    fs::create_dir_all(&dir).map_err(|e| format!("Не удалось создать {}: {e}", dir.display()))?;

    let file = dir.join("autoexec.cfg");
    if file.exists() {
        let old_bytes = read_bytes_with_retry(&file)
            .map_err(|e| format!("Не удалось прочитать {}: {e}", file.display()))?;
        let old_text = String::from_utf8_lossy(&old_bytes);
        let old_clean = old_text.trim_start_matches('\u{feff}');

        if !old_clean.starts_with(MARKER) && !old_clean.starts_with(OLD_MARKER) {
            let backup = dir.join("autoexec.cfg.bak");
            if let Err(e) = atomic_write_with_retry(&backup, &old_bytes) {
                if e.kind() == ErrorKind::PermissionDenied {
                    let fallback_dir = Settings::gameinfo_backups_dir();
                    let fallback_bak = fallback_dir.join("autoexec.cfg.bak");
                    atomic_write_with_retry(&fallback_bak, &old_bytes).map_err(|e2| {
                        format!(
                            "Не удалось сделать бэкап autoexec.cfg (нет прав на запись в папку игры и резервную папку): {e2}. Запустите ShaurMeaw.exe от имени администратора."
                        )
                    })?;
                } else {
                    return Err(format!("Не удалось сделать бэкап autoexec.cfg: {e}"));
                }
            }
        }
    }

    atomic_write_with_retry(&file, content.as_bytes()).map_err(|e| {
        if e.kind() == ErrorKind::PermissionDenied {
            format!(
                "Нет прав на запись {}: {e}. Снимите «Только чтение», проверьте антивирус или запустите ShaurMeaw.exe от имени администратора.",
                file.display()
            )
        } else {
            format!("Не удалось записать {}: {e}", file.display())
        }
    })?;
    Ok(file)
}

/// Формирует URI `steam://run/<AppID>//<args>` на случай, если `steam.exe` не найден по пути.
pub fn build_steam_run_uri(args: &[String]) -> String {
    if args.is_empty() {
        return format!("steam://run/{}", steam::APP_ID);
    }
    let encoded: Vec<String> = args
        .iter()
        .map(|arg| {
            arg.replace('%', "%25")
                .replace(' ', "%20")
                .replace('"', "%22")
        })
        .collect();
    format!("steam://run/{}//{}", steam::APP_ID, encoded.join("%20"))
}

/// Запускает игру выбранным способом.
pub fn launch(s: &Settings) -> Result<String, String> {
    launch_with_extra(s, &[])
}

/// Запускает игру с дополнительными аргументами (например, для полигона Sandbox).
pub fn launch_with_extra(s: &Settings, extra_args: &[&str]) -> Result<String, String> {
    let mut args = build_launch_args(s);
    for extra in extra_args {
        if !extra.is_empty() {
            args.push((*extra).to_owned());
        }
    }

    match s.launch_mode {
        LaunchMode::Steam => {
            let configured = PathBuf::from(&s.steam_path).join("steam.exe");
            let exe_opt = if !s.steam_path.trim().is_empty() && configured.is_file() {
                Some(configured)
            } else {
                steam::find_steam_path().map(|p| p.join("steam.exe"))
            };

            if let Some(exe) = exe_opt.filter(|p| p.is_file()) {
                Command::new(&exe)
                    .arg("-applaunch")
                    .arg(steam::APP_ID)
                    .args(&args)
                    .spawn()
                    .map_err(|e| format!("Не удалось запустить Steam: {e}"))?;
                Ok("Запуск через Steam… Игра откроется через несколько секунд".into())
            } else {
                let uri = build_steam_run_uri(&args);
                Command::new(crate::tools::windir_exe("explorer.exe"))
                    .arg(&uri)
                    .spawn()
                    .map_err(|e| {
                        format!("steam.exe не найден, а запуск через steam:// не удался: {e}")
                    })?;
                Ok("steam.exe не указан — запуск через протокол steam://…".into())
            }
        }
        LaunchMode::Direct => {
            let exe = steam::game_exe(Path::new(&s.game_path));
            if !exe.exists() {
                return Err(format!("deadlock.exe не найден: {}", exe.display()));
            }
            let mut cmd = Command::new(&exe);
            cmd.args(&args);
            if let Some(dir) = exe.parent() {
                cmd.current_dir(dir);
            }
            cmd.spawn()
                .map_err(|e| format!("Не удалось запустить игру: {e}"))?;
            Ok("Игра запускается напрямую…".into())
        }
    }
}

/// Открывает папку в проводнике Windows.
pub fn open_folder(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("Папка не существует: {}", path.display()));
    }
    Command::new(crate::tools::windir_exe("explorer.exe"))
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_write_autoexec_handles_readonly_and_backup() {
        let root = std::env::temp_dir().join(format!(
            "shaurmeaw_autoexec_ro_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let ae_dir = root.join("ae_presets");
        let citadel_dir = root.join("game").join("citadel");
        let cfg_dir = citadel_dir.join("cfg");
        fs::create_dir_all(&cfg_dir).unwrap();
        autoexec_presets::ensure_default_presets_in(&ae_dir).unwrap();

        let autoexec_path = cfg_dir.join("autoexec.cfg");
        let backup_path = cfg_dir.join("autoexec.cfg.bak");
        fs::write(&autoexec_path, "sensitivity 2.5\n").unwrap();
        fs::write(&backup_path, "old_bak\n").unwrap();

        for p in [&autoexec_path, &backup_path] {
            let mut perms = fs::metadata(p).unwrap().permissions();
            perms.set_readonly(true);
            fs::set_permissions(p, perms).unwrap();
        }

        let mut s = Settings::default();
        s.game_path = root.display().to_string();

        // 1. Дефолтный пресет ShaurMeaw (Мой autoexec)
        let content_shaur = resolve_autoexec_content_in(&s, &ae_dir);
        assert!(content_shaur.contains("r_texture_stream_mip_bias 8"));
        assert!(content_shaur.contains("host_writeconfig"));
        write_autoexec_with_content(&s, &content_shaur).unwrap();
        assert_eq!(fs::read_to_string(&backup_path).unwrap(), "sensitivity 2.5\n");
        assert!(!fs::metadata(&backup_path).unwrap().permissions().readonly());

        // Ставим readonly снова и проверяем режим Valve
        let mut perms = fs::metadata(&autoexec_path).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&autoexec_path, perms).unwrap();

        s.autoexec_preset = PRESET_VALVE.to_owned();
        let content_valve = resolve_autoexec_content_in(&s, &ae_dir);
        assert!(content_valve.contains("Официальный режим Valve"));
        assert!(!content_valve.contains("r_texture_stream_mip_bias"));
        write_autoexec_with_content(&s, &content_valve).unwrap();
        assert_eq!(fs::read_to_string(&backup_path).unwrap(), "sensitivity 2.5\n");

        // Проверяем режим Конструктора
        s.autoexec_preset = MODE_BUILDER.to_owned();
        let content_builder = resolve_autoexec_content_in(&s, &ae_dir);
        assert!(content_builder.contains("fps_max 240"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_build_steam_run_uri() {
        assert_eq!(build_steam_run_uri(&[]), "steam://run/1422450");
        let args = vec![
            "-novid".to_owned(),
            "+exec".to_owned(),
            "my file".to_owned(),
        ];
        assert_eq!(
            build_steam_run_uri(&args),
            "steam://run/1422450//-novid%20+exec%20my%20file"
        );
    }
}
