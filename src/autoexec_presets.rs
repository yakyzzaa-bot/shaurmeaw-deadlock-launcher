//! Каталог и менеджер пресетов `autoexec.cfg` (`%APPDATA%\ShaurMeaw\autoexec_presets\`).

use std::fs;
use std::path::{Path, PathBuf};

use crate::gameinfo_presets::{format_system_time_human, sanitize_name};
use crate::settings::Settings;
use crate::steam;

pub const PRESET_SHAURMEAW: &str = "ShaurMeaw (Мой autoexec)";
pub const PRESET_VALVE: &str = "Официальный от Valve (Чистый)";
pub const MODE_BUILDER: &str = "Конструктор лаунчера";

pub const VALVE_AUTOEXEC: &str =
    "// Официальный конфиг Valve (без кастомных команд autoexec)\n";

#[derive(Clone, Debug)]
pub struct AutoexecPresetEntry {
    pub name: String,
    pub path: PathBuf,
    pub size_bytes: u64,
    pub modified_label: String,
}

pub fn ensure_default_presets() {
    let _ = ensure_default_presets_in(&Settings::autoexec_presets_dir());
}

pub fn read_preset(name: &str) -> Result<String, String> {
    read_preset_in(&Settings::autoexec_presets_dir(), name)
}

pub fn save_preset(name: &str, content: &str) -> Result<String, String> {
    save_preset_in(&Settings::autoexec_presets_dir(), name, content)
}

pub fn create_from_current(game_path: &Path, name: &str) -> Result<String, String> {
    create_from_current_in(&Settings::autoexec_presets_dir(), game_path, name)
}

pub fn import_from_file(src: &Path) -> Result<String, String> {
    import_from_file_in(&Settings::autoexec_presets_dir(), src)
}

pub fn delete_preset(name: &str) -> Result<(), String> {
    delete_preset_in(&Settings::autoexec_presets_dir(), name)
}

// ============================================================ реализация с явной директорией

pub fn ensure_default_presets_in(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir)
        .map_err(|e| format!("Не удалось создать папку autoexec_presets: {e}"))?;

    let shaurmeaw_path = dir.join(format!("{PRESET_SHAURMEAW}.cfg"));
    if !shaurmeaw_path.exists() {
        save_preset_in(dir, PRESET_SHAURMEAW, crate::embedded::shaurmeaw_autoexec())?;
    }

    let valve_path = dir.join(format!("{PRESET_VALVE}.cfg"));
    if !valve_path.exists() {
        save_preset_in(dir, PRESET_VALVE, VALVE_AUTOEXEC)?;
    }

    Ok(())
}

pub fn list_presets_in(dir: &Path) -> Vec<AutoexecPresetEntry> {
    let mut list = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("cfg"))
            {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    let meta_fs = entry.metadata().ok();
                    let size_bytes = meta_fs.as_ref().map(|m| m.len()).unwrap_or(0);
                    let modified_label = meta_fs
                        .and_then(|m| m.modified().ok())
                        .map(format_system_time_human)
                        .unwrap_or_else(|| "—".to_owned());
                    list.push(AutoexecPresetEntry {
                        name: stem.to_owned(),
                        path: p,
                        size_bytes,
                        modified_label,
                    });
                }
            }
        }
    }

    list.sort_by(|a, b| {
        rank(&a.name)
            .cmp(&rank(&b.name))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    list
}

fn rank(name: &str) -> u8 {
    if name == PRESET_SHAURMEAW {
        0
    } else if name == PRESET_VALVE {
        1
    } else {
        2
    }
}

pub fn read_preset_in(dir: &Path, name: &str) -> Result<String, String> {
    let clean = sanitize_name(name)?;
    let path = dir.join(format!("{clean}.cfg"));
    crate::fs_safe::read_text_lossy_with_retry(&path)
        .map_err(|e| format!("Не удалось прочитать autoexec-пресет «{clean}»: {e}"))
}

pub fn save_preset_in(dir: &Path, name: &str, content: &str) -> Result<String, String> {
    let clean = sanitize_name(name)?;
    let path = dir.join(format!("{clean}.cfg"));
    crate::fs_safe::atomic_write_with_retry(&path, content.as_bytes())
        .map_err(|e| format!("Не удалось сохранить autoexec-пресет «{clean}»: {e}"))?;
    Ok(clean)
}

pub fn create_from_current_in(dir: &Path, game_path: &Path, name: &str) -> Result<String, String> {
    let file = steam::cfg_dir(game_path).join("autoexec.cfg");
    if !file.exists() {
        return Err("В папке game\\citadel\\cfg нет файла autoexec.cfg".into());
    }
    let content = crate::fs_safe::read_text_lossy_with_retry(&file)
        .map_err(|e| format!("Не удалось прочитать текущий autoexec.cfg: {e}"))?;
    save_preset_in(dir, name, &content)
}

pub fn import_from_file_in(dir: &Path, src: &Path) -> Result<String, String> {
    let content = crate::fs_safe::read_text_lossy_with_retry(src)
        .map_err(|e| format!("Не удалось прочитать {}: {e}", src.display()))?;
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Импортированный autoexec");
    save_preset_in(dir, stem, &content)
}

pub fn delete_preset_in(dir: &Path, name: &str) -> Result<(), String> {
    let clean = sanitize_name(name)?;
    if clean == PRESET_SHAURMEAW || clean == PRESET_VALVE {
        return Err("Встроенный пресет нельзя удалить".into());
    }
    let path = dir.join(format!("{clean}.cfg"));
    if !path.exists() {
        return Err(format!("Пресет «{clean}» не найден"));
    }
    fs::remove_file(&path).map_err(|e| format!("Не удалось удалить пресет: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_autoexec_presets_defaults_and_crud() {
        let root = std::env::temp_dir().join(format!(
            "shaurmeaw_ae_presets_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let ae_dir = root.join("autoexec_presets");
        let game_dir = root.join("Deadlock");
        let cfg_dir = game_dir.join("game").join("citadel").join("cfg");
        fs::create_dir_all(&cfg_dir).unwrap();
        fs::write(cfg_dir.join("autoexec.cfg"), "fps_max 144\n").unwrap();

        ensure_default_presets_in(&ae_dir).unwrap();
        let list = list_presets_in(&ae_dir);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, PRESET_SHAURMEAW);
        assert_eq!(list[1].name, PRESET_VALVE);

        let shaur = read_preset_in(&ae_dir, PRESET_SHAURMEAW).unwrap();
        assert!(shaur.contains("r_texture_stream_mip_bias 8"));
        assert!(shaur.contains("host_writeconfig"));

        let created = create_from_current_in(&ae_dir, &game_dir, "Тестовый cfg").unwrap();
        assert_eq!(created, "Тестовый cfg");
        assert!(read_preset_in(&ae_dir, "Тестовый cfg")
            .unwrap()
            .contains("fps_max 144"));

        // Встроенные защищены от удаления, пользовательские удаляются
        assert!(delete_preset_in(&ae_dir, PRESET_VALVE).is_err());
        delete_preset_in(&ae_dir, "Тестовый cfg").unwrap();

        let _ = fs::remove_dir_all(&root);
    }
}
