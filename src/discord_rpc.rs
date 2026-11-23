//! Нативный Discord Rich Presence для Deadlock (Windows Named Pipe IPC + парсер `console.log`).
//!
//! Поддерживает актуальные режимы осени 2026 (*Hideout*, *Sandbox / Explore NYC*, *Pregame Vote*,
//! *Ranked Beta Season 2*, *The Cursed Apple*, *Street Brawl*) и всех 44 героев, включая *City Never Sleeps*.

use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui;

use crate::game_booster::{wait_event_or_dirs, WinEvent, WAIT_INFINITE};
use crate::settings::{DiscordRpcConfig, Settings};

pub const HERO_COUNT: usize = 44;

/// Официальный Discord Application ID игры Deadlock (работает без ручной регистрации в Developer Portal).
pub const DEFAULT_DISCORD_APP_ID: &str = "1276737795012165766";

const HERO_ICON_BASE_URL: &str =
    "https://assets-bucket.deadlock-api.com/assets-api-res/images/heroes";

/// Полный актуальный ростер из 44 героев Deadlock (конец 2026 года, включая *City Never Sleeps*).
#[cfg(test)]
const ALL_HEROES: [&str; HERO_COUNT] = [
    // Базовые и 2025 (32)
    "Abrams",
    "Bebop",
    "Billy",
    "Calico",
    "The Doorman",
    "Drifter",
    "Dynamo",
    "Grey Talon",
    "Haze",
    "Holliday",
    "Infernus",
    "Ivy",
    "Kelvin",
    "Lady Geist",
    "Lash",
    "McGinnis",
    "Mina",
    "Mirage",
    "Mo & Krill",
    "Paige",
    "Paradox",
    "Pocket",
    "Seven",
    "Shiv",
    "Sinclair",
    "Victor",
    "Vindicta",
    "Viscous",
    "Vyper",
    "Warden",
    "Wraith",
    "Yamato",
    // Начало 2026 (6)
    "Rem",
    "Graves",
    "Silver",
    "Venator",
    "Celeste",
    "Apollo",
    // Осень 2026 — City Never Sleeps (6)
    "Rat King",
    "Baba",
    "Solomon",
    "Deadman Danny",
    "Violet",
    "Nurse Harrow",
];

/// Сопоставляет технический идентификатор героя из `console.log` с отображаемым именем (44 героя).
pub fn resolve_hero_name(raw: &str) -> Option<&'static str> {
    let mut id = raw
        .trim()
        .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .to_ascii_lowercase();

    for prefix in ["citadel_hero_", "npc_hero_", "hero_"] {
        if let Some(rest) = id.strip_prefix(prefix) {
            id = rest.to_owned();
            break;
        }
    }

    match id.as_str() {
        // Базовые и 2025 (32)
        "abrams" | "atlas" | "bull" => Some("Abrams"),
        "bebop" => Some("Bebop"),
        "billy" | "punkgoat" => Some("Billy"),
        "calico" | "nano" => Some("Calico"),
        "doorman" | "the_doorman" => Some("The Doorman"),
        "drifter" => Some("Drifter"),
        "dynamo" | "sumo" => Some("Dynamo"),
        "grey_talon" | "greytalon" | "orion" | "archer" => Some("Grey Talon"),
        "haze" => Some("Haze"),
        "holliday" | "astro" => Some("Holliday"),
        "inferno" | "infernus" => Some("Infernus"),
        "ivy" | "tengu" => Some("Ivy"),
        "kelvin" => Some("Kelvin"),
        "lady_geist" | "geist" | "ghost" | "spectre" => Some("Lady Geist"),
        "lash" => Some("Lash"),
        "mcginnis" | "forge" | "engineer" => Some("McGinnis"),
        "mina" | "vampirebat" => Some("Mina"),
        "mirage" => Some("Mirage"),
        "mo_and_krill" | "krill" | "digger" => Some("Mo & Krill"),
        "paige" | "bookworm" => Some("Paige"),
        "paradox" | "chrono" => Some("Paradox"),
        "pocket" | "synth" => Some("Pocket"),
        "seven" | "gigawatt" => Some("Seven"),
        "shiv" => Some("Shiv"),
        "sinclair" | "magician" => Some("Sinclair"),
        "victor" | "frank" => Some("Victor"),
        "vindicta" | "hornet" => Some("Vindicta"),
        "viscous" => Some("Viscous"),
        "vyper" | "viper" | "kali" => Some("Vyper"),
        "warden" => Some("Warden"),
        "wraith" => Some("Wraith"),
        "yamato" => Some("Yamato"),
        // Начало 2026 (6)
        "rem" | "familiar" => Some("Rem"),
        "graves" | "necro" => Some("Graves"),
        "silver" | "werewolf" => Some("Silver"),
        "venator" | "priest" => Some("Venator"),
        "celeste" | "unicorn" => Some("Celeste"),
        "apollo" | "fencer" => Some("Apollo"),
        // Осень 2026 — City Never Sleeps (6)
        "rat_king" | "ratking" => Some("Rat King"),
        "baba" => Some("Baba"),
        "solomon" | "chessmaster" => Some("Solomon"),
        "deadman_danny" | "danny" | "deadpack" => Some("Deadman Danny"),
        "violet" | "artist" => Some("Violet"),
        "nurse_harrow" | "harrow" | "nurse" => Some("Nurse Harrow"),
        _ => None,
    }
}

/// Возвращает URL квадратной PNG-иконки (128×128) для каждого из 44 героев Deadlock.
pub fn hero_icon_url(hero_name: &str) -> Option<String> {
    let slug = match hero_name {
        "Abrams" => "bull",
        "Bebop" => "bebop",
        "Billy" => "punkgoat",
        "Calico" => "nano",
        "The Doorman" => "doorman",
        "Drifter" => "drifter",
        "Dynamo" => "sumo",
        "Grey Talon" => "archer",
        "Haze" => "haze",
        "Holliday" => "astro",
        "Infernus" => "inferno",
        "Ivy" => "tengu",
        "Kelvin" => "kelvin",
        "Lady Geist" => "spectre",
        "Lash" => "lash",
        "McGinnis" => "engineer",
        "Mina" => "vampirebat",
        "Mirage" => "mirage",
        "Mo & Krill" => "digger",
        "Paige" => "bookworm",
        "Paradox" => "chrono",
        "Pocket" => "synth",
        "Seven" => "gigawatt",
        "Shiv" => "shiv",
        "Sinclair" => "magician",
        "Victor" => "frank",
        "Vindicta" => "hornet",
        "Viscous" => "viscous",
        "Vyper" => "kali",
        "Warden" => "warden",
        "Wraith" => "wraith",
        "Yamato" => "yamato",
        "Rem" => "familiar",
        "Graves" => "necro",
        "Silver" => "werewolf",
        "Venator" => "priest",
        "Celeste" => "unicorn",
        "Apollo" => "fencer",
        "Rat King" => "ratking",
        "Baba" => "baba",
        "Solomon" => "chessmaster",
        "Deadman Danny" => "deadpack",
        "Violet" => "artist",
        "Nurse Harrow" => "nurse",
        _ => return None,
    };
    Some(format!("{HERO_ICON_BASE_URL}/{slug}_sm.png"))
}

/// Проверяет, является ли строка локальным путём к файлу (Discord IPC их не поддерживает).
pub fn is_local_filesystem_path(raw: &str) -> bool {
    let s = raw.trim();
    s.starts_with("file://")
        || s.starts_with(r"\\")
        || matches!(s.as_bytes(), [drive, b':', ..] if drive.is_ascii_alphabetic())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GameMode {
    #[default]
    MainMenu,
    Hideout,
    Sandbox,
    PregameLobby,
    RankedMatch,
    StandardMatch,
    StreetBrawl,
}

impl GameMode {
    pub fn label(self) -> &'static str {
        match self {
            GameMode::MainMenu => "Главное меню",
            GameMode::Hideout => "Hideout",
            GameMode::Sandbox => "Sandbox / Explore NYC",
            GameMode::PregameLobby => "Выбор героев",
            GameMode::RankedMatch => "Ranked • The Cursed Apple",
            GameMode::StandardMatch => "The Cursed Apple",
            GameMode::StreetBrawl => "Street Brawl",
        }
    }

    pub fn has_timer(self) -> bool {
        matches!(
            self,
            GameMode::Sandbox
                | GameMode::RankedMatch
                | GameMode::StandardMatch
                | GameMode::StreetBrawl
        )
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ConsoleState {
    pub mode: GameMode,
    pub hero: Option<&'static str>,
    pub player_name: Option<String>,
    pub started_at: Option<u64>,
    pub ranked_hint: bool,
    pub current_map: String,
    pub bot_entity_ids: Vec<u32>,
}

impl ConsoleState {
    fn set_mode(&mut self, mode: GameMode, now_unix: u64) {
        if self.mode != mode {
            self.mode = mode;
            self.started_at = if mode.has_timer() {
                Some(now_unix)
            } else {
                None
            };
        } else if mode.has_timer() && self.started_at.is_none() {
            self.started_at = Some(now_unix);
        }
    }
}

fn mode_for_map(map_name: &str, ranked_hint: bool) -> Option<GameMode> {
    if map_name == "<empty>" || map_name.contains("citadel_main") {
        Some(GameMode::MainMenu)
    } else if map_name.contains("dl_hideout") || map_name == "hideout" {
        Some(GameMode::Hideout)
    } else if map_name.contains("street_test")
        || map_name.contains("hero_testing")
        || map_name.contains("sandbox")
        || map_name.contains("new_player_basics")
    {
        Some(GameMode::Sandbox)
    } else if map_name.contains("street_brawl") {
        Some(GameMode::StreetBrawl)
    } else if map_name.contains("dl_midtown") || map_name.starts_with("dl_") {
        Some(if ranked_hint {
            GameMode::RankedMatch
        } else {
            GameMode::StandardMatch
        })
    } else {
        None
    }
}

const NOISE_PATTERNS: [&str; 17] = [
    "overrides identical",
    "failed loading resource",
    "attempted to attach",
    "has no model name",
    "was not precached",
    "invalid value for property",
    "clocalize",
    "created bot",
    "hero_testhero",
    "hero_reveal_card",
    "hero_stats",
    "hero_abilities",
    "hero_settings",
    "hero_builds",
    "hero_probe",
    "ambient_fx",
    "respawned",
];

const HERO_PICK_MARKERS: [&str; 10] = [
    "selecthero",
    "selected hero",
    "local player hero",
    "hero_name=",
    "switch_hero",
    "change_hero",
    "loaded local hero",
    "[heroselect]",
    "citadel_hero_select",
    "picked hero",
];

const RANKED_MARKERS: [&str; 7] = [
    "k_ecitadelgamemode_ranked",
    "ranked_match",
    "match_mode=ranked",
    "match_mode: ranked",
    "citadel_ranked_match",
    "ranked beta",
    "season 2",
];

fn extract_map_name(line: &str) -> Option<String> {
    let patterns = [
        "Loading map \"",
        "[Client] Map: \"",
        "Spawn Server: ",
        "Loading (",
    ];
    for pat in patterns {
        if let Some(pos) = line.find(pat) {
            let rest = &line[pos + pat.len()..];
            let raw = if pat.ends_with('"') {
                rest.split('"').next().unwrap_or("")
            } else if pat.ends_with('(') {
                rest.split(')').next().unwrap_or("")
            } else {
                rest.split_whitespace().next().unwrap_or("")
            };
            let cleaned = raw.trim().to_ascii_lowercase();
            if !cleaned.is_empty() {
                return Some(cleaned);
            }
        }
    }
    None
}

fn extract_player_hero(lower_line: &str) -> Option<&'static str> {
    let matched_marker = HERO_PICK_MARKERS
        .iter()
        .find(|m| lower_line.contains(**m))?;

    if let Some(pos) = lower_line.find(matched_marker) {
        let after = &lower_line[pos + matched_marker.len()..];
        for tok in after.split(|c: char| {
            c.is_whitespace() || matches!(c, '"' | '\'' | '=' | ':' | '/' | '(' | ')' | ',' | ';')
        }) {
            if tok.is_empty() {
                continue;
            }
            if let Some(hero) = resolve_hero_name(tok) {
                return Some(hero);
            }
        }
    }

    for tok in lower_line.split(|c: char| {
        c.is_whitespace() || matches!(c, '"' | '\'' | '=' | ':' | '/' | '(' | ')' | ',' | ';')
    }) {
        if tok.starts_with("hero_")
            || tok.starts_with("citadel_hero_")
            || tok.starts_with("npc_hero_")
        {
            if let Some(hero) = resolve_hero_name(tok) {
                return Some(hero);
            }
        }
    }
    None
}

fn extract_server_loaded_hero(lower_line: &str, bot_ids: &[u32]) -> Option<&'static str> {
    let pos = lower_line.find("loaded hero ")?;
    let rest = lower_line[pos + "loaded hero ".len()..].trim_start();
    let token = rest.split_whitespace().next()?;
    if let Some((id_str, hero_part)) = token.split_once('/') {
        if let Ok(entity_id) = id_str.parse::<u32>() {
            if bot_ids.contains(&entity_id) {
                return None;
            }
        }
        let hero_tok = hero_part.split('/').next().unwrap_or(hero_part);
        return resolve_hero_name(hero_tok);
    }
    resolve_hero_name(token)
}

fn extract_camera_pose_hero(lower_line: &str) -> Option<&'static str> {
    if !lower_line.contains("vmdl camera pose success!") {
        return None;
    }
    for marker in [
        "models/heroes_wip/",
        "models/heroes_staging/",
        "models/heroes/",
    ] {
        if let Some(pos) = lower_line.find(marker) {
            let rest = &lower_line[pos + marker.len()..];
            let seg = rest.split(['/', '"', '\\']).next().unwrap_or("").trim();
            if seg.is_empty() {
                continue;
            }
            if let Some(hero) = resolve_hero_name(seg) {
                return Some(hero);
            }
            if let Some(prefix) = seg.split('_').next() {
                if let Some(hero) = resolve_hero_name(prefix) {
                    return Some(hero);
                }
            }
        }
    }
    None
}

fn extract_signon_player_name(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    if !lower.contains("[signonstate]") || !lower.contains("signon state") {
        return None;
    }
    let marker = "client 0 '";
    let pos = lower.find(marker)?;
    let rest = &line[pos + marker.len()..];
    let name = rest.split('\'').next()?.trim();
    if name.is_empty() || name.eq_ignore_ascii_case("unnamed") {
        return None;
    }
    let nl = name.to_ascii_lowercase();
    if nl
        .strip_prefix("bot")
        .is_some_and(|rest| rest.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    Some(name.to_owned())
}

/// Разбирает одну строку `Deadlock\game\citadel\console.log` и обновляет состояние сессии.
pub fn parse_console_line(state: &mut ConsoleState, line: &str, now_unix: u64) {
    let lower = line.to_ascii_lowercase();

    // Фиксируем entity ID ботов до фильтрации шума, чтобы отличать игрока в `[Server] Loaded hero`
    if let Some(pos) = lower.find("created bot ") {
        let rest = lower[pos + "created bot ".len()..].trim_start();
        let id_str = rest
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .unwrap_or("");
        if let Ok(bot_id) = id_str.parse::<u32>() {
            if state.bot_entity_ids.len() < 256 && !state.bot_entity_ids.contains(&bot_id) {
                state.bot_entity_ids.push(bot_id);
            }
        }
        return;
    }

    if let Some(name) = extract_signon_player_name(line) {
        state.player_name = Some(name);
    }

    if NOISE_PATTERNS.iter().any(|p| lower.contains(p)) {
        return;
    }

    if RANKED_MARKERS.iter().any(|m| lower.contains(m)) {
        state.ranked_hint = true;
        if state.mode == GameMode::StandardMatch {
            state.set_mode(GameMode::RankedMatch, now_unix);
        }
    }

    if let Some(map_name) = extract_map_name(line) {
        state.bot_entity_ids.clear();
        state.current_map = map_name.clone();
        if let Some(mode) = mode_for_map(&map_name, state.ranked_hint) {
            if matches!(mode, GameMode::MainMenu | GameMode::Hideout) {
                state.ranked_hint = false;
                state.hero = None;
            }
            state.set_mode(mode, now_unix);
        }
        return;
    }

    let in_hub_map = state.current_map.is_empty()
        || state.current_map == "<empty>"
        || state.current_map.contains("hideout")
        || state.current_map.contains("citadel_main");

    let explicit_draft = lower.contains("pregamevote")
        || lower.contains("pregame_vote")
        || lower.contains("herodraft")
        || lower.contains("hero_draft")
        || lower.contains("banphase")
        || lower.contains("ban_phase");

    if explicit_draft || (!in_hub_map && lower.contains("changegamestate: heroselection")) {
        state.set_mode(GameMode::PregameLobby, now_unix);
    } else if state.mode == GameMode::PregameLobby
        && (lower.contains("changegamestate: gameinprogress")
            || lower.contains("changegamestate: pregamewait")
            || lower.contains("changegamestate: matchintro"))
    {
        let mode =
            mode_for_map(&state.current_map, state.ranked_hint).unwrap_or(GameMode::StandardMatch);
        state.set_mode(mode, now_unix);
    }

    if let Some(hero) = extract_player_hero(&lower)
        .or_else(|| extract_server_loaded_hero(&lower, &state.bot_entity_ids))
        .or_else(|| extract_camera_pose_hero(&lower))
    {
        state.hero = Some(hero);
    }
}

/// Формирует JSON активности Discord Rich Presence (`details`, опционально `state`, `type`, `assets`, `timestamps`).
#[cfg(test)]
pub fn build_activity_json(
    console: &ConsoleState,
    cfg: &DiscordRpcConfig,
    steam_name: Option<&str>,
) -> serde_json::Value {
    build_activity_json_with_preset(console, cfg, steam_name, None)
}

/// Формирует JSON активности Discord Rich Presence с опциональным отображением активного пресета.
pub fn build_activity_json_with_preset(
    console: &ConsoleState,
    cfg: &DiscordRpcConfig,
    steam_name: Option<&str>,
    active_preset: Option<&str>,
) -> serde_json::Value {
    let mut obj = serde_json::json!({
        "details": console.mode.label(),
        "type": 0,
    });

    let hero = if cfg.show_hero { console.hero } else { None };
    let nick = if cfg.show_steam_name {
        steam_name
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                console
                    .player_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            })
    } else {
        None
    };

    let preset_part = if cfg.show_preset {
        active_preset.map(str::trim).filter(|s| !s.is_empty())
    } else {
        None
    };

    let base_state = match (hero, nick) {
        (Some(h), Some(n)) => Some(format!("{h} • {n}")),
        (Some(h), None) => Some(h.to_owned()),
        (None, Some(n)) => Some(n.to_owned()),
        (None, None) => None,
    };

    let state_text = match (base_state, preset_part) {
        (Some(base), Some(p)) => Some(format!("{base} • {p}")),
        (None, Some(p)) => Some(format!("Пресет: {p}")),
        (Some(base), None) => Some(base),
        (None, None) => None,
    }
    .map(|text| {
        if text.chars().count() > 128 {
            let mut clipped: String = text.chars().take(127).collect();
            clipped.push('…');
            clipped
        } else {
            text
        }
    });

    if let Some(s) = state_text {
        if s.chars().count() >= 2 {
            if let Some(map) = obj.as_object_mut() {
                map.insert("state".to_owned(), serde_json::json!(s));
            }
        }
    }

    let app_icon = {
        let trimmed = cfg.app_icon_url.trim();
        if !trimmed.is_empty() && !is_local_filesystem_path(trimmed) {
            Some(trimmed)
        } else {
            None
        }
    };
    let hero_icon = if cfg.show_hero && cfg.show_hero_icon {
        console
            .hero
            .and_then(|h| hero_icon_url(h).map(|u| (h, u)))
    } else {
        None
    };

    let assets = match (app_icon, hero_icon) {
        (Some(app), Some((hero_name, hero_url))) => {
            if cfg.hero_as_large_icon {
                Some(serde_json::json!({
                    "large_image": hero_url,
                    "large_text": hero_name,
                    "small_image": app,
                    "small_text": "ShaurMeaw",
                }))
            } else {
                Some(serde_json::json!({
                    "large_image": app,
                    "large_text": "ShaurMeaw",
                    "small_image": hero_url,
                    "small_text": hero_name,
                }))
            }
        }
        (Some(app), None) => Some(serde_json::json!({
            "large_image": app,
            "large_text": "ShaurMeaw",
        })),
        (None, Some((hero_name, hero_url))) => Some(serde_json::json!({
            "large_image": hero_url,
            "large_text": hero_name,
        })),
        (None, None) => None,
    };

    if let Some(map) = obj.as_object_mut() {
        if let Some(a) = assets {
            map.insert("assets".to_owned(), a);
        }
        if cfg.show_timer && console.mode.has_timer() {
            if let Some(ts) = console.started_at {
                map.insert("timestamps".to_owned(), serde_json::json!({ "start": ts }));
            }
        }
    }

    obj
}

/// Кодирует фрейм протокола Discord Named Pipe IPC (little-endian opcode + length + JSON).
pub fn encode_ipc_frame(opcode: u32, payload: &str) -> Vec<u8> {
    let bytes = payload.as_bytes();
    let mut out = Vec::with_capacity(8 + bytes.len());
    out.extend_from_slice(&opcode.to_le_bytes());
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
    out
}

pub fn is_valid_client_id(id: &str) -> bool {
    let trimmed = id.trim();
    (17..=20).contains(&trimmed.len()) && trimmed.bytes().all(|b| b.is_ascii_digit())
}

/// Возвращает эффективный Discord Application ID: встроенный для Deadlock при пустом поле,
/// либо пользовательский после проверки формата (17–20 цифр).
pub fn effective_client_id(custom: &str) -> Result<&str, String> {
    let trimmed = custom.trim();
    if trimmed.is_empty() {
        Ok(DEFAULT_DISCORD_APP_ID)
    } else if is_valid_client_id(trimmed) {
        Ok(trimmed)
    } else {
        Err("Discord Application ID должен содержать 17–20 цифр (или оставьте поле пустым)".into())
    }
}

/// Проверяет ответ Discord IPC на ошибки протокола (`evt: "ERROR"` или opcode `2` CLOSE).
pub fn parse_ipc_response(op: u32, body: &str) -> Result<(), String> {
    if op == 2 {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(body) {
            let code = val["code"]
                .as_i64()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".to_owned());
            let msg = val["message"]
                .as_str()
                .unwrap_or("неизвестная причина");
            return Err(format!("Discord закрыл IPC-соединение ({code}): {msg}"));
        }
        return Err("Discord закрыл IPC-соединение".into());
    }

    if let Ok(val) = serde_json::from_str::<serde_json::Value>(body) {
        if val["evt"].as_str() == Some("ERROR") {
            let code = val["data"]["code"]
                .as_i64()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".to_owned());
            let msg = val["data"]["message"]
                .as_str()
                .unwrap_or("неизвестная ошибка");
            return Err(format!("Discord IPC ошибка {code}: {msg}"));
        }
    }

    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RpcConnectionStatus {
    Disabled,
    #[default]
    WaitingForGame,
    NeedsAppId,
    WaitingForDiscord,
    Connected,
}

impl RpcConnectionStatus {
    pub fn label(self) -> &'static str {
        match self {
            RpcConnectionStatus::Disabled => "ВЫКЛЮЧЕНО",
            RpcConnectionStatus::WaitingForGame => "ОЖИДАНИЕ ИГРЫ",
            RpcConnectionStatus::NeedsAppId => "НУЖЕН APP ID",
            RpcConnectionStatus::WaitingForDiscord => "ОЖИДАНИЕ DISCORD",
            RpcConnectionStatus::Connected => "ПОДКЛЮЧЕНО",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RpcSnapshot {
    pub status: RpcConnectionStatus,
    pub details: String,
    pub state: String,
    pub hero: Option<String>,
    pub mode_label: String,
    pub last_error: Option<String>,
}

impl Default for RpcSnapshot {
    fn default() -> Self {
        Self {
            status: RpcConnectionStatus::WaitingForGame,
            details: GameMode::MainMenu.label().to_owned(),
            state: String::new(),
            hero: None,
            mode_label: GameMode::MainMenu.label().to_owned(),
            last_error: None,
        }
    }
}

#[cfg(windows)]
fn wait_pipe_readable(pipe: &File, timeout_ms: u64, shutdown: &AtomicBool) -> Result<(), String> {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;

    #[link(name = "kernel32")]
    extern "system" {
        fn PeekNamedPipe(
            h_named_pipe: *mut c_void,
            lp_buffer: *mut c_void,
            n_buffer_size: u32,
            lp_bytes_read: *mut u32,
            lp_total_bytes_avail: *mut u32,
            lp_bytes_left_this_message: *mut u32,
        ) -> i32;
    }

    let start = Instant::now();
    let handle = pipe.as_raw_handle();
    loop {
        if shutdown.load(Ordering::Acquire) {
            return Err("Остановка Discord RPC при выходе".into());
        }
        let mut avail: u32 = 0;
        let ok = unsafe {
            PeekNamedPipe(
                handle,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut avail,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err("Канал Discord IPC закрыт".into());
        }
        if avail > 0 {
            return Ok(());
        }
        if start.elapsed() >= Duration::from_millis(timeout_ms) {
            return Err("Таймаут ожидания ответа Discord IPC".into());
        }
        std::thread::sleep(Duration::from_millis(35));
    }
}

#[cfg(not(windows))]
fn wait_pipe_readable(_pipe: &File, _timeout_ms: u64, shutdown: &AtomicBool) -> Result<(), String> {
    if shutdown.load(Ordering::Acquire) {
        return Err("Остановка Discord RPC при выходе".into());
    }
    Ok(())
}

struct DiscordIpcClient {
    pipe: File,
    nonce: u64,
}

impl DiscordIpcClient {
    fn connect(client_id: &str, shutdown: &AtomicBool) -> Result<Self, String> {
        let mut last_err: Option<String> = None;
        for idx in 0..=9 {
            if shutdown.load(Ordering::Acquire) {
                return Err("Остановка Discord RPC при выходе".into());
            }
            let path = format!(r"\\.\pipe\discord-ipc-{idx}");
            let Ok(mut pipe) = OpenOptions::new().read(true).write(true).open(&path) else {
                continue;
            };
            let handshake = serde_json::json!({
                "v": 1,
                "client_id": client_id.trim(),
            })
            .to_string();
            if let Err(e) = pipe.write_all(&encode_ipc_frame(0, &handshake)) {
                last_err = Some(format!("Ошибка записи handshake в Discord IPC: {e}"));
                continue;
            }
            match Self::read_frame(&mut pipe, shutdown)
                .and_then(|(op, body)| parse_ipc_response(op, &body))
            {
                Ok(()) => return Ok(Self { pipe, nonce: 1 }),
                Err(e) => {
                    last_err = Some(e);
                }
            }
        }
        Err(last_err
            .unwrap_or_else(|| "Discord IPC не обнаружен (запустите десктопный Discord)".into()))
    }

    fn read_frame(pipe: &mut File, shutdown: &AtomicBool) -> Result<(u32, String), String> {
        wait_pipe_readable(pipe, 3_000, shutdown)?;
        let mut hdr = [0u8; 8];
        pipe.read_exact(&mut hdr)
            .map_err(|e| format!("Ошибка чтения заголовка Discord IPC: {e}"))?;
        let op = u32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
        let len = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as usize;
        if len > 65_536 {
            return Err("Слишком большой ответ Discord IPC".into());
        }
        if len > 0 {
            wait_pipe_readable(pipe, 3_000, shutdown)?;
        }
        let mut buf = vec![0u8; len];
        pipe.read_exact(&mut buf)
            .map_err(|e| format!("Ошибка чтения тела Discord IPC: {e}"))?;
        Ok((op, String::from_utf8_lossy(&buf).into_owned()))
    }

    fn send_activity(
        &mut self,
        pid: u32,
        activity: Option<&serde_json::Value>,
        shutdown: &AtomicBool,
    ) -> Result<(), String> {
        if shutdown.load(Ordering::Acquire) {
            return Err("Остановка Discord RPC при выходе".into());
        }
        self.nonce = self.nonce.wrapping_add(1);
        let args = match activity {
            Some(act) => serde_json::json!({ "pid": pid, "activity": act }),
            None => serde_json::json!({ "pid": pid }),
        };
        let frame_json = serde_json::json!({
            "cmd": "SET_ACTIVITY",
            "args": args,
            "nonce": format!("shaurmeaw-{}", self.nonce),
        })
        .to_string();

        self.pipe
            .write_all(&encode_ipc_frame(1, &frame_json))
            .map_err(|e| format!("Ошибка записи в Discord IPC: {e}"))?;
        let (op, body) = Self::read_frame(&mut self.pipe, shutdown)?;
        parse_ipc_response(op, &body)?;
        Ok(())
    }
}

#[derive(Clone)]
struct RpcShared {
    cfg: DiscordRpcConfig,
    steam_path: String,
    game_path: String,
    active_preset: String,
    game_running: bool,
    revision: u64,
    force_reconnect: bool,
    shutdown: bool,
}

/// Фоновый сервис Discord Rich Presence, отслеживающий `console.log` во время игры.
pub struct DiscordRpcService {
    shared: Arc<Mutex<RpcShared>>,
    snapshot: Arc<Mutex<RpcSnapshot>>,
    snap_rev: Arc<AtomicU64>,
    shutdown_flag: Arc<AtomicBool>,
    wake: Option<Arc<WinEvent>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Читает новые строки `console.log` с `offset`, переиспользуя переданные буферы без лишних аллокаций.
/// Возвращает `true`, если состояние консоли могло измениться (прочитаны строки или файл удалён/усечён).
fn read_console_tail_or_append(
    log_path: &Path,
    offset: &mut u64,
    console: &mut ConsoleState,
    read_buf: &mut Vec<u8>,
    text_buf: &mut String,
    now_unix: u64,
) -> bool {
    const CHUNK_WINDOW: u64 = 2 * 1024 * 1024;

    let Ok(meta) = fs::metadata(log_path) else {
        if *offset != 0 || *console != ConsoleState::default() {
            *offset = 0;
            *console = ConsoleState::default();
            return true;
        }
        return false;
    };
    let len = meta.len();
    if len == *offset {
        return false;
    }

    let mut changed = false;
    let was_truncated = len < *offset;
    let was_zero = *offset == 0;
    if was_truncated {
        *offset = 0;
        *console = ConsoleState::default();
        changed = true;
        if len == 0 {
            return true;
        }
    } else if was_zero {
        *console = ConsoleState::default();
        changed = true;
    }

    let Ok(mut reader) = File::open(log_path) else {
        return changed;
    };

    let start = if was_truncated || was_zero {
        len.saturating_sub(CHUNK_WINDOW)
    } else {
        *offset
    };

    if reader.seek(SeekFrom::Start(start)).is_err() {
        return changed;
    }

    read_buf.clear();
    if read_buf.capacity() < crate::fs_safe::IO_BUF_CAPACITY {
        read_buf.reserve(crate::fs_safe::IO_BUF_CAPACITY - read_buf.capacity());
    }
    let buf_reader = BufReader::with_capacity(crate::fs_safe::IO_BUF_CAPACITY, reader);
    if buf_reader.take(CHUNK_WINDOW).read_to_end(read_buf).is_err() {
        return changed;
    }

    let consumed = if read_buf.is_empty() {
        0
    } else {
        read_buf
            .iter()
            .rposition(|&b| b == b'\n')
            .map(|p| p + 1)
            .unwrap_or(read_buf.len())
    };

    if consumed > 0 {
        text_buf.clear();
        match std::str::from_utf8(&read_buf[..consumed]) {
            Ok(s) => text_buf.push_str(s),
            Err(_) => {
                let lossy = String::from_utf8_lossy(&read_buf[..consumed]);
                text_buf.push_str(&lossy);
            }
        }
        let skip_partial_first_line = start > 0 && (was_zero || was_truncated);
        let lines_iter = if skip_partial_first_line {
            text_buf.split_once('\n').map(|(_, rest)| rest).unwrap_or("")
        } else {
            text_buf.as_str()
        };
        for line in lines_iter.lines() {
            parse_console_line(console, line, now_unix);
        }
        *offset = start + consumed as u64;
        changed = true;
    }

    changed
}

impl DiscordRpcService {
    pub fn spawn(
        initial: &Settings,
        ctx: egui::Context,
        window_minimized: Arc<AtomicBool>,
        game_running_shared: Arc<AtomicBool>,
    ) -> Self {
        let shared = Arc::new(Mutex::new(RpcShared {
            cfg: initial.discord_rpc.clone(),
            steam_path: initial.steam_path.clone(),
            game_path: initial.game_path.clone(),
            active_preset: initial.gameinfo.active_preset.clone(),
            game_running: game_running_shared.load(Ordering::Acquire),
            revision: 1,
            force_reconnect: false,
            shutdown: false,
        }));
        let initial_snap = RpcSnapshot {
            status: if initial.discord_rpc.enabled {
                RpcConnectionStatus::WaitingForGame
            } else {
                RpcConnectionStatus::Disabled
            },
            details: GameMode::MainMenu.label().to_owned(),
            state: String::new(),
            hero: None,
            mode_label: GameMode::MainMenu.label().to_owned(),
            last_error: None,
        };
        let snapshot = Arc::new(Mutex::new(initial_snap));
        let snap_rev = Arc::new(AtomicU64::new(1));
        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let wake = WinEvent::new().map(Arc::new);

        let shared_bg = Arc::clone(&shared);
        let snap_bg = Arc::clone(&snapshot);
        let rev_bg = Arc::clone(&snap_rev);
        let shutdown_bg = Arc::clone(&shutdown_flag);
        let min_flag = Arc::clone(&window_minimized);
        let game_bg = Arc::clone(&game_running_shared);
        let wake_bg = wake.as_ref().map(Arc::clone);

        let worker = std::thread::Builder::new()
            .name("shaurmeaw-discord".to_owned())
            .spawn(move || {
                let mut console = ConsoleState::default();
                let mut log_offset = 0u64;
                let mut read_buf: Vec<u8> = Vec::with_capacity(crate::fs_safe::IO_BUF_CAPACITY);
                let mut text_buf = String::with_capacity(crate::fs_safe::IO_BUF_CAPACITY);
                let mut client: Option<(String, DiscordIpcClient)> = None;
                let mut cached_activity: Option<(serde_json::Value, String)> = None;
                let mut last_built_rev = 0u64;
                let mut last_built_steam_name: Option<String> = None;
                let mut last_sent_json: Option<String> = None;
                let mut last_sent_at = 0u64;
                let mut cached_steam_name: Option<String> = None;
                let mut last_steam_check = 0u64;
                let mut last_steam_path = String::new();
                let mut last_connect_attempt = 0u64;
                let mut last_client_id = String::new();
                let mut last_ipc_err: Option<String> = None;
                let ipc_pid = std::process::id();

                loop {
                    let (should_sleep, state_copy, handled_rev) = {
                        let mut guard = shared_bg.lock().unwrap_or_else(|e| e.into_inner());
                        if guard.shutdown || shutdown_bg.load(Ordering::Acquire) {
                            drop(guard);
                            drop(client.take());
                            break;
                        }
                        let game_up = guard.game_running || game_bg.load(Ordering::Acquire);
                        if !guard.cfg.enabled || !game_up {
                            let status = if !guard.cfg.enabled {
                                RpcConnectionStatus::Disabled
                            } else {
                                RpcConnectionStatus::WaitingForGame
                            };
                            let copy = guard.clone();
                            guard.force_reconnect = false;
                            drop(guard);

                            let idle_activity = build_activity_json_with_preset(
                                &ConsoleState::default(),
                                &copy.cfg,
                                None,
                                Some(copy.active_preset.as_str()),
                            );
                            let new_snap = RpcSnapshot {
                                status,
                                details: idle_activity["details"].as_str().unwrap_or("").to_owned(),
                                state: idle_activity["state"].as_str().unwrap_or("").to_owned(),
                                hero: None,
                                mode_label: GameMode::MainMenu.label().to_owned(),
                                last_error: None,
                            };
                            drop(idle_activity);
                            if let Ok(mut sg) = snap_bg.lock() {
                                if *sg != new_snap {
                                    *sg = new_snap;
                                    rev_bg.fetch_add(1, Ordering::Release);
                                    if !min_flag.load(Ordering::Relaxed) {
                                        ctx.request_repaint();
                                    }
                                }
                            }
                            if let Some((_, mut c)) = client.take() {
                                if !shutdown_bg.load(Ordering::Acquire) {
                                    let _ = c.send_activity(ipc_pid, None, &shutdown_bg);
                                }
                            }
                            console = ConsoleState::default();
                            log_offset = 0;
                            read_buf.clear();
                            read_buf.shrink_to(0);
                            text_buf.clear();
                            text_buf.shrink_to(0);
                            cached_activity = None;
                            last_built_rev = 0;
                            last_built_steam_name = None;
                            last_sent_json = None;
                            last_steam_check = 0;
                            last_connect_attempt = 0;
                            last_ipc_err = None;
                            (true, copy, 0)
                        } else {
                            let copy = guard.clone();
                            let rev = guard.revision;
                            guard.force_reconnect = false;
                            (false, copy, rev)
                        }
                    };

                    if should_sleep {
                        if let Some(ref ev) = wake_bg {
                            let _ = wait_event_or_dirs(ev, &[], WAIT_INFINITE);
                        } else {
                            std::thread::sleep(Duration::from_secs(3));
                        }
                        continue;
                    }

                    let now = unix_now();
                    if last_steam_path != state_copy.steam_path
                        || last_steam_check == 0
                        || now.saturating_sub(last_steam_check) >= 10
                    {
                        cached_steam_name =
                            crate::steam::current_persona_name(Path::new(&state_copy.steam_path));
                        last_steam_path.clone_from(&state_copy.steam_path);
                        last_steam_check = now.max(1);
                    }

                    let clog = Path::new(&state_copy.game_path)
                        .join("game")
                        .join("citadel")
                        .join("console.log");
                    let parsed_new = read_console_tail_or_append(
                        &clog,
                        &mut log_offset,
                        &mut console,
                        &mut read_buf,
                        &mut text_buf,
                        now,
                    );

                    // Пересчитываем JSON только при изменении состояния консоли, конфига или имени игрока
                    let need_rebuild = cached_activity.is_none()
                        || parsed_new
                        || handled_rev != last_built_rev
                        || cached_steam_name != last_built_steam_name
                        || state_copy.force_reconnect;
                    if need_rebuild {
                        let act = build_activity_json_with_preset(
                            &console,
                            &state_copy.cfg,
                            cached_steam_name.as_deref(),
                            Some(state_copy.active_preset.as_str()),
                        );
                        let act_str = act.to_string();
                        cached_activity = Some((act, act_str));
                        last_built_rev = handled_rev;
                        last_built_steam_name = cached_steam_name.clone();
                    }

                    let Some((ref activity, ref activity_str)) = cached_activity else {
                        continue;
                    };

                    if state_copy.force_reconnect {
                        if let Some((_, mut c)) = client.take() {
                            if !shutdown_bg.load(Ordering::Acquire) {
                                let _ = c.send_activity(ipc_pid, None, &shutdown_bg);
                            }
                        }
                        last_sent_json = None;
                    }

                    let (status, last_error) = match effective_client_id(&state_copy.cfg.client_id) {
                        Err(e) => {
                            if let Some((_, mut c)) = client.take() {
                                if !shutdown_bg.load(Ordering::Acquire) {
                                    let _ = c.send_activity(ipc_pid, None, &shutdown_bg);
                                }
                            }
                            last_sent_json = None;
                            last_connect_attempt = 0;
                            last_ipc_err = None;
                            (RpcConnectionStatus::NeedsAppId, Some(e))
                        }
                        Ok(cid) => {
                            let mut ipc_err: Option<String> = None;
                            let need_reconnect = client
                                .as_ref()
                                .is_none_or(|(bound_cid, _)| bound_cid != cid);
                            if need_reconnect {
                                if let Some((_, mut old_c)) = client.take() {
                                    if !shutdown_bg.load(Ordering::Acquire) {
                                        let _ = old_c.send_activity(ipc_pid, None, &shutdown_bg);
                                    }
                                }
                                let should_try = state_copy.force_reconnect
                                    || last_client_id != cid
                                    || last_connect_attempt == 0
                                    || now.saturating_sub(last_connect_attempt) >= 5;
                                if should_try && !shutdown_bg.load(Ordering::Acquire) {
                                    last_client_id = cid.to_owned();
                                    last_connect_attempt = now.max(1);
                                    match DiscordIpcClient::connect(cid, &shutdown_bg) {
                                        Ok(c) => {
                                            client = Some((cid.to_owned(), c));
                                            last_sent_json = None;
                                            last_ipc_err = None;
                                        }
                                        Err(e) => {
                                            last_ipc_err = Some(e.clone());
                                            ipc_err = Some(e);
                                        }
                                    }
                                } else {
                                    ipc_err = last_ipc_err.clone();
                                }
                            }

                            if let Some((_, ref mut ipc)) = client {
                                let should_send = last_sent_json.as_deref() != Some(activity_str)
                                    || now.saturating_sub(last_sent_at) >= 30;
                                if should_send {
                                    match ipc.send_activity(ipc_pid, Some(activity), &shutdown_bg) {
                                        Ok(()) => {
                                            last_sent_json = Some(activity_str.clone());
                                            last_sent_at = now;
                                            last_ipc_err = None;
                                            (RpcConnectionStatus::Connected, None)
                                        }
                                        Err(e) => {
                                            client = None;
                                            last_sent_json = None;
                                            last_connect_attempt = now.max(1);
                                            last_ipc_err = Some(e.clone());
                                            (RpcConnectionStatus::WaitingForDiscord, Some(e))
                                        }
                                    }
                                } else {
                                    (RpcConnectionStatus::Connected, None)
                                }
                            } else {
                                (RpcConnectionStatus::WaitingForDiscord, ipc_err)
                            }
                        }
                    };

                    if shutdown_bg.load(Ordering::Acquire) {
                        drop(client.take());
                        break;
                    }

                    let new_snap = RpcSnapshot {
                        status,
                        details: activity["details"].as_str().unwrap_or("").to_owned(),
                        state: activity["state"].as_str().unwrap_or("").to_owned(),
                        hero: console.hero.map(str::to_owned),
                        mode_label: console.mode.label().to_owned(),
                        last_error,
                    };
                    if let Ok(mut sg) = snap_bg.lock() {
                        if *sg != new_snap {
                            *sg = new_snap;
                            rev_bg.fetch_add(1, Ordering::Release);
                            if !min_flag.load(Ordering::Relaxed) {
                                ctx.request_repaint();
                            }
                        }
                    }

                    let still_active = shared_bg
                        .lock()
                        .map(|g| {
                            !g.shutdown
                                && g.revision == handled_rev
                                && g.cfg.enabled
                                && (g.game_running || game_bg.load(Ordering::Acquire))
                        })
                        .unwrap_or(false);
                    if still_active {
                        // Не используем DirChangeHandle на папку с console.log: игра пишет в него
                        // десятки-сотни строк в секунду; редкий опрос 2.5с / 8с исключает шторм пробуждений.
                        let sleep_ms: u32 = if client.is_some() { 2500 } else { 8000 };
                        if let Some(ref ev) = wake_bg {
                            let _ = wait_event_or_dirs(ev, &[], sleep_ms);
                        } else {
                            std::thread::sleep(Duration::from_millis(u64::from(sleep_ms)));
                        }
                    }
                }
            })
            .ok();

        Self {
            shared,
            snapshot,
            snap_rev,
            shutdown_flag,
            wake,
            worker,
        }
    }

    pub fn wake_handle(&self) -> Option<Arc<WinEvent>> {
        self.wake.as_ref().map(Arc::clone)
    }

    pub fn sync(&self, settings: &Settings, game_running: bool, _game_pid: Option<u32>) {
        if let Ok(mut guard) = self.shared.lock() {
            guard.cfg = settings.discord_rpc.clone();
            guard.steam_path.clone_from(&settings.steam_path);
            guard.game_path.clone_from(&settings.game_path);
            guard.active_preset.clone_from(&settings.gameinfo.active_preset);
            guard.game_running = game_running;
            guard.revision = guard.revision.wrapping_add(1);
        }
        if let Some(ref w) = self.wake {
            w.signal();
        }
    }

    pub fn reconnect(&self) {
        if let Ok(mut guard) = self.shared.lock() {
            guard.force_reconnect = true;
            guard.revision = guard.revision.wrapping_add(1);
        }
        if let Some(ref w) = self.wake {
            w.signal();
        }
    }

    /// Обновляет `out` и `last_rev` только при реальном изменении снапшота (без блокировки в покое).
    pub fn snapshot_if_changed(&self, last_rev: &mut u64, out: &mut RpcSnapshot) -> bool {
        let cur = self.snap_rev.load(Ordering::Acquire);
        if cur == *last_rev {
            return false;
        }
        if let Ok(guard) = self.snapshot.lock() {
            *out = guard.clone();
            *last_rev = cur;
            return true;
        }
        false
    }

    pub fn shutdown(&mut self) {
        self.shutdown_flag.store(true, Ordering::Release);
        if let Ok(mut guard) = self.shared.lock() {
            guard.shutdown = true;
        }
        if let Some(ref w) = self.wake {
            w.signal();
        }
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for DiscordRpcService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_all_44_heroes_and_aliases_mapping() {
        assert_eq!(ALL_HEROES.len(), 44);
        let unique: HashSet<&str> = ALL_HEROES.iter().copied().collect();
        assert_eq!(unique.len(), 44, "В ALL_HEROES есть дубликаты");

        let cases = [
            // Базовые + 2025 + кодовые алиасы
            ("abrams", "Abrams"),
            ("hero_atlas", "Abrams"),
            ("bebop", "Bebop"),
            ("billy", "Billy"),
            ("hero_punkgoat", "Billy"),
            ("calico", "Calico"),
            ("doorman", "The Doorman"),
            ("drifter", "Drifter"),
            ("dynamo", "Dynamo"),
            ("grey_talon", "Grey Talon"),
            ("hero_orion", "Grey Talon"),
            ("haze", "Haze"),
            ("holliday", "Holliday"),
            ("hero_astro", "Holliday"),
            ("inferno", "Infernus"),
            ("infernus", "Infernus"),
            ("ivy", "Ivy"),
            ("hero_tengu", "Ivy"),
            ("kelvin", "Kelvin"),
            ("lady_geist", "Lady Geist"),
            ("hero_geist", "Lady Geist"),
            ("lash", "Lash"),
            ("mcginnis", "McGinnis"),
            ("hero_forge", "McGinnis"),
            ("mina", "Mina"),
            ("mirage", "Mirage"),
            ("mo_and_krill", "Mo & Krill"),
            ("hero_krill", "Mo & Krill"),
            ("digger", "Mo & Krill"),
            ("paige", "Paige"),
            ("paradox", "Paradox"),
            ("hero_chrono", "Paradox"),
            ("pocket", "Pocket"),
            ("hero_synth", "Pocket"),
            ("seven", "Seven"),
            ("hero_gigawatt", "Seven"),
            ("shiv", "Shiv"),
            ("sinclair", "Sinclair"),
            ("hero_magician", "Sinclair"),
            ("victor", "Victor"),
            ("vindicta", "Vindicta"),
            ("hero_hornet", "Vindicta"),
            ("viscous", "Viscous"),
            ("vyper", "Vyper"),
            ("viper", "Vyper"),
            ("warden", "Warden"),
            ("wraith", "Wraith"),
            ("yamato", "Yamato"),
            // Начало 2026 (6)
            ("hero_rem", "Rem"),
            ("hero_graves", "Graves"),
            ("hero_silver", "Silver"),
            ("hero_venator", "Venator"),
            ("hero_celeste", "Celeste"),
            ("hero_apollo", "Apollo"),
            // Осень 2026 — City Never Sleeps (6)
            ("hero_rat_king", "Rat King"),
            ("baba", "Baba"),
            ("hero_solomon", "Solomon"),
            ("hero_chessmaster", "Solomon"),
            ("deadman_danny", "Deadman Danny"),
            ("hero_danny", "Deadman Danny"),
            ("hero_deadpack", "Deadman Danny"),
            ("citadel_hero_violet", "Violet"),
            ("hero_artist", "Violet"),
            ("nurse_harrow", "Nurse Harrow"),
            ("hero_harrow", "Nurse Harrow"),
            ("hero_nurse", "Nurse Harrow"),
            // Официальные кодовые имена Build 6864 (Source 2)
            ("hero_ghost", "Lady Geist"),
            ("hero_familiar", "Rem"),
            ("hero_necro", "Graves"),
            ("hero_werewolf", "Silver"),
            ("hero_priest", "Venator"),
            ("hero_unicorn", "Celeste"),
            ("hero_fencer", "Apollo"),
            ("hero_ratking", "Rat King"),
            ("bull", "Abrams"),
            ("archer", "Grey Talon"),
            ("spectre", "Lady Geist"),
            ("engineer", "McGinnis"),
            ("sumo", "Dynamo"),
        ];

        for (raw, expected) in cases {
            assert_eq!(
                resolve_hero_name(raw),
                Some(expected),
                "Неверное сопоставление для `{raw}`"
            );
        }

        for hero in ALL_HEROES {
            let url = hero_icon_url(hero).expect("должен быть URL иконки для каждого из 44 героев");
            assert!(
                url.starts_with("https://assets-bucket.deadlock-api.com/assets-api-res/images/heroes/"),
                "{hero}: {url}"
            );
            assert!(url.ends_with("_sm.png"), "{hero}: {url}");
        }
    }

    #[test]
    fn test_console_log_parser_modes_ranked_and_hero() {
        let mut st = ConsoleState::default();

        // Реальный шум при старте НЕ должен ложно переключать режим или ставить бота в hero
        parse_console_line(
            &mut st,
            "11/06 20:55:26 [Server] street_brawl_state_changed overrides identical base gameevent",
            100,
        );
        assert_eq!(st.mode, GameMode::MainMenu);

        parse_console_line(&mut st, "11/06 20:55:31 Loading map \"dl_hideout\"", 101);
        assert_eq!(st.mode, GameMode::Hideout);
        assert!(st.started_at.is_none());

        // Боты и ChangeGameState в hideout игнорируются
        parse_console_line(
            &mut st,
            "11/06 20:55:34 Created bot 298/hero_inferno/hero_inferno at location 11877",
            102,
        );
        parse_console_line(&mut st, "11/06 20:55:35 ChangeGameState: HeroSelection (3)", 103);
        assert_eq!(st.mode, GameMode::Hideout);
        assert_eq!(st.hero, None);

        // Sandbox
        parse_console_line(&mut st, "Loading map \"street_test\"", 200);
        assert_eq!(st.mode, GameMode::Sandbox);
        assert_eq!(st.started_at, Some(200));

        // Предматчевое лобби и выбор нового героя осени 2026
        parse_console_line(&mut st, "[Lobby] PregameVote started", 250);
        assert_eq!(st.mode, GameMode::PregameLobby);
        assert!(st.started_at.is_none());
        parse_console_line(&mut st, "[HeroSelect] selecthero hero_rat_king", 255);
        assert_eq!(st.hero, Some("Rat King"));

        // Рейтинговый матч
        parse_console_line(&mut st, "[GC] match_mode=ranked k_ECitadelGameMode_Ranked", 300);
        parse_console_line(&mut st, "[Client] Map: \"dl_midtown\"", 301);
        assert_eq!(st.mode, GameMode::RankedMatch);
        assert_eq!(st.started_at, Some(301));

        // Потасовка Street Brawl и другой герой осени 2026
        parse_console_line(&mut st, "Loading map \"street_brawl\"", 500);
        parse_console_line(&mut st, "selected hero: hero_nurse_harrow", 505);
        assert_eq!(st.mode, GameMode::StreetBrawl);
        assert_eq!(st.hero, Some("Nurse Harrow"));
        assert_eq!(st.started_at, Some(500));
    }

    #[test]
    fn test_effective_client_id_default_and_custom() {
        assert_eq!(effective_client_id(""), Ok(DEFAULT_DISCORD_APP_ID));
        assert_eq!(effective_client_id("   "), Ok(DEFAULT_DISCORD_APP_ID));
        assert_eq!(
            effective_client_id(" 123456789012345678 "),
            Ok("123456789012345678")
        );
        assert!(effective_client_id("12345").is_err());
        assert!(effective_client_id("abc123456789012345").is_err());
    }

    #[test]
    fn test_parse_ipc_response_ok_and_error() {
        let ready = r#"{"cmd":"DISPATCH","evt":"READY","data":{"v":1}}"#;
        assert_eq!(parse_ipc_response(1, ready), Ok(()));

        let err = r#"{"cmd":"SET_ACTIVITY","evt":"ERROR","data":{"code":4000,"message":"Invalid Client ID"}}"#;
        let res = parse_ipc_response(1, err).unwrap_err();
        assert!(res.contains("4000"), "{res}");
        assert!(res.contains("Invalid Client ID"), "{res}");

        let close = r#"{"code":4000,"message":"Invalid client ID"}"#;
        let close_res = parse_ipc_response(2, close).unwrap_err();
        assert!(close_res.contains("4000"), "{close_res}");
    }

    #[test]
    fn test_console_log_live_build_6864_sequence() {
        let mut st = ConsoleState::default();

        // Hideout: боты 298 и 300 должны быть отфильтрованы, а локальный 304/hero_gigawatt — засчитан
        parse_console_line(&mut st, r#"11/21 23:24:12 Loading map "dl_hideout""#, 100);
        parse_console_line(
            &mut st,
            "11/21 23:24:15 Created bot 298/hero_inferno/hero_inferno at location 11877.875",
            101,
        );
        parse_console_line(
            &mut st,
            "11/21 23:24:15 Created bot 300/hero_gigawatt/hero_gigawatt at location 11685.875",
            101,
        );
        parse_console_line(&mut st, "11/21 23:24:16 [Server] Loaded hero 304/hero_gigawatt", 102);
        parse_console_line(&mut st, "11/21 23:24:17 [Server] Loaded hero 298/hero_inferno", 102);
        parse_console_line(&mut st, "11/21 23:24:17 [Server] Loaded hero 300/hero_atlas", 102);
        assert_eq!(st.mode, GameMode::Hideout);
        assert_eq!(st.hero, Some("Seven"));
        assert!(st.started_at.is_none());

        // Переход в матч dl_midtown: кратковременный HeroSelection не должен оставлять PregameLobby навсегда
        parse_console_line(&mut st, r#"11/21 23:25:53 Loading map "dl_midtown""#, 200);
        parse_console_line(&mut st, "11/21 23:25:59 ChangeGameState: HeroSelection (3)", 205);
        assert_eq!(st.mode, GameMode::PregameLobby);
        parse_console_line(&mut st, "11/21 23:25:59 ChangeGameState: GameInProgress (7)", 206);
        parse_console_line(&mut st, "11/21 23:25:59 [Server] Loaded hero 2799/hero_gigawatt", 207);
        assert_eq!(st.mode, GameMode::StandardMatch);
        assert_eq!(st.hero, Some("Seven"));
        assert!(st.started_at.is_some());

        // Смена персонажа на Silver (hero_werewolf) и резервное определение по VMDL Camera Pose
        parse_console_line(
            &mut st,
            "11/21 23:25:59 [SignonState] Client 0 'ShaurMeaw' signon state SIGNONSTATE_SPAWN -> SIGNONSTATE_FULL",
            210,
        );
        assert_eq!(st.player_name.as_deref(), Some("ShaurMeaw"));
        parse_console_line(&mut st, "11/21 23:40:55 [Server] Loaded hero 2799/hero_werewolf", 220);
        assert_eq!(st.hero, Some("Silver"));
        parse_console_line(
            &mut st,
            r#"VMDL Camera Pose Success! "models/heroes_staging/gigawatt_prisoner/gigawatt_prisoner.vmdl" has loaded camera poses from vmdl"#,
            230,
        );
        assert_eq!(st.hero, Some("Seven"));
    }

    #[test]
    fn test_build_activity_assets_combinations() {
        let mut st = ConsoleState::default();
        st.mode = GameMode::StandardMatch;
        st.hero = Some("Silver");
        st.started_at = Some(1_795_200_100);

        // 1. Без app_icon_url, с героем и ником Steam -> "Silver • ShaurMeaw"
        let cfg_hero_only = DiscordRpcConfig::default();
        let a1 = build_activity_json(&st, &cfg_hero_only, Some("ShaurMeaw"));
        assert_eq!(a1["details"].as_str(), Some("The Cursed Apple"));
        assert_eq!(a1["state"].as_str(), Some("Silver • ShaurMeaw"));
        assert_eq!(
            a1["assets"]["large_image"].as_str(),
            Some("https://assets-bucket.deadlock-api.com/assets-api-res/images/heroes/werewolf_sm.png")
        );
        assert_eq!(a1["assets"]["large_text"].as_str(), Some("Silver"));
        assert!(a1["assets"].get("small_image").is_none());

        // 1б. С отключенным show_steam_name -> только герой
        let mut cfg_no_nick = DiscordRpcConfig::default();
        cfg_no_nick.show_steam_name = false;
        let a1_no_nick = build_activity_json(&st, &cfg_no_nick, Some("ShaurMeaw"));
        assert_eq!(a1_no_nick["state"].as_str(), Some("Silver"));

        // 2. С app_icon_url и hero_as_large_icon=false -> app крупно, персонаж в углу
        let mut cfg_app_first = DiscordRpcConfig::default();
        cfg_app_first.app_icon_url = "https://example.com/shaurmeaw.png".into();
        let a2 = build_activity_json(&st, &cfg_app_first, Some("ShaurMeaw"));
        assert_eq!(
            a2["assets"]["large_image"].as_str(),
            Some("https://example.com/shaurmeaw.png")
        );
        assert_eq!(a2["assets"]["large_text"].as_str(), Some("ShaurMeaw"));
        assert_eq!(
            a2["assets"]["small_image"].as_str(),
            Some("https://assets-bucket.deadlock-api.com/assets-api-res/images/heroes/werewolf_sm.png")
        );
        assert_eq!(a2["assets"]["small_text"].as_str(), Some("Silver"));

        // 3. hero_as_large_icon=true -> персонаж крупно, app в углу
        cfg_app_first.hero_as_large_icon = true;
        let a3 = build_activity_json(&st, &cfg_app_first, Some("ShaurMeaw"));
        assert_eq!(
            a3["assets"]["large_image"].as_str(),
            Some("https://assets-bucket.deadlock-api.com/assets-api-res/images/heroes/werewolf_sm.png")
        );
        assert_eq!(
            a3["assets"]["small_image"].as_str(),
            Some("https://example.com/shaurmeaw.png")
        );

        // 4. Локальный путь C:\icon.png без героя: с ником -> state = ник, без ника -> state отсутствует
        let mut st_no_hero = ConsoleState::default();
        st_no_hero.mode = GameMode::Hideout;
        let mut cfg_local = DiscordRpcConfig::default();
        cfg_local.app_icon_url = r"C:\icon.png".into();
        let a4_nick = build_activity_json(&st_no_hero, &cfg_local, Some("ShaurMeaw"));
        assert_eq!(a4_nick["details"].as_str(), Some("Hideout"));
        assert_eq!(a4_nick["state"].as_str(), Some("ShaurMeaw"));
        assert!(a4_nick.get("assets").is_none());

        let a4 = build_activity_json(&st_no_hero, &cfg_local, None);
        assert_eq!(a4["details"].as_str(), Some("Hideout"));
        assert!(a4.get("state").is_none());
        assert!(a4.get("assets").is_none());
    }

    #[test]
    fn test_ipc_frame_encoding_and_activity_json() {
        let mut st = ConsoleState::default();
        st.mode = GameMode::RankedMatch;
        st.hero = Some("Rat King");
        st.started_at = Some(1_795_200_000);

        let cfg = DiscordRpcConfig::default();
        let activity = build_activity_json(&st, &cfg, Some("ShaurMeaw"));
        assert_eq!(
            activity["assets"]["large_image"].as_str(),
            Some("https://assets-bucket.deadlock-api.com/assets-api-res/images/heroes/ratking_sm.png")
        );
        assert_eq!(activity["assets"]["large_text"].as_str(), Some("Rat King"));
        assert_eq!(activity["type"].as_u64(), Some(0));
        assert_eq!(
            activity["details"].as_str(),
            Some("Ranked • The Cursed Apple")
        );
        assert_eq!(activity["state"].as_str(), Some("Rat King • ShaurMeaw"));
        assert_eq!(activity["timestamps"]["start"].as_u64(), Some(1_795_200_000));

        let payload = activity.to_string();
        assert!(!payload.contains("Пресет"), "{payload}");
        let frame = encode_ipc_frame(1, &payload);
        assert_eq!(u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]), 1);
        assert_eq!(
            u32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]]) as usize,
            payload.len()
        );
        assert_eq!(&frame[8..], payload.as_bytes());
    }

    #[test]
    fn test_read_console_tail_or_append_reuses_buffers_and_handles_truncate() {
        let dir = std::env::temp_dir().join(format!(
            "shaurmeaw_rpc_tail_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let log = dir.join("console.log");

        fs::write(
            &log,
            "11/21 23:25:59 ChangeGameUIState: DOTA_GAME_UI_STATE_GAME_IN_PROGRESS\n\
             11/21 23:26:01 Loaded hero hero_atlas\n",
        )
        .unwrap();

        let mut offset = 0u64;
        let mut console = ConsoleState::default();
        let mut read_buf = Vec::new();
        let mut text_buf = String::new();

        assert!(read_console_tail_or_append(
            &log,
            &mut offset,
            &mut console,
            &mut read_buf,
            &mut text_buf,
            1000
        ));
        assert_eq!(console.hero, Some("Abrams"));
        let saved_offset = offset;

        // Повторный вызов без изменений не читает лишнего и возвращает false
        assert!(!read_console_tail_or_append(
            &log,
            &mut offset,
            &mut console,
            &mut read_buf,
            &mut text_buf,
            1001
        ));
        assert_eq!(offset, saved_offset);

        // Дописываем нового героя — читается только хвост
        {
            let mut f = OpenOptions::new().append(true).open(&log).unwrap();
            writeln!(f, "11/21 23:26:15 Loaded hero hero_haze").unwrap();
        }
        assert!(read_console_tail_or_append(
            &log,
            &mut offset,
            &mut console,
            &mut read_buf,
            &mut text_buf,
            1002
        ));
        assert_eq!(console.hero, Some("Haze"));

        // При усечении файла (перезапуск игры) состояние сбрасывается и перечитывается заново
        fs::write(&log, "11/21 23:30:00 Loaded hero hero_kelvin\n").unwrap();
        assert!(read_console_tail_or_append(
            &log,
            &mut offset,
            &mut console,
            &mut read_buf,
            &mut text_buf,
            1003
        ));
        assert_eq!(console.hero, Some("Kelvin"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_console_reader_resets_on_delete_truncate_long_line_and_same_map_bots() {
        let dir = std::env::temp_dir().join(format!(
            "shaurmeaw_rpc_edge_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let log = dir.join("console.log");

        // 1. Ошибка при удалении файла должна сбрасывать offset и ConsoleState
        fs::write(&log, "11/21 23:26:01 Loaded hero hero_atlas\n").unwrap();
        let mut offset = 0u64;
        let mut console = ConsoleState::default();
        let mut read_buf = Vec::new();
        let mut text_buf = String::new();
        assert!(read_console_tail_or_append(
            &log,
            &mut offset,
            &mut console,
            &mut read_buf,
            &mut text_buf,
            1000
        ));
        assert!(offset > 0);
        assert_eq!(console.hero, Some("Abrams"));

        fs::remove_file(&log).unwrap();
        assert!(read_console_tail_or_append(
            &log,
            &mut offset,
            &mut console,
            &mut read_buf,
            &mut text_buf,
            1001
        ));
        assert_eq!(offset, 0);
        assert_eq!(console.hero, None);

        // 2. Усечение до 0 байт при перезапуске игры сбрасывает offset
        fs::write(&log, "11/21 23:26:01 Loaded hero hero_haze\n").unwrap();
        assert!(read_console_tail_or_append(
            &log,
            &mut offset,
            &mut console,
            &mut read_buf,
            &mut text_buf,
            1002
        ));
        assert!(offset > 0);
        fs::write(&log, b"").unwrap();
        assert!(read_console_tail_or_append(
            &log,
            &mut offset,
            &mut console,
            &mut read_buf,
            &mut text_buf,
            1003
        ));
        assert_eq!(offset, 0);

        // 3. Строка длиннее CHUNK_WINDOW (2 МБ) без '\n' не приводит к бесконечному зависанию offset
        let giant = "X".repeat(2 * 1024 * 1024 + 512);
        fs::write(&log, giant.as_bytes()).unwrap();
        assert!(read_console_tail_or_append(
            &log,
            &mut offset,
            &mut console,
            &mut read_buf,
            &mut text_buf,
            1004
        ));
        assert!(offset > 2 * 1024 * 1024);

        // 4. Повторная загрузка той же карты dl_midtown очищает bot_entity_ids прошлого матча
        let mut match_st = ConsoleState::default();
        parse_console_line(&mut match_st, "Loading map \"dl_midtown\"", 100);
        parse_console_line(&mut match_st, "Created bot 298/hero_inferno", 101);
        parse_console_line(&mut match_st, "Loading map \"dl_midtown\"", 200);
        parse_console_line(&mut match_st, "[Server] Loaded hero 298/hero_kelvin", 201);
        assert_eq!(match_st.hero, Some("Kelvin"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_build_activity_with_preset_option() {
        let mut st = ConsoleState::default();
        st.mode = GameMode::StandardMatch;
        st.hero = Some("Haze");

        let mut cfg = DiscordRpcConfig::default();
        cfg.show_preset = true;
        let with_hero =
            build_activity_json_with_preset(&st, &cfg, Some("Player"), Some("Мой конфиг"));
        assert_eq!(
            with_hero["state"].as_str(),
            Some("Haze • Player • Мой конфиг")
        );

        let no_hero = ConsoleState::default();
        let mut cfg_preset_only = DiscordRpcConfig::default();
        cfg_preset_only.show_hero = false;
        cfg_preset_only.show_steam_name = false;
        cfg_preset_only.show_preset = true;
        let preset_only =
            build_activity_json_with_preset(&no_hero, &cfg_preset_only, None, Some("Турнир"));
        assert_eq!(preset_only["state"].as_str(), Some("Пресет: Турнир"));

        cfg_preset_only.show_preset = false;
        let hidden =
            build_activity_json_with_preset(&no_hero, &cfg_preset_only, None, Some("Турнир"));
        assert!(hidden.get("state").is_none());
    }
}
