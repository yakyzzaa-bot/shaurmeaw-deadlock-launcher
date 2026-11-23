//! Поиск Steam и установленной игры Deadlock на любом диске и в реестре Windows.

use std::path::{Path, PathBuf};

/// Steam AppID игры Deadlock.
pub const APP_ID: &str = "1422450";

/// Проверяет, похожа ли папка на установленный клиент Steam.
pub fn is_valid_steam_dir(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path.join("steam.exe").is_file()
}

/// Пытается восстановить корень Steam по пути к Deadlock (`...\Steam\steamapps\common\Deadlock`).
pub fn infer_steam_from_game(game: &Path) -> Option<PathBuf> {
    let steam_candidate = game.ancestors().nth(3)?;
    is_valid_steam_dir(steam_candidate).then(|| steam_candidate.to_path_buf())
}

/// Ищет папку Steam: через реестр Windows (HKCU/HKLM), переменные окружения и на всех дисках.
pub fn find_steam_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        use winreg::RegKey;

        // HKCU\Software\Valve\Steam -> SteamPath / SteamExe
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(key) = hkcu.open_subkey(r"Software\Valve\Steam") {
            if let Ok(p) = key.get_value::<String, _>("SteamPath") {
                let path = PathBuf::from(p.replace('/', "\\"));
                if is_valid_steam_dir(&path) {
                    return Some(path);
                }
            }
            if let Ok(exe) = key.get_value::<String, _>("SteamExe") {
                let exe_path = PathBuf::from(exe.replace('/', "\\"));
                if let Some(parent) = exe_path.parent() {
                    if is_valid_steam_dir(parent) {
                        return Some(parent.to_path_buf());
                    }
                }
            }
        }

        // HKLM -> InstallPath (32-bit и 64-bit ветки)
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        for sub in [r"SOFTWARE\WOW6432Node\Valve\Steam", r"SOFTWARE\Valve\Steam"] {
            if let Ok(key) = hklm.open_subkey(sub) {
                if let Ok(p) = key.get_value::<String, _>("InstallPath") {
                    let path = PathBuf::from(p.replace('/', "\\"));
                    if is_valid_steam_dir(&path) {
                        return Some(path);
                    }
                }
            }
        }
    }

    // Стандартные пути через переменные окружения
    for var in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
        if let Ok(base) = std::env::var(var) {
            let trimmed = base.trim();
            if !trimmed.is_empty() {
                let candidate = PathBuf::from(trimmed).join("Steam");
                if is_valid_steam_dir(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }

    // Сканирование типовых папок Steam на всех существующих дисках C:\ .. Z:\
    for drive in b'C'..=b'Z' {
        let root_str = format!("{}:\\", drive as char);
        let root = Path::new(&root_str);
        if !root.exists() {
            continue;
        }
        for rel in [
            "Steam",
            r"Program Files (x86)\Steam",
            r"Program Files\Steam",
            r"Games\Steam",
            r"SteamLibrary\Steam",
        ] {
            let candidate = root.join(rel);
            if is_valid_steam_dir(&candidate) {
                return Some(candidate);
            }
        }
    }

    None
}

fn push_unique_lib(libs: &mut Vec<PathBuf>, candidate: PathBuf) {
    let norm = candidate
        .to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_ascii_lowercase();
    if norm.is_empty() {
        return;
    }
    let duplicate = libs.iter().any(|l| {
        l.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_ascii_lowercase()
            == norm
    });
    if !duplicate {
        libs.push(candidate);
    }
}

/// Разбирает содержимое `libraryfolders.vdf` (как новый формат с `"path"`, так и старый `"1" "D:\\..."`).
pub fn parse_library_folders_vdf(text: &str, base_steam: &Path) -> Vec<PathBuf> {
    let mut libs = Vec::new();
    if !base_steam.as_os_str().is_empty() {
        push_unique_lib(&mut libs, base_steam.to_path_buf());
    }

    for line in text.lines() {
        let Some((key_raw, val_raw)) = parse_vdf_kv(line) else {
            continue;
        };
        let key = key_raw.trim();
        let val = val_raw.trim();
        if val.is_empty() {
            continue;
        }

        let is_modern_path = key.eq_ignore_ascii_case("path");
        let is_legacy_numeric_path = !key.is_empty()
            && key.bytes().all(|b| b.is_ascii_digit())
            && (val.contains(":\\") || val.contains(":/") || val.starts_with("\\\\"));

        if is_modern_path || is_legacy_numeric_path {
            let normalized = val.replace("\\\\", "\\").replace('/', "\\");
            push_unique_lib(&mut libs, PathBuf::from(normalized));
        }
    }

    libs
}

/// Возвращает все библиотеки Steam из `steamapps/libraryfolders.vdf`.
pub fn library_folders(steam: &Path) -> Vec<PathBuf> {
    let vdf = steam.join("steamapps").join("libraryfolders.vdf");
    match crate::fs_safe::read_text_lossy_with_retry(&vdf) {
        Ok(text) => parse_library_folders_vdf(&text, steam),
        Err(_) => {
            let mut libs = Vec::new();
            if !steam.as_os_str().is_empty() {
                libs.push(steam.to_path_buf());
            }
            libs
        }
    }
}

/// Ищет Deadlock: в библиотеках указанного Steam, в реестре Uninstall и на всех дисках C:\..Z:\.
pub fn find_deadlock_anywhere(steam_hint: Option<&Path>) -> Option<PathBuf> {
    if let Some(steam) = steam_hint {
        if !steam.as_os_str().is_empty() && steam.exists() {
            if let Some(found) = library_folders(steam)
                .into_iter()
                .map(|lib| lib.join("steamapps").join("common").join("Deadlock"))
                .find(|candidate| is_valid_game_dir(candidate))
            {
                return Some(found);
            }
        }
    }

    #[cfg(windows)]
    {
        use winreg::enums::HKEY_LOCAL_MACHINE;
        use winreg::RegKey;

        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        for sub in [
            format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Steam App {APP_ID}"),
            format!(r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Steam App {APP_ID}"),
        ] {
            if let Ok(key) = hklm.open_subkey(sub) {
                if let Ok(loc) = key.get_value::<String, _>("InstallLocation") {
                    let candidate = PathBuf::from(loc.replace('/', "\\"));
                    if is_valid_game_dir(&candidate) {
                        return Some(candidate);
                    }
                }
            }
        }
    }

    for drive in b'C'..=b'Z' {
        let root_str = format!("{}:\\", drive as char);
        let root = Path::new(&root_str);
        if !root.exists() {
            continue;
        }
        for rel in [
            r"SteamLibrary\steamapps\common\Deadlock",
            r"Steam\steamapps\common\Deadlock",
            r"Games\Steam\steamapps\common\Deadlock",
            r"Games\SteamLibrary\steamapps\common\Deadlock",
            r"Program Files (x86)\Steam\steamapps\common\Deadlock",
            r"Program Files\Steam\steamapps\common\Deadlock",
        ] {
            let candidate = root.join(rel);
            if is_valid_game_dir(&candidate) {
                return Some(candidate);
            }
        }
    }

    None
}

/// Похоже ли это на корневую папку Deadlock.
pub fn is_valid_game_dir(game: &Path) -> bool {
    game.join("game").join("citadel").exists()
}

/// Папка с конфигами: Deadlock\game\citadel\cfg
pub fn cfg_dir(game: &Path) -> PathBuf {
    game.join("game").join("citadel").join("cfg")
}

/// Исполняемый файл игры: `deadlock.exe` (приоритетно) или fallback `project8.exe`.
pub fn game_exe(game: &Path) -> PathBuf {
    let win64 = game.join("game").join("bin").join("win64");
    let primary = win64.join("deadlock.exe");
    if primary.exists() {
        return primary;
    }
    let fallback = win64.join("project8.exe");
    if fallback.exists() {
        return fallback;
    }
    primary
}

const STEAM_ID64_BASE: u64 = 76_561_197_960_265_728;

fn parse_vdf_line(line: &str) -> (Vec<String>, i32) {
    let mut tokens = Vec::new();
    let mut brace_delta: i32 = 0;
    let mut in_quotes = false;
    let mut escape = false;
    let mut cur = String::new();
    let mut prev_slash = false;

    for ch in line.trim_start_matches('\u{FEFF}').chars() {
        if in_quotes {
            if escape {
                match ch {
                    'n' => cur.push('\n'),
                    'r' => cur.push('\r'),
                    't' => cur.push('\t'),
                    _ => cur.push(ch),
                }
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_quotes = false;
                tokens.push(std::mem::take(&mut cur));
            } else {
                cur.push(ch);
            }
        } else {
            if ch == '/' && prev_slash {
                break;
            }
            prev_slash = ch == '/';
            match ch {
                '"' => in_quotes = true,
                '{' => brace_delta += 1,
                '}' => brace_delta -= 1,
                _ => {}
            }
        }
    }

    (tokens, brace_delta)
}

fn parse_vdf_kv(line: &str) -> Option<(String, String)> {
    let (mut tokens, _) = parse_vdf_line(line);
    if tokens.len() >= 2 {
        let v = tokens.swap_remove(1);
        let k = tokens.swap_remove(0);
        Some((k, v))
    } else {
        None
    }
}

/// Извлекает `"PersonaName"` из `userdata\<id>\config\localconfig.vdf`.
pub fn parse_localconfig_persona(text: &str) -> Option<String> {
    for line in text.lines() {
        if let Some((key, val)) = parse_vdf_kv(line) {
            if key.eq_ignore_ascii_case("PersonaName") {
                let nick = val.trim();
                if !nick.is_empty() {
                    return Some(nick.to_owned());
                }
            }
        }
    }
    None
}

#[derive(Default)]
struct LoginUserEntry {
    steam_id64: u64,
    persona_name: String,
    most_recent: bool,
    auto_login: bool,
    timestamp: u64,
}

/// Извлекает ник активного пользователя из `config\loginusers.vdf` с учётом `ActiveUser`.
pub fn parse_loginusers_persona(text: &str, active_user_id: Option<u32>) -> Option<String> {
    let mut entries: Vec<LoginUserEntry> = Vec::new();
    let mut depth: i32 = 0;
    let mut pending_id: Option<u64> = None;
    let mut current: Option<LoginUserEntry> = None;

    for line in text.lines() {
        let (tokens, delta) = parse_vdf_line(line);
        let prev_depth = depth;
        let next_depth = (depth + delta).max(0);

        if prev_depth == 1 {
            if let [first] = tokens.as_slice() {
                if let Ok(sid) = first.trim().parse::<u64>() {
                    pending_id = Some(sid);
                }
            }
        }

        if prev_depth < 2 && next_depth >= 2 {
            if let Some(sid) = pending_id.take() {
                current = Some(LoginUserEntry {
                    steam_id64: sid,
                    ..Default::default()
                });
            }
        }

        if prev_depth == 2 || next_depth == 2 {
            if let Some(ref mut entry) = current {
                if let [k_raw, v_raw, ..] = tokens.as_slice() {
                    let key = k_raw.trim();
                    let val = v_raw.trim();
                    if key.eq_ignore_ascii_case("PersonaName") {
                        entry.persona_name = val.to_owned();
                    } else if key.eq_ignore_ascii_case("MostRecent") {
                        entry.most_recent = val == "1";
                    } else if key.eq_ignore_ascii_case("AutoLogin") {
                        entry.auto_login = val == "1";
                    } else if key.eq_ignore_ascii_case("timestamp") {
                        entry.timestamp = val.parse::<u64>().unwrap_or(0);
                    }
                }
            }
        }

        if prev_depth >= 2 && next_depth < 2 {
            if let Some(entry) = current.take() {
                if !entry.persona_name.is_empty() {
                    entries.push(entry);
                }
            }
        }

        depth = next_depth;
    }

    if let Some(entry) = current.take() {
        if !entry.persona_name.is_empty() {
            entries.push(entry);
        }
    }

    if entries.is_empty() {
        return None;
    }

    if let Some(uid) = active_user_id.filter(|&u| u > 0) {
        let target_sid = STEAM_ID64_BASE + uid as u64;
        if let Some(found) = entries.iter().find(|e| e.steam_id64 == target_sid) {
            return Some(found.persona_name.clone());
        }
    }

    entries
        .into_iter()
        .max_by_key(|e| (e.most_recent, e.auto_login, e.timestamp))
        .map(|e| e.persona_name)
}

/// Читает ID активного пользователя Steam из реестра Windows (`HKCU\Software\Valve\Steam\ActiveProcess\ActiveUser`).
pub fn active_steam_user_id() -> Option<u32> {
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(key) = hkcu.open_subkey(r"Software\Valve\Steam\ActiveProcess") {
            if let Ok(uid) = key.get_value::<u32, _>("ActiveUser") {
                if uid > 0 {
                    return Some(uid);
                }
            }
        }
    }
    None
}

/// Определяет текущий отображаемый ник игрока из Steam (`localconfig.vdf` / `loginusers.vdf`).
pub fn current_persona_name(steam_dir: &Path) -> Option<String> {
    let resolved_root;
    let root = if !steam_dir.as_os_str().is_empty() && steam_dir.exists() {
        steam_dir
    } else {
        resolved_root = find_steam_path()?;
        resolved_root.as_path()
    };

    let active_uid = active_steam_user_id();
    if let Some(uid) = active_uid {
        let localconfig = root
            .join("userdata")
            .join(uid.to_string())
            .join("config")
            .join("localconfig.vdf");
        if let Ok(bytes) = std::fs::read(&localconfig) {
            let text = String::from_utf8_lossy(&bytes);
            if let Some(nick) = parse_localconfig_persona(&text) {
                return Some(nick);
            }
        }
    }

    let loginusers = root.join("config").join("loginusers.vdf");
    if let Ok(bytes) = std::fs::read(&loginusers) {
        let text = String::from_utf8_lossy(&bytes);
        if let Some(nick) = parse_loginusers_persona(&text, active_uid) {
            return Some(nick);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_parse_library_folders_modern_and_legacy_vdf() {
        let vdf = r#"
"libraryfolders"
{
    "0"
    {
        "path"		"C:\\Program Files (x86)\\Steam"
        "label"		""
        "contentid"		"123456"
    }
    "1"		"D:\\SteamLibrary"
    "2"		"e:/Games/SteamLibrary/"
    "3"		"C:\\Program Files (x86)\\Steam\\"
    "TimeNextStatsReport"		"1700000000"
}
"#;
        let base = PathBuf::from(r"C:\Program Files (x86)\Steam");
        let libs = parse_library_folders_vdf(vdf, &base);
        assert_eq!(libs.len(), 3);
        assert_eq!(libs[0], PathBuf::from(r"C:\Program Files (x86)\Steam"));
        assert_eq!(libs[1], PathBuf::from(r"D:\SteamLibrary"));
        assert_eq!(libs[2], PathBuf::from(r"e:\Games\SteamLibrary\"));
    }

    #[test]
    fn test_infer_steam_from_game() {
        let root = std::env::temp_dir().join(format!(
            "shaurmeaw_steam_infer_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let game = root.join("steamapps").join("common").join("Deadlock");
        std::fs::create_dir_all(game.join("game").join("citadel")).unwrap();
        std::fs::write(root.join("steam.exe"), b"").unwrap();

        assert_eq!(infer_steam_from_game(&game), Some(root.clone()));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_parse_loginusers_persona_active_user_and_fallbacks() {
        let vdf = r#"
"users"
{
	"76561199496644343"
	{
		"AccountName"		"platonboard"
		"PersonaName"		"ShaurMeaw"
		"AutoLogin"		"1"
		"timestamp"		"1795198000"
	}
	"76561199843311203"
	{
		"AccountName"		"other"
		"PersonaName"		"Оставь отзыв пж <3"
		"MostRecent"		"1"
		"AutoLogin"		"0"
		"timestamp"		"1795199000"
	}
	"76561198000000001"
	{
		"AccountName"		"third"
		"PersonaName"		"Добрая собачка :3"
		"timestamp"		"1794080796"
	}
}
"#;
        // 1. Приоритет активного ActiveUser из реестра (76561197960265728 + 1536378615 = 76561199496644343)
        assert_eq!(
            parse_loginusers_persona(vdf, Some(1_536_378_615)).as_deref(),
            Some("ShaurMeaw")
        );
        // 2. Fallback без ActiveUser -> MostRecent = "1"
        assert_eq!(
            parse_loginusers_persona(vdf, None).as_deref(),
            Some("Оставь отзыв пж <3")
        );

        // 3. Fallback без MostRecent -> AutoLogin = "1", иначе максимальный timestamp
        let vdf_no_recent = r#"
"users"
{
	"76561199496644343"
	{
		"PersonaName"		"ShaurMeaw"
		"AutoLogin"		"1"
		"timestamp"		"100"
	}
	"76561198000000001"
	{
		"PersonaName"		"Second"
		"AutoLogin"		"0"
		"timestamp"		"999"
	}
}
"#;
        assert_eq!(
            parse_loginusers_persona(vdf_no_recent, None).as_deref(),
            Some("ShaurMeaw")
        );

        let vdf_ts_only = r#"
"users"
{
	"76561199496644343"
	{
		"PersonaName"		"Older"
		"timestamp"		"100"
	}
	"76561198000000001"
	{
		"PersonaName"		"Newer"
		"timestamp"		"999"
	}
}
"#;
        assert_eq!(
            parse_loginusers_persona(vdf_ts_only, None).as_deref(),
            Some("Newer")
        );
    }

    #[test]
    fn test_parse_localconfig_persona() {
        let local = r#"
"UserLocalConfigStore"
{
	"friends"
	{
		"PersonaName"		"ShaurMeaw"
	}
}
"#;
        assert_eq!(parse_localconfig_persona(local).as_deref(), Some("ShaurMeaw"));
    }
}
