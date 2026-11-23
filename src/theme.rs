//! Дизайн-система ShaurMeaw в эстетике Deadlock (Оккультный нуар + Арт-деко 1930-х).
//!
//! Содержит цветовые токены, типографику (Editorial Serif / Sans / Monospace),
//! векторные иконки (stroke 1.5px), кастомные компоненты Art Deco
//! и кэшируемую подсветку синтаксиса VDF / CFG.

use std::sync::Arc;

use eframe::egui::cache::{ComputerMut, FrameCache};
use eframe::egui::emath::easing;
use eframe::egui::epaint::{CornerRadius, PathShape, PathStroke, RectShape, StrokeKind};
use eframe::egui::{
    self, pos2, vec2, Align, Color32, CursorIcon, FontData, FontDefinitions, FontFamily, FontId,
    Margin, Mesh, Painter, Rect, Response, RichText, Sense, Shape, Stroke, TextFormat, Ui,
};

use crate::config_editor::{BraceCheck, TextMatch};
use crate::settings::Theme;
use crate::tools;

// ============================================================ Дизайн-токены палитры Deadlock

/// Основной глубокий угольно-аспидный фон (Deep Noir Slate).
pub const BG_NOIR: Color32 = Color32::from_rgb(0x11, 0x13, 0x15);
/// Вторичный глубокий сланец для боковой и нижней панелей.
pub const BG_SLATE: Color32 = Color32::from_rgb(0x14, 0x16, 0x19);
/// Фон манускрипта / технического архива кода.
pub const BG_MANUSCRIPT: Color32 = Color32::from_rgb(0x0E, 0x10, 0x12);

/// Поверхность карточек-досье и панелей (Parchment Dark / Obsidian).
pub const SURFACE_CARD: Color32 = Color32::from_rgb(0x1B, 0x1E, 0x22);
/// Приподнятая поверхность при наведении / вводе.
pub const SURFACE_ELEVATED: Color32 = Color32::from_rgb(0x23, 0x27, 0x2C);

/// Главный акцент — Оккультный Янтарь / Состаренная Латунь (Amber Gold).
pub const BRASS: Color32 = Color32::from_rgb(0xD4, 0xA3, 0x59);
/// Светлая латунь для заголовков и ключей VDF.
pub const BRASS_LIGHT: Color32 = Color32::from_rgb(0xE5, 0xB8, 0x69);
/// Верхний цвет градиента латунной кнопки.
pub const BRASS_GRAD_TOP: Color32 = Color32::from_rgb(0xE2, 0xB3, 0x66);
/// Нижний цвет градиента латунной кнопки.
pub const BRASS_GRAD_BOT: Color32 = Color32::from_rgb(0xC8, 0x96, 0x4B);
/// Акцентная латунная линия 1px (`rgba(212, 163, 89, 0.35)`).
pub const BRASS_BORDER: Color32 = Color32::from_rgba_premultiplied(74, 57, 31, 89);

/// Вторичный акцент — Эктоплазма / Души (Ethereal Teal) для активных статусов.
pub const SOUL_TEAL: Color32 = Color32::from_rgb(0x3E, 0xD0, 0xA5);
/// Эктоплазма для значений в кавычках редактора кода.
pub const SOUL_VALUE: Color32 = Color32::from_rgb(0x48, 0xD1, 0xB5);

/// Тревожный / критический акцент (Crimson Occult).
pub const CRIMSON: Color32 = Color32::from_rgb(0xC9, 0x4A, 0x4A);

/// Основной тёплый кремовый текст (Aged Cream / Bone).
pub const TEXT_CREAM: Color32 = Color32::from_rgb(0xED, 0xE6, 0xD6);
/// Второстепенный серо-бежевый текст.
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x8E, 0x8B, 0x82);
/// Приглушённый оливково-серый для комментариев VDF.
pub const VDF_COMMENT: Color32 = Color32::from_rgb(0x68, 0x6D, 0x66);

/// Обычные тонкие границы и разделители.
pub const BORDER_SUBTLE: Color32 = Color32::from_rgb(0x2E, 0x33, 0x38);

/// Стандартная длительность плавных переходов (180 ms ease-out).
pub const ANIM_DURATION_SEC: f32 = 0.18;

/// Активная палитра с учётом выбранного оттенка темы в настройках.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub accent: Color32,
    pub accent_light: Color32,
    pub accent_border: Color32,
    pub grad_top: Color32,
    pub grad_bot: Color32,
    pub selection: Color32,
}

impl Palette {
    /// Собирает палитру, сохраняя точные латунные токены Deadlock для базовой темы.
    pub fn from_theme(theme: Theme) -> Self {
        let [sr, sg, sb] = theme.selection_rgb();
        let selection = Color32::from_rgb(sr, sg, sb);

        if theme == Theme::ShaurMeaw {
            return Self {
                accent: BRASS,
                accent_light: BRASS_LIGHT,
                accent_border: BRASS_BORDER,
                grad_top: BRASS_GRAD_TOP,
                grad_bot: BRASS_GRAD_BOT,
                selection,
            };
        }

        let [ar, ag, ab] = theme.accent_rgb();
        let accent = Color32::from_rgb(ar, ag, ab);
        let accent_light = Color32::from_rgb(
            ar.saturating_add(18),
            ag.saturating_add(18),
            ab.saturating_add(14),
        );
        let grad_bot = Color32::from_rgb(
            ar.saturating_sub(14),
            ag.saturating_sub(14),
            ab.saturating_sub(14),
        );
        Self {
            accent,
            accent_light,
            accent_border: Color32::from_rgba_unmultiplied(ar, ag, ab, 89),
            grad_top: accent_light,
            grad_bot,
            selection,
        }
    }
}

// ============================================================ Типографика и глобальный стиль

/// Семейство шрифта с засечками (Editorial Serif / Art Deco заголовки).
pub fn serif_family() -> FontFamily {
    FontFamily::Name(Arc::from("serif"))
}

/// Заголовок или подпись в стиле 1930-х: UPPERCASE, Serif, увеличенный трекинг `~0.085em`.
pub fn serif_heading(text: &str, size: f32, color: Color32) -> RichText {
    RichText::new(text.to_uppercase())
        .family(serif_family())
        .size(size)
        .color(color)
        .extra_letter_spacing((size * 0.085).max(0.8))
}

/// Моноширинный текст для хешей, путей и размеров файлов.
pub fn mono_text(text: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(text).monospace().size(size).color(color)
}

/// Загружает системные шрифты Windows (Editorial Serif, Sans-serif, Monospace) параллельно без раздувания `.exe`.
pub fn setup_fonts_and_style(ctx: &egui::Context, theme: Theme) {
    let mut fonts = FontDefinitions::empty();
    let fonts_dir = tools::windows_root().join("Fonts");

    let (prop_bytes, serif_bytes, mono_bytes) = std::thread::scope(|scope| {
        let prop_handle = scope.spawn(|| -> Option<Vec<u8>> {
            for file in [
                "segoeui.ttf",
                "arial.ttf",
                "tahoma.ttf",
                "calibri.ttf",
                "verdana.ttf",
                "micross.ttf",
                "times.ttf",
                "cour.ttf",
                "lucon.ttf",
            ] {
                if let Ok(bytes) = crate::fs_safe::read_bytes_with_retry(&fonts_dir.join(file)) {
                    return Some(bytes);
                }
            }
            if let Ok(entries) = std::fs::read_dir(&fonts_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    let is_ttf = p
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case("ttf"));
                    if !is_ttf {
                        continue;
                    }
                    let Ok(meta) = entry.metadata() else {
                        continue;
                    };
                    if !(16 * 1024..=4 * 1024 * 1024).contains(&meta.len()) {
                        continue;
                    }
                    if let Ok(bytes) = crate::fs_safe::read_bytes_with_retry(&p) {
                        return Some(bytes);
                    }
                }
            }
            None
        });

        let serif_handle = scope.spawn(|| -> Option<Vec<u8>> {
            for file in [
                "constanb.ttf",
                "palab.ttf",
                "georgiab.ttf",
                "cambriab.ttf",
                "timesbd.ttf",
                "constan.ttf",
                "georgia.ttf",
            ] {
                if let Ok(bytes) = crate::fs_safe::read_bytes_with_retry(&fonts_dir.join(file)) {
                    return Some(bytes);
                }
            }
            None
        });

        let mono_handle = scope.spawn(|| -> Option<Vec<u8>> {
            for file in ["consola.ttf", "lucon.ttf", "cour.ttf"] {
                if let Ok(bytes) = crate::fs_safe::read_bytes_with_retry(&fonts_dir.join(file)) {
                    return Some(bytes);
                }
            }
            None
        });

        (
            prop_handle.join().ok().flatten(),
            serif_handle.join().ok().flatten(),
            mono_handle.join().ok().flatten(),
        )
    });

    // 1. Гуманистический гротеск (основной интерфейс)
    let mut has_proportional = false;
    if let Some(bytes) = prop_bytes {
        fonts
            .font_data
            .insert("win_proportional".to_owned(), Arc::new(FontData::from_owned(bytes)));
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .push("win_proportional".to_owned());
        has_proportional = true;
    }

    // 2. Editorial Serif (заголовки, названия пресетов, печать)
    let mut has_serif = false;
    if let Some(bytes) = serif_bytes {
        fonts
            .font_data
            .insert("win_serif".to_owned(), Arc::new(FontData::from_owned(bytes)));
        fonts
            .families
            .entry(serif_family())
            .or_default()
            .push("win_serif".to_owned());
        has_serif = true;
    }
    if !has_serif && has_proportional {
        fonts
            .families
            .entry(serif_family())
            .or_default()
            .push("win_proportional".to_owned());
    } else if has_serif && has_proportional {
        fonts
            .families
            .entry(serif_family())
            .or_default()
            .push("win_proportional".to_owned());
    }

    // 3. Моноширинный шрифт (код gameinfo.gi, пути, SHA-256)
    if let Some(bytes) = mono_bytes {
        fonts
            .font_data
            .insert("win_monospace".to_owned(), Arc::new(FontData::from_owned(bytes)));
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .push("win_monospace".to_owned());
    }
    if has_proportional {
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .push("win_proportional".to_owned());
    }

    if fonts.font_data.is_empty() {
        fonts = FontDefinitions::default();
        if let Some(prop) = fonts.families.get(&FontFamily::Proportional).cloned() {
            fonts
                .families
                .entry(serif_family())
                .or_default()
                .extend(prop);
        }
    } else {
        let fallback_key = fonts
            .families
            .get(&FontFamily::Proportional)
            .and_then(|v| v.first())
            .cloned()
            .or_else(|| fonts.font_data.keys().next().cloned());
        if let Some(fk) = fallback_key {
            if fonts
                .families
                .get(&FontFamily::Proportional)
                .is_none_or(Vec::is_empty)
            {
                fonts
                    .families
                    .entry(FontFamily::Proportional)
                    .or_default()
                    .push(fk.clone());
            }
            if fonts
                .families
                .get(&serif_family())
                .is_none_or(Vec::is_empty)
            {
                fonts
                    .families
                    .entry(serif_family())
                    .or_default()
                    .push(fk.clone());
            }
            if fonts
                .families
                .get(&FontFamily::Monospace)
                .is_none_or(Vec::is_empty)
            {
                fonts
                    .families
                    .entry(FontFamily::Monospace)
                    .or_default()
                    .push(fk);
            }
        }
    }

    ctx.set_fonts(fonts);
    apply_theme(ctx, theme);

    ctx.style_mut(|style| {
        style.animation_time = ANIM_DURATION_SEC;
        style.spacing.item_spacing = vec2(8.0, 7.0);
        style.spacing.button_padding = vec2(11.0, 6.0);
        style.spacing.interact_size = vec2(36.0, 22.0);
        style.spacing.slider_width = 190.0;
        style.spacing.scroll = egui::style::ScrollStyle {
            bar_width: 6.0,
            handle_min_length: 24.0,
            floating: false,
            ..Default::default()
        };
        for (text_style, font) in style.text_styles.iter_mut() {
            match text_style {
                egui::TextStyle::Body | egui::TextStyle::Button => font.size = 14.5,
                egui::TextStyle::Monospace => font.size = 13.0,
                egui::TextStyle::Small => font.size = 12.0,
                egui::TextStyle::Heading => font.size = 18.0,
                _ => {}
            }
        }
    });
}

/// Применяет палитру оккультного нуара ко всем стандартным виджетам `egui`.
pub fn apply_theme(ctx: &egui::Context, theme: Theme) {
    let pal = Palette::from_theme(theme);
    let mut visuals = egui::Visuals::dark();

    visuals.dark_mode = true;
    visuals.panel_fill = BG_NOIR;
    visuals.window_fill = SURFACE_CARD;
    visuals.extreme_bg_color = BG_MANUSCRIPT;
    visuals.faint_bg_color = SURFACE_CARD;
    visuals.code_bg_color = BG_MANUSCRIPT;

    visuals.override_text_color = Some(TEXT_CREAM);
    visuals.warn_fg_color = pal.accent_light;
    visuals.error_fg_color = CRIMSON;
    visuals.hyperlink_color = pal.accent_light;

    visuals.window_corner_radius = CornerRadius::same(2);
    visuals.menu_corner_radius = CornerRadius::same(2);
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER_SUBTLE);
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(160),
    };
    visuals.popup_shadow = visuals.window_shadow;

    visuals.selection.bg_fill = pal.selection;
    visuals.selection.stroke = Stroke::new(1.0_f32, pal.accent);

    visuals.text_cursor.stroke = Stroke::new(2.0_f32, pal.accent_light);
    visuals.slider_trailing_fill = true;
    visuals.handle_shape = egui::style::HandleShape::Rect { aspect_ratio: 0.45 };
    visuals.collapsing_header_frame = true;
    visuals.indent_has_left_vline = true;

    visuals.widgets.noninteractive.bg_fill = SURFACE_CARD;
    visuals.widgets.noninteractive.weak_bg_fill = SURFACE_CARD;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER_SUBTLE);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_CREAM);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(2);

    visuals.widgets.inactive.bg_fill = SURFACE_CARD;
    visuals.widgets.inactive.weak_bg_fill = SURFACE_CARD;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER_SUBTLE);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.2_f32, TEXT_CREAM);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(2);
    visuals.widgets.inactive.expansion = 0.0;

    visuals.widgets.hovered.bg_fill = SURFACE_ELEVATED;
    visuals.widgets.hovered.weak_bg_fill = SURFACE_ELEVATED;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, pal.accent);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.3_f32, TEXT_CREAM);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(2);
    visuals.widgets.hovered.expansion = 0.0;

    visuals.widgets.active.bg_fill = SURFACE_ELEVATED;
    visuals.widgets.active.weak_bg_fill = SURFACE_ELEVATED;
    visuals.widgets.active.bg_stroke = Stroke::new(1.2_f32, pal.accent_light);
    visuals.widgets.active.fg_stroke = Stroke::new(1.5_f32, pal.accent_light);
    visuals.widgets.active.corner_radius = CornerRadius::same(2);
    visuals.widgets.active.expansion = 0.0;

    visuals.widgets.open.bg_fill = SURFACE_ELEVATED;
    visuals.widgets.open.weak_bg_fill = SURFACE_ELEVATED;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, pal.accent);
    visuals.widgets.open.fg_stroke = Stroke::new(1.2_f32, pal.accent_light);
    visuals.widgets.open.corner_radius = CornerRadius::same(2);

    ctx.set_visuals(visuals);
}

// ============================================================ Векторные линейные иконки (stroke 1.5px)

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Launch,
    Bolt,
    Tools,
    Settings,
    Play,
    Save,
    Import,
    Globe,
    Edit,
    Trash,
    Refresh,
    Copy,
    Folder,
    Plus,
    Restore,
    Power,
    Check,
    Alert,
    External,
    Archive,
    VolumeOn,
    VolumeOff,
    MoreDots,
    Timer,
    Drop,
}

/// Рисует чёткую векторную линейную иконку в стиле гравюры (толщина линий ~1.5px).
pub fn paint_icon(painter: &Painter, rect: Rect, icon: Icon, color: Color32, width: f32) {
    let c = rect.center();
    let r = (rect.width().min(rect.height()) * 0.42).max(3.0);
    let st = Stroke::new(width, color);
    let p_st = PathStroke::new(width, color);

    match icon {
        Icon::Launch => {
            // Сопла/крылья оккультного снаряда-стрелы вверх-вправо
            let tip = pos2(c.x + r * 0.85, c.y - r * 0.85);
            let tail = pos2(c.x - r * 0.75, c.y + r * 0.75);
            painter.line_segment([tail, tip], st);
            painter.line_segment([tip, pos2(c.x + r * 0.1, c.y - r * 0.8)], st);
            painter.line_segment([tip, pos2(c.x + r * 0.8, c.y - r * 0.1)], st);
            painter.circle_stroke(pos2(c.x - r * 0.15, c.y + r * 0.15), r * 0.34, st);
        }
        Icon::Bolt => {
            // Векторная молния ускорения (центр оптимизации)
            let pts = vec![
                pos2(c.x + r * 0.14, c.y - r * 0.92),
                pos2(c.x - r * 0.68, c.y + r * 0.08),
                pos2(c.x - r * 0.06, c.y + r * 0.08),
                pos2(c.x - r * 0.22, c.y + r * 0.92),
                pos2(c.x + r * 0.68, c.y - r * 0.12),
                pos2(c.x + r * 0.06, c.y - r * 0.12),
            ];
            painter.add(PathShape::closed_line(pts, p_st));
        }
        Icon::Archive => {
            // Пергаментный лист со скошенным правым верхним углом
            let x0 = c.x - r * 0.68;
            let x1 = c.x + r * 0.68;
            let y0 = c.y - r * 0.86;
            let y1 = c.y + r * 0.86;
            let fold = r * 0.38;
            painter.add(PathShape::closed_line(
                vec![
                    pos2(x0, y0),
                    pos2(x1 - fold, y0),
                    pos2(x1, y0 + fold),
                    pos2(x1, y1),
                    pos2(x0, y1),
                ],
                p_st,
            ));
            painter.line_segment([pos2(x0 + r * 0.3, c.y - r * 0.15), pos2(x1 - r * 0.3, c.y - r * 0.15)], st);
            painter.line_segment([pos2(x0 + r * 0.3, c.y + r * 0.30), pos2(x1 - r * 0.3, c.y + r * 0.30)], st);
        }
        Icon::Tools => {
            // Алхимический ромб с перекрестием
            let pts = vec![
                pos2(c.x, c.y - r * 0.88),
                pos2(c.x + r * 0.88, c.y),
                pos2(c.x, c.y + r * 0.88),
                pos2(c.x - r * 0.88, c.y),
            ];
            painter.add(PathShape::closed_line(pts, p_st));
            painter.circle_stroke(c, r * 0.32, st);
        }
        Icon::Folder => {
            // Архивная папка с язычком
            let x0 = c.x - r * 0.85;
            let x1 = c.x + r * 0.85;
            let y0 = c.y - r * 0.65;
            let y1 = c.y + r * 0.68;
            painter.add(PathShape::closed_line(
                vec![
                    pos2(x0, y0),
                    pos2(c.x - r * 0.15, y0),
                    pos2(c.x + r * 0.05, y0 + r * 0.25),
                    pos2(x1, y0 + r * 0.25),
                    pos2(x1, y1),
                    pos2(x0, y1),
                ],
                p_st,
            ));
        }
        Icon::Play => {
            // Строгий треугольник запуска
            let pts = vec![
                pos2(c.x - r * 0.48, c.y - r * 0.72),
                pos2(c.x + r * 0.72, c.y),
                pos2(c.x - r * 0.48, c.y + r * 0.72),
            ];
            painter.add(PathShape::convex_polygon(pts, color, Stroke::NONE));
        }
        Icon::Save => {
            // Дискета-печать архива со скосом угла
            let x0 = c.x - r * 0.75;
            let x1 = c.x + r * 0.75;
            let y0 = c.y - r * 0.75;
            let y1 = c.y + r * 0.75;
            let cut = r * 0.34;
            painter.add(PathShape::closed_line(
                vec![
                    pos2(x0, y0),
                    pos2(x1 - cut, y0),
                    pos2(x1, y0 + cut),
                    pos2(x1, y1),
                    pos2(x0, y1),
                ],
                p_st,
            ));
            let inner = Rect::from_min_max(pos2(x0 + r * 0.3, c.y + r * 0.1), pos2(x1 - r * 0.3, y1));
            painter.rect_stroke(inner, 0.0, st, StrokeKind::Inside);
        }
        Icon::Import => {
            // Стрелка вниз в поддон
            painter.line_segment([pos2(c.x, c.y - r * 0.8), pos2(c.x, c.y + r * 0.35)], st);
            painter.line_segment(
                [pos2(c.x - r * 0.42, c.y - r * 0.05), pos2(c.x, c.y + r * 0.35)],
                st,
            );
            painter.line_segment(
                [pos2(c.x + r * 0.42, c.y - r * 0.05), pos2(c.x, c.y + r * 0.35)],
                st,
            );
            painter.line_segment(
                [pos2(c.x - r * 0.75, c.y + r * 0.72), pos2(c.x + r * 0.75, c.y + r * 0.72)],
                st,
            );
        }
        Icon::Globe => {
            painter.circle_stroke(c, r * 0.78, st);
            painter.line_segment([pos2(c.x - r * 0.78, c.y), pos2(c.x + r * 0.78, c.y)], st);
            painter.line_segment([pos2(c.x, c.y - r * 0.78), pos2(c.x, c.y + r * 0.78)], st);
        }
        Icon::Edit => {
            painter.line_segment(
                [pos2(c.x - r * 0.72, c.y + r * 0.72), pos2(c.x + r * 0.68, c.y - r * 0.68)],
                st,
            );
            painter.line_segment(
                [pos2(c.x - r * 0.72, c.y + r * 0.72), pos2(c.x - r * 0.25, c.y + r * 0.62)],
                st,
            );
            painter.line_segment(
                [pos2(c.x + r * 0.35, c.y - r * 0.35), pos2(c.x + r * 0.72, c.y + r * 0.02)],
                st,
            );
        }
        Icon::Trash => {
            let lid_y = c.y - r * 0.52;
            painter.line_segment([pos2(c.x - r * 0.75, lid_y), pos2(c.x + r * 0.75, lid_y)], st);
            painter.line_segment(
                [pos2(c.x - r * 0.28, lid_y - r * 0.26), pos2(c.x + r * 0.28, lid_y - r * 0.26)],
                st,
            );
            let body = Rect::from_min_max(
                pos2(c.x - r * 0.55, lid_y),
                pos2(c.x + r * 0.55, c.y + r * 0.78),
            );
            painter.rect_stroke(body, 0.5, st, StrokeKind::Inside);
        }
        Icon::Refresh => {
            // Две встречные дуги/стрелки по кругу
            let d = r * 0.72;
            painter.line_segment([pos2(c.x - d, c.y - d * 0.25), pos2(c.x + d * 0.55, c.y - d * 0.65)], st);
            painter.line_segment([pos2(c.x + d * 0.55, c.y - d * 0.65), pos2(c.x + d * 0.15, c.y - d * 0.15)], st);
            painter.line_segment([pos2(c.x + d, c.y + d * 0.25), pos2(c.x - d * 0.55, c.y + d * 0.65)], st);
            painter.line_segment([pos2(c.x - d * 0.55, c.y + d * 0.65), pos2(c.x - d * 0.15, c.y + d * 0.15)], st);
        }
        Icon::Copy => {
            let b1 = Rect::from_min_max(pos2(c.x - r * 0.7, c.y - r * 0.7), pos2(c.x + r * 0.35, c.y + r * 0.4));
            let b2 = Rect::from_min_max(pos2(c.x - r * 0.25, c.y - r * 0.3), pos2(c.x + r * 0.75, c.y + r * 0.78));
            painter.rect_stroke(b1, 0.5, st, StrokeKind::Inside);
            painter.rect_filled(b2, 0.5, BG_NOIR);
            painter.rect_stroke(b2, 0.5, st, StrokeKind::Inside);
        }
        Icon::Plus => {
            painter.line_segment([pos2(c.x - r * 0.72, c.y), pos2(c.x + r * 0.72, c.y)], st);
            painter.line_segment([pos2(c.x, c.y - r * 0.72), pos2(c.x, c.y + r * 0.72)], st);
        }
        Icon::Restore => {
            // Стрелка возврата влево-вверх
            painter.line_segment([pos2(c.x - r * 0.72, c.y - r * 0.2), pos2(c.x + r * 0.45, c.y - r * 0.2)], st);
            painter.line_segment([pos2(c.x + r * 0.45, c.y - r * 0.2), pos2(c.x + r * 0.72, c.y + r * 0.18)], st);
            painter.line_segment([pos2(c.x + r * 0.72, c.y + r * 0.18), pos2(c.x + r * 0.35, c.y + r * 0.65)], st);
            painter.line_segment([pos2(c.x - r * 0.72, c.y - r * 0.2), pos2(c.x - r * 0.32, c.y - r * 0.62)], st);
            painter.line_segment([pos2(c.x - r * 0.72, c.y - r * 0.2), pos2(c.x - r * 0.32, c.y + r * 0.22)], st);
        }
        Icon::Power => {
            painter.circle_stroke(pos2(c.x, c.y + r * 0.08), r * 0.66, st);
            painter.line_segment([pos2(c.x, c.y - r * 0.85), pos2(c.x, c.y + r * 0.05)], st);
        }
        Icon::Check => {
            painter.line_segment(
                [pos2(c.x - r * 0.72, c.y + r * 0.06), pos2(c.x - r * 0.16, c.y + r * 0.60)],
                st,
            );
            painter.line_segment(
                [pos2(c.x - r * 0.16, c.y + r * 0.60), pos2(c.x + r * 0.78, c.y - r * 0.56)],
                st,
            );
        }
        Icon::Alert => {
            let pts = vec![
                pos2(c.x, c.y - r * 0.85),
                pos2(c.x + r * 0.82, c.y + r * 0.72),
                pos2(c.x - r * 0.82, c.y + r * 0.72),
            ];
            painter.add(PathShape::closed_line(pts, p_st));
            painter.line_segment([pos2(c.x, c.y - r * 0.25), pos2(c.x, c.y + r * 0.22)], st);
            painter.circle_filled(pos2(c.x, c.y + r * 0.46), width * 0.75, color);
        }
        Icon::External => {
            let tr = pos2(c.x + r * 0.78, c.y - r * 0.78);
            painter.line_segment([pos2(c.x - r * 0.55, c.y + r * 0.55), tr], st);
            painter.line_segment([tr, pos2(c.x + r * 0.10, c.y - r * 0.78)], st);
            painter.line_segment([tr, pos2(c.x + r * 0.78, c.y - r * 0.10)], st);
        }
        Icon::Settings => {
            // Геометрическая шестерёнка Art Deco (8 зубцов + двойной контур)
            painter.circle_stroke(c, r * 0.30, st);
            painter.circle_stroke(c, r * 0.60, st);
            for i in 0..8 {
                let ang = (i as f32) * std::f32::consts::FRAC_PI_4;
                let (sin_a, cos_a) = ang.sin_cos();
                painter.line_segment(
                    [
                        pos2(c.x + r * 0.62 * cos_a, c.y + r * 0.62 * sin_a),
                        pos2(c.x + r * 0.90 * cos_a, c.y + r * 0.90 * sin_a),
                    ],
                    st,
                );
            }
        }
        Icon::VolumeOn => {
            // Динамик слева и звуковая дуга справа
            let poly = vec![
                pos2(c.x - r * 0.72, c.y - r * 0.28),
                pos2(c.x - r * 0.36, c.y - r * 0.28),
                pos2(c.x + r * 0.08, c.y - r * 0.66),
                pos2(c.x + r * 0.08, c.y + r * 0.66),
                pos2(c.x - r * 0.36, c.y + r * 0.28),
                pos2(c.x - r * 0.72, c.y + r * 0.28),
            ];
            painter.add(PathShape::closed_line(poly, p_st.clone()));
            let wave1 = vec![
                pos2(c.x + r * 0.34, c.y - r * 0.34),
                pos2(c.x + r * 0.52, c.y),
                pos2(c.x + r * 0.34, c.y + r * 0.34),
            ];
            painter.add(PathShape::line(wave1, p_st.clone()));
            let wave2 = vec![
                pos2(c.x + r * 0.56, c.y - r * 0.60),
                pos2(c.x + r * 0.80, c.y),
                pos2(c.x + r * 0.56, c.y + r * 0.60),
            ];
            painter.add(PathShape::line(wave2, p_st));
        }
        Icon::VolumeOff => {
            let poly = vec![
                pos2(c.x - r * 0.72, c.y - r * 0.28),
                pos2(c.x - r * 0.36, c.y - r * 0.28),
                pos2(c.x + r * 0.06, c.y - r * 0.66),
                pos2(c.x + r * 0.06, c.y + r * 0.66),
                pos2(c.x - r * 0.36, c.y + r * 0.28),
                pos2(c.x - r * 0.72, c.y + r * 0.28),
            ];
            painter.add(PathShape::closed_line(poly, p_st));
            painter.line_segment(
                [pos2(c.x + r * 0.30, c.y - r * 0.32), pos2(c.x + r * 0.82, c.y + r * 0.32)],
                st,
            );
            painter.line_segment(
                [pos2(c.x + r * 0.82, c.y - r * 0.32), pos2(c.x + r * 0.30, c.y + r * 0.32)],
                st,
            );
        }
        Icon::MoreDots => {
            let rad = (r * 0.21).max(1.5);
            painter.circle_filled(pos2(c.x - r * 0.55, c.y), rad, color);
            painter.circle_filled(pos2(c.x, c.y), rad, color);
            painter.circle_filled(pos2(c.x + r * 0.55, c.y), rad, color);
        }
        Icon::Timer => {
            let dial = pos2(c.x, c.y + r * 0.12);
            painter.circle_stroke(dial, r * 0.66, st);
            painter.line_segment([pos2(c.x - r * 0.24, c.y - r * 0.80), pos2(c.x + r * 0.24, c.y - r * 0.80)], st);
            painter.line_segment([dial, pos2(dial.x, dial.y - r * 0.38)], st);
            painter.line_segment([dial, pos2(dial.x + r * 0.30, dial.y + r * 0.14)], st);
        }
        Icon::Drop => {
            painter.line_segment([pos2(c.x, c.y - r * 0.72), pos2(c.x, c.y + r * 0.25)], st);
            painter.line_segment([pos2(c.x - r * 0.38, c.y - r * 0.12), pos2(c.x, c.y + r * 0.25)], st);
            painter.line_segment([pos2(c.x + r * 0.38, c.y - r * 0.12), pos2(c.x, c.y + r * 0.25)], st);
            let tray = vec![
                pos2(c.x - r * 0.78, c.y + r * 0.25),
                pos2(c.x - r * 0.78, c.y + r * 0.68),
                pos2(c.x + r * 0.78, c.y + r * 0.68),
                pos2(c.x + r * 0.78, c.y + r * 0.25),
            ];
            painter.add(PathShape::line(tray, p_st));
        }
    }
}

// ============================================================ Геометрия и компоненты Art Deco

/// Рисует декоративные L-уголки Art Deco по 4 углам прямоугольника.
pub fn paint_corner_brackets(painter: &Painter, rect: Rect, len: f32, stroke: Stroke) {
    let l = rect.left();
    let r = rect.right();
    let t = rect.top();
    let b = rect.bottom();
    let k = len.min(rect.width() * 0.25).min(rect.height() * 0.25);

    let segs = [
        [pos2(l, t + k), pos2(l, t)],
        [pos2(l, t), pos2(l + k, t)],
        [pos2(r - k, t), pos2(r, t)],
        [pos2(r, t), pos2(r, t + k)],
        [pos2(l, b - k), pos2(l, b)],
        [pos2(l, b), pos2(l + k, b)],
        [pos2(r - k, b), pos2(r, b)],
        [pos2(r, b), pos2(r, b - k)],
    ];
    for seg in segs {
        painter.line_segment(seg, stroke);
    }
}

/// Рисует восьмиугольник со скошенными углами и вертикальным градиентом через Mesh.
pub fn paint_chamfered_gradient_rect(
    painter: &Painter,
    rect: Rect,
    chamfer: f32,
    top_col: Color32,
    bot_col: Color32,
    outer_stroke: Stroke,
    inner_stroke: Option<Stroke>,
) {
    let c = chamfer.min(rect.width() * 0.22).min(rect.height() * 0.42);
    let h = rect.height().max(1.0);
    let t_top = (c / h).clamp(0.0, 1.0);
    let t_bot = (1.0 - t_top).clamp(0.0, 1.0);
    let col_upper = top_col.lerp_to_gamma(bot_col, t_top);
    let col_lower = top_col.lerp_to_gamma(bot_col, t_bot);

    let l = rect.left();
    let r = rect.right();
    let t = rect.top();
    let b = rect.bottom();

    let pts = [
        pos2(l + c, t),
        pos2(r - c, t),
        pos2(r, t + c),
        pos2(r, b - c),
        pos2(r - c, b),
        pos2(l + c, b),
        pos2(l, b - c),
        pos2(l, t + c),
    ];
    let cols = [
        top_col, top_col, col_upper, col_lower, bot_col, bot_col, col_lower, col_upper,
    ];

    let mut mesh = Mesh::default();
    mesh.reserve_vertices(8);
    mesh.reserve_triangles(6);
    for (p, col) in pts.into_iter().zip(cols) {
        mesh.colored_vertex(p, col);
    }
    for i in 1u32..=6 {
        mesh.add_triangle(0, i, i + 1);
    }
    painter.add(Shape::mesh(mesh));

    if outer_stroke.width > 0.0 {
        painter.add(PathShape::closed_line(
            pts.to_vec(),
            PathStroke::new(outer_stroke.width, outer_stroke.color),
        ));
    }
    if let Some(in_st) = inner_stroke {
        let ir = rect.shrink(2.5);
        let ic = (c - 1.0).max(1.5);
        let ipts = vec![
            pos2(ir.left() + ic, ir.top()),
            pos2(ir.right() - ic, ir.top()),
            pos2(ir.right(), ir.top() + ic),
            pos2(ir.right(), ir.bottom() - ic),
            pos2(ir.right() - ic, ir.bottom()),
            pos2(ir.left() + ic, ir.bottom()),
            pos2(ir.left(), ir.bottom() - ic),
            pos2(ir.left(), ir.top() + ic),
        ];
        painter.add(PathShape::closed_line(
            ipts,
            PathStroke::new(in_st.width, in_st.color),
        ));
    }
}

/// Тонкий геометрический разделитель Art Deco с центральным ромбом (`──◆──`).
pub fn paint_deco_divider(ui: &mut Ui, line_color: Color32, diamond_color: Color32) {
    let w = ui.available_width().max(24.0);
    let (rect, _) = ui.allocate_exact_size(vec2(w, 10.0), Sense::hover());
    let p = ui.painter();
    let y = rect.center().y;
    let cx = rect.center().x;

    p.line_segment([pos2(rect.left(), y), pos2(cx - 7.0, y)], Stroke::new(1.0_f32, line_color));
    p.line_segment([pos2(cx + 7.0, y), pos2(rect.right(), y)], Stroke::new(1.0_f32, line_color));

    let d = 3.5;
    let diamond = vec![pos2(cx, y - d), pos2(cx + d, y), pos2(cx, y + d), pos2(cx - d, y)];
    p.add(PathShape::convex_polygon(diamond, diamond_color, Stroke::NONE));
}

/// Компактная печать-логотип для узкого сайдбара (180px): иконка 32×32 + `SHAURMEAW` / `DEADLOCK`.
pub fn paint_cursed_apple_seal_compact(
    ui: &mut Ui,
    avatar_tex: egui::TextureId,
    pal: &Palette,
    glow_t: f32,
) -> Response {
    let glow = glow_t.clamp(0.0, 1.0);
    let avail_w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(avail_w, 46.0), Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }

    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let img_rect = Rect::from_min_size(pos2(rect.left() + 4.0, rect.center().y - 16.0), vec2(32.0, 32.0));
        if glow > 0.01 {
            p.rect_filled(img_rect.expand(5.0), 8.0, SOUL_TEAL.gamma_multiply(0.22 * glow));
        }
        p.image(
            avatar_tex,
            img_rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );

        let title_col = pal.accent_light.lerp_to_gamma(SOUL_TEAL, glow);
        let title_galley = p.layout_no_wrap(
            "SHAURMEAW".to_owned(),
            FontId::new(13.0, serif_family()),
            title_col,
        );
        let sub_galley = p.layout_no_wrap(
            "DEADLOCK".to_owned(),
            FontId::monospace(9.5),
            TEXT_MUTED,
        );
        let text_x = img_rect.right() + 10.0;
        let total_h = title_galley.size().y + sub_galley.size().y + 1.0;
        let top_y = rect.center().y - total_h * 0.5;
        p.galley(pos2(text_x, top_y), title_galley, title_col);
        p.galley(pos2(text_x, top_y + 16.0), sub_galley, TEXT_MUTED);
    }

    ui.add_space(4.0);
    let line_col = pal.accent_border.lerp_to_gamma(SOUL_TEAL, 0.75 * glow);
    let diamond_col = pal.accent.lerp_to_gamma(SOUL_TEAL, glow);
    paint_deco_divider(ui, line_col, diamond_col);
    resp
}

/// Рисует ровную пунктирную прямоугольную рамку.
pub fn paint_dashed_rect(
    painter: &Painter,
    rect: Rect,
    stroke: Stroke,
    dash_len: f32,
    gap_len: f32,
) {
    let draw_edge = |start: egui::Pos2, end: egui::Pos2| {
        let delta = end - start;
        let total = delta.length();
        if total <= 0.1 || dash_len <= 0.1 {
            return;
        }
        let dir = delta / total;
        let step = (dash_len + gap_len).max(1.0);
        let mut dist = 0.0_f32;
        while dist < total {
            let d_end = (dist + dash_len).min(total);
            painter.line_segment([start + dir * dist, start + dir * d_end], stroke);
            dist += step;
        }
    };

    let tl = rect.left_top();
    let tr = rect.right_top();
    let br = rect.right_bottom();
    let bl = rect.left_bottom();
    draw_edge(tl, tr);
    draw_edge(tr, br);
    draw_edge(br, bl);
    draw_edge(bl, tl);
}

/// Чистая минималистичная карточка с внутренним отступом `16px` и мягкой окантовкой Art Deco.
pub fn card_frame<R>(
    ui: &mut Ui,
    pal: &Palette,
    highlighted: bool,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    let bg_idx = ui.painter().add(Shape::Noop);
    let mut prepared = egui::Frame::new().inner_margin(Margin::same(16)).begin(ui);
    prepared
        .content_ui
        .set_min_width(prepared.content_ui.available_width());
    let ret = add_contents(&mut prepared.content_ui);
    let resp = prepared.allocate_space(ui);
    let rect = resp.rect;

    if ui.is_rect_visible(rect) {
        let (border_col, border_w) = if highlighted {
            (pal.accent, 1.3_f32)
        } else {
            (BORDER_SUBTLE, 1.0_f32)
        };
        ui.painter().set(
            bg_idx,
            Shape::Rect(RectShape::new(
                rect,
                CornerRadius::same(4),
                SURFACE_CARD,
                Stroke::new(border_w, border_col),
                StrokeKind::Inside,
            )),
        );
        if highlighted {
            paint_corner_brackets(
                ui.painter(),
                rect.shrink(1.0),
                6.0,
                Stroke::new(1.2_f32, pal.accent_light),
            );
        }
    }

    ret
}

/// Карточка-баннер уведомления (эктоплазменная или алая рамка с уголками Art Deco).
pub fn accent_banner_frame<R>(
    ui: &mut Ui,
    fill: Color32,
    border: Color32,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    let bg_idx = ui.painter().add(Shape::Noop);
    let mut prepared = egui::Frame::new()
        .inner_margin(Margin::symmetric(12, 9))
        .begin(ui);
    prepared
        .content_ui
        .set_min_width(prepared.content_ui.available_width());
    let ret = add_contents(&mut prepared.content_ui);
    let resp = prepared.allocate_space(ui);
    let rect = resp.rect;

    if ui.is_rect_visible(rect) {
        ui.painter().set(
            bg_idx,
            Shape::Rect(RectShape::new(
                rect,
                CornerRadius::same(2),
                fill,
                Stroke::new(1.0_f32, border),
                StrokeKind::Inside,
            )),
        );
        paint_corner_brackets(ui.painter(), rect, 7.0, Stroke::new(1.5_f32, border));
    }

    ret
}

// ============================================================ Кастомные кнопки с плавным hover

/// Главная кнопка с градиентом состаренной латуни, скосом углов, гравировкой и свечением при наведении.
pub fn brass_primary_button(
    ui: &mut Ui,
    icon: Option<Icon>,
    label: &str,
    min_size: egui::Vec2,
    enabled: bool,
    pal: &Palette,
) -> Response {
    let font_id = FontId::new(13.5, serif_family());
    let text_up = label.to_uppercase();
    let text_galley = ui.painter().layout_no_wrap(
        text_up.clone(),
        font_id.clone(),
        BG_NOIR,
    );
    let icon_space = if icon.is_some() { 20.0 } else { 0.0 };
    let desired_w = (text_galley.size().x + icon_space + 28.0).max(min_size.x);
    let desired_h = (text_galley.size().y + 14.0).max(min_size.y);

    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(desired_w, desired_h), sense);

    let anim_id = resp.id.with("brass_hover");
    let anim_t = ui.style().animation_time;
    let hover_t = ui.ctx().animate_bool_with_time_and_easing(
        anim_id,
        enabled && (resp.hovered() || resp.has_focus()),
        anim_t,
        easing::quadratic_out,
    );

    if enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }

    if ui.is_rect_visible(rect) {
        let p = ui.painter();

        // Мягкое тёплое латунное свечение при наведении
        if hover_t > 0.01 {
            p.rect_filled(
                rect.expand(2.5),
                4.0,
                pal.accent_light.gamma_multiply(0.16 * hover_t),
            );
        }

        let (top_col, bot_col, txt_col, outer_col) = if !enabled {
            (
                SURFACE_ELEVATED,
                SURFACE_CARD,
                TEXT_MUTED,
                BORDER_SUBTLE,
            )
        } else if resp.is_pointer_button_down_on() {
            (pal.grad_bot, pal.grad_bot, BG_NOIR, pal.accent_light)
        } else {
            (
                pal.grad_top.lerp_to_gamma(Color32::from_rgb(0xF0, 0xCA, 0x7E), 0.25 * hover_t),
                pal.grad_bot.lerp_to_gamma(pal.grad_top, 0.30 * hover_t),
                BG_NOIR,
                pal.accent_light,
            )
        };

        paint_chamfered_gradient_rect(
            p,
            rect,
            4.5,
            top_col,
            bot_col,
            Stroke::new(1.0_f32, outer_col),
            Some(Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(0x11, 0x13, 0x15, if enabled { 55 } else { 25 }),
            )),
        );

        let galley = if enabled {
            text_galley
        } else {
            p.layout_no_wrap(text_up, font_id, txt_col)
        };

        let content_w = icon_space + galley.size().x;
        let start_x = rect.center().x - content_w * 0.5;
        if let Some(ic) = icon {
            let icon_rect =
                Rect::from_center_size(pos2(start_x + 7.0, rect.center().y), vec2(14.0, 14.0));
            paint_icon(p, icon_rect, ic, txt_col, 1.7);
        }
        let text_pos = pos2(start_x + icon_space, rect.center().y - galley.size().y * 0.5);
        p.galley(text_pos, galley, txt_col);
    }

    resp
}

/// Второстепенная кнопка Art Deco: прозрачный фон, рамка 1px латунь, плавная заливка при наведении.
pub fn brass_secondary_button(
    ui: &mut Ui,
    icon: Option<Icon>,
    label: &str,
    min_size: egui::Vec2,
    enabled: bool,
    pal: &Palette,
) -> Response {
    outlined_action_button(
        ui,
        icon,
        label,
        min_size,
        enabled,
        pal.accent,
        pal.accent_light,
        TEXT_CREAM,
    )
}

/// Тревожная / деструктивная кнопка с рамкой `#C94A4A`.
pub fn crimson_button(
    ui: &mut Ui,
    icon: Option<Icon>,
    label: &str,
    min_size: egui::Vec2,
    enabled: bool,
) -> Response {
    outlined_action_button(
        ui,
        icon,
        label,
        min_size,
        enabled,
        CRIMSON,
        CRIMSON,
        TEXT_CREAM,
    )
}

#[allow(clippy::too_many_arguments)]
fn outlined_action_button(
    ui: &mut Ui,
    icon: Option<Icon>,
    label: &str,
    min_size: egui::Vec2,
    enabled: bool,
    border_col: Color32,
    icon_col: Color32,
    text_col: Color32,
) -> Response {
    let font_id = FontId::proportional(13.5);
    let draw_text_col = if enabled { text_col } else { TEXT_MUTED };
    let draw_border = if enabled { border_col } else { BORDER_SUBTLE };
    let draw_icon = if enabled { icon_col } else { TEXT_MUTED };

    let galley = ui.painter().layout_no_wrap(label.to_owned(), font_id, draw_text_col);
    let icon_space = if icon.is_some() { 19.0 } else { 0.0 };
    let desired_w = (galley.size().x + icon_space + 22.0).max(min_size.x);
    let desired_h = (galley.size().y + 11.0).max(min_size.y);

    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(desired_w, desired_h), sense);

    let anim_t = ui.style().animation_time;
    let hover_t = ui.ctx().animate_bool_with_time_and_easing(
        resp.id.with("sec_hover"),
        enabled && (resp.hovered() || resp.has_focus()),
        anim_t,
        easing::quadratic_out,
    );

    if enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }

    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let fill = if !enabled {
            SURFACE_CARD.gamma_multiply(0.55)
        } else if resp.is_pointer_button_down_on() {
            border_col.gamma_multiply(0.26)
        } else if hover_t > 0.01 {
            border_col.gamma_multiply(0.16 * hover_t)
        } else {
            Color32::from_rgba_unmultiplied(0x1B, 0x1E, 0x22, 120)
        };

        let stroke_w = if enabled { 1.0 + 0.35 * hover_t } else { 1.0 };
        p.rect(
            rect,
            CornerRadius::same(2),
            fill,
            Stroke::new(stroke_w, draw_border),
            StrokeKind::Inside,
        );

        let content_w = icon_space + galley.size().x;
        let start_x = rect.center().x - content_w * 0.5;
        if let Some(ic) = icon {
            let icon_rect =
                Rect::from_center_size(pos2(start_x + 6.5, rect.center().y), vec2(13.0, 13.0));
            paint_icon(p, icon_rect, ic, draw_icon, 1.5);
        }
        p.galley(
            pos2(start_x + icon_space, rect.center().y - galley.size().y * 0.5),
            galley,
            draw_text_col,
        );
    }

    resp
}

/// Компактный декоративный чип выбора (FOV, пресеты FPS, фоны прицела).
pub fn deco_chip(ui: &mut Ui, selected: bool, label: &str, pal: &Palette) -> Response {
    let font_id = FontId::proportional(13.0);
    let txt_col = if selected { BG_NOIR } else { TEXT_CREAM };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font_id, txt_col);
    let (rect, resp) = ui.allocate_exact_size(
        vec2(galley.size().x + 18.0, (galley.size().y + 9.0).max(26.0)),
        Sense::click(),
    );

    let anim_t = ui.style().animation_time;
    let hover_t = ui.ctx().animate_bool_with_time_and_easing(
        resp.id.with("chip_hover"),
        resp.hovered(),
        anim_t,
        easing::quadratic_out,
    );

    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }

    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let (fill, stroke_col) = if selected {
            (pal.accent, pal.accent_light)
        } else {
            (
                SURFACE_CARD.lerp_to_gamma(SURFACE_ELEVATED, hover_t),
                BORDER_SUBTLE.lerp_to_gamma(pal.accent, hover_t),
            )
        };
        p.rect(
            rect,
            CornerRadius::same(2),
            fill,
            Stroke::new(1.0_f32, stroke_col),
            StrokeKind::Inside,
        );
        p.galley(
            pos2(
                rect.center().x - galley.size().x * 0.5,
                rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            txt_col,
        );
    }

    resp
}

/// Пункт навигации в боковой панели с линейной иконкой 1.5px и плавной латунной планкой.
pub fn nav_tab_button(
    ui: &mut Ui,
    selected: bool,
    icon: Icon,
    title: &str,
    pal: &Palette,
) -> Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 38.0), Sense::click());

    let anim_t = ui.style().animation_time;
    let hover_t = ui.ctx().animate_bool_with_time_and_easing(
        resp.id.with("nav_hover"),
        resp.hovered(),
        anim_t,
        easing::quadratic_out,
    );
    let active_t = ui.ctx().animate_bool_with_time_and_easing(
        resp.id.with("nav_active"),
        selected,
        anim_t,
        easing::quadratic_out,
    );

    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }

    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let bg_fill = Color32::from_rgb(0x16, 0x19, 0x1D).lerp_to_gamma(
            SURFACE_ELEVATED,
            hover_t.max(active_t),
        );
        p.rect_filled(rect, 3.0, bg_fill);

        if active_t > 0.01 {
            let bar_h = rect.height() * (0.50 + 0.34 * active_t);
            let bar = Rect::from_center_size(
                pos2(rect.left() + 1.5, rect.center().y),
                vec2(3.0, bar_h),
            );
            p.rect_filled(bar, 1.0, pal.accent_light.gamma_multiply(active_t));
            p.rect_stroke(
                rect,
                3.0,
                Stroke::new(1.0_f32, pal.accent_border.gamma_multiply(active_t)),
                StrokeKind::Inside,
            );
        } else if hover_t > 0.05 {
            p.rect_stroke(
                rect,
                3.0,
                Stroke::new(1.0_f32, BORDER_SUBTLE.gamma_multiply(hover_t)),
                StrokeKind::Inside,
            );
        }

        let base_icon = Color32::from_rgb(0xAE, 0xA8, 0x9B);
        let base_text = Color32::from_rgb(0xD4, 0xCD, 0xBD);
        let icon_col = base_icon.lerp_to_gamma(pal.accent_light, active_t.max(hover_t * 0.75));
        let text_col = base_text.lerp_to_gamma(
            TEXT_CREAM.lerp_to_gamma(pal.accent_light, 0.55 * active_t),
            active_t.max(hover_t),
        );

        let ic_rect = Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(16.0, 16.0));
        paint_icon(p, ic_rect, icon, icon_col, 1.5);

        let font_id = if selected {
            FontId::new(13.5, serif_family())
        } else {
            FontId::proportional(14.0)
        };
        let galley = p.layout_no_wrap(title.to_owned(), font_id, text_col);
        p.galley(
            pos2(rect.left() + 38.0, rect.center().y - galley.size().y * 0.5),
            galley,
            text_col,
        );
    }

    resp
}

/// Плавный переключатель «Вкл / Выкл» (Toggle Switch) в эстетике Deadlock.
pub fn toggle_switch(ui: &mut Ui, on: &mut bool, pal: &Palette) -> Response {
    let enabled = ui.is_enabled();
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, mut resp) = ui.allocate_exact_size(vec2(38.0, 20.0), sense);

    if enabled && resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    if enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }

    let anim_t = ui.ctx().animate_bool_with_time_and_easing(
        resp.id.with("toggle_sw"),
        *on,
        ui.style().animation_time,
        easing::quadratic_out,
    );
    let hover_t = ui.ctx().animate_bool_with_time_and_easing(
        resp.id.with("toggle_hover"),
        enabled && resp.hovered(),
        ui.style().animation_time,
        easing::quadratic_out,
    );

    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let bg_col = if enabled {
            BG_MANUSCRIPT.lerp_to_gamma(SOUL_TEAL, anim_t)
        } else {
            BG_MANUSCRIPT
        };
        let stroke_col = if !enabled {
            BORDER_SUBTLE
        } else if *on {
            SOUL_TEAL.lerp_to_gamma(TEXT_CREAM, 0.20 * hover_t)
        } else {
            BORDER_SUBTLE.lerp_to_gamma(pal.accent, hover_t)
        };
        p.rect(
            rect,
            CornerRadius::same(10),
            bg_col,
            Stroke::new(1.0_f32, stroke_col),
            StrokeKind::Inside,
        );

        let knob_x = egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), anim_t);
        let knob_col = if !enabled {
            TEXT_MUTED.gamma_multiply(0.5)
        } else {
            TEXT_MUTED.lerp_to_gamma(BG_NOIR, anim_t)
        };
        p.circle_filled(pos2(knob_x, rect.center().y), 7.0, knob_col);
    }

    resp
}

/// Спокойная компактная кнопка «•••» для вызова меню дополнительных действий карточки.
pub fn dots_menu_button(ui: &mut Ui, pal: &Palette) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 24.0), Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let hover_t = ui.ctx().animate_bool_with_time_and_easing(
        resp.id.with("dots_hover"),
        resp.hovered(),
        ui.style().animation_time,
        easing::quadratic_out,
    );

    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        if hover_t > 0.01 {
            p.rect(
                rect,
                CornerRadius::same(3),
                SURFACE_ELEVATED.gamma_multiply(hover_t),
                Stroke::new(1.0_f32, pal.accent_border.gamma_multiply(hover_t)),
                StrokeKind::Inside,
            );
        }
        let icon_col = TEXT_MUTED.lerp_to_gamma(pal.accent_light, hover_t);
        paint_icon(
            p,
            Rect::from_center_size(rect.center(), vec2(14.0, 14.0)),
            Icon::MoreDots,
            icon_col,
            1.5,
        );
    }
    resp
}

/// Чистая строка настройки с подписью слева, тумблером справа и тултипом на всю строку.
pub fn setting_toggle_row(
    ui: &mut Ui,
    title: &str,
    tooltip: &str,
    value: &mut bool,
    pal: &Palette,
) -> Response {
    let row_resp = ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
        let sw = toggle_switch(ui, value, pal).on_hover_text(tooltip);
        ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
            ui.add(egui::Label::new(RichText::new(title).size(13.5).color(TEXT_CREAM)).wrap())
                .on_hover_text(tooltip);
        });
        sw
    });
    row_resp.inner
}

/// Строгий оккультный бейдж-плашка (`АКТИВЕН`, `ВЫБРАН`, `НЕ СОХРАНЕНО`, `ВКЛ`).
pub fn badge_pill(ui: &mut Ui, text: &str, color: Color32, with_dot: bool) {
    let font_id = FontId::monospace(10.5);
    let galley = ui.painter().layout_no_wrap(text.to_uppercase(), font_id, color);
    let dot_w = if with_dot { 12.0 } else { 0.0 };
    let (rect, _) = ui.allocate_exact_size(
        vec2(galley.size().x + dot_w + 14.0, 20.0),
        Sense::hover(),
    );
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect(
            rect,
            CornerRadius::same(2),
            color.gamma_multiply(0.14),
            Stroke::new(1.0_f32, color.gamma_multiply(0.75)),
            StrokeKind::Inside,
        );
        if with_dot {
            p.circle_filled(pos2(rect.left() + 8.5, rect.center().y), 3.0, color);
        }
        p.galley(
            pos2(
                rect.left() + 7.0 + dot_w,
                rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            color,
        );
    }
}

// ============================================================ Подсветка синтаксиса KeyValues (VDF) & CFG

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HighlightMode {
    Vdf,
    Cfg,
}

#[derive(Default)]
pub struct VdfHighlighter;

impl<'a> ComputerMut<(&'a str, u32), egui::text::LayoutJob> for VdfHighlighter {
    fn compute(&mut self, (code, size_bits): (&'a str, u32)) -> egui::text::LayoutJob {
        highlight_vdf(code, f32::from_bits(size_bits))
    }
}

#[derive(Default)]
pub struct CfgHighlighter;

impl<'a> ComputerMut<(&'a str, u32), egui::text::LayoutJob> for CfgHighlighter {
    fn compute(&mut self, (code, size_bits): (&'a str, u32)) -> egui::text::LayoutJob {
        highlight_cfg(code, f32::from_bits(size_bits))
    }
}

fn split_line_ending(line: &str) -> (&str, &str) {
    if let Some(s) = line.strip_suffix("\r\n") {
        (s, "\r\n")
    } else if let Some(s) = line.strip_suffix('\n') {
        (s, "\n")
    } else {
        (line, "")
    }
}

/// Ищет индекс начала однострочного `//` вне кавычек (чтобы URL вида `"https://..."` не ломался).
fn find_comment_start(line: &str) -> Option<usize> {
    let b = line.as_bytes();
    let mut in_quotes = false;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => in_quotes = !in_quotes,
            b'\\' if in_quotes && i + 1 < b.len() => i += 1,
            b'/' if !in_quotes && i + 1 < b.len() && b[i + 1] == b'/' => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

fn mono_fmt(font_id: &FontId, color: Color32) -> TextFormat {
    TextFormat::simple(font_id.clone(), color)
}

/// Подсветка KeyValues (VDF) для `gameinfo.gi` и `video.txt`:
/// ключи `#E5B869`, значения `#48D1B5`, скобки `#D4A359`, комментарии `#686D66`.
/// Побайтово сохраняет исходный текст (`job.text == code`).
pub fn highlight_vdf(code: &str, font_size: f32) -> egui::text::LayoutJob {
    let font_id = FontId::monospace(font_size);
    let mut job = egui::text::LayoutJob::default();
    job.text.reserve(code.len());

    let fmt_key = mono_fmt(&font_id, BRASS_LIGHT);
    let fmt_val = mono_fmt(&font_id, SOUL_VALUE);
    let fmt_brace = mono_fmt(&font_id, BRASS);
    let fmt_comment = mono_fmt(&font_id, VDF_COMMENT);
    let fmt_plain = mono_fmt(&font_id, TEXT_CREAM);

    for line in code.split_inclusive('\n') {
        let (body, ending) = split_line_ending(line);
        let (code_part, comment_part) = match find_comment_start(body) {
            Some(idx) => (&body[..idx], Some(&body[idx..])),
            None => (body, None),
        };

        let mut token_idx = 0usize;
        let bytes = code_part.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i].is_ascii_whitespace() {
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                job.append(&code_part[start..i], 0.0, fmt_plain.clone());
            } else if bytes[i] == b'{' || bytes[i] == b'}' {
                let ch_len = 1;
                job.append(&code_part[i..i + ch_len], 0.0, fmt_brace.clone());
                i += ch_len;
            } else if bytes[i] == b'"' {
                let start = i;
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == b'"' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                let fmt = if token_idx == 0 {
                    fmt_key.clone()
                } else {
                    fmt_val.clone()
                };
                job.append(&code_part[start..i], 0.0, fmt);
                token_idx += 1;
            } else {
                let start = i;
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && bytes[i] != b'{'
                    && bytes[i] != b'}'
                    && bytes[i] != b'"'
                {
                    i += 1;
                }
                let fmt = if token_idx == 0 {
                    fmt_key.clone()
                } else {
                    fmt_val.clone()
                };
                job.append(&code_part[start..i], 0.0, fmt);
                token_idx += 1;
            }
        }

        if let Some(cmt) = comment_part {
            job.append(cmt, 0.0, fmt_comment.clone());
        }
        if !ending.is_empty() {
            job.append(ending, 0.0, fmt_plain.clone());
        }
    }

    job
}

/// Подсветка консольных команд (`autoexec.cfg`): команда `#E5B869`, аргументы `#48D1B5`, `;` `#D4A359`, комментарии `#686D66`.
pub fn highlight_cfg(code: &str, font_size: f32) -> egui::text::LayoutJob {
    let font_id = FontId::monospace(font_size);
    let mut job = egui::text::LayoutJob::default();
    job.text.reserve(code.len());

    let fmt_cmd = mono_fmt(&font_id, BRASS_LIGHT);
    let fmt_arg = mono_fmt(&font_id, SOUL_VALUE);
    let fmt_sep = mono_fmt(&font_id, BRASS);
    let fmt_comment = mono_fmt(&font_id, VDF_COMMENT);
    let fmt_plain = mono_fmt(&font_id, TEXT_CREAM);

    for line in code.split_inclusive('\n') {
        let (body, ending) = split_line_ending(line);
        let (code_part, comment_part) = match find_comment_start(body) {
            Some(idx) => (&body[..idx], Some(&body[idx..])),
            None => (body, None),
        };

        let mut token_idx = 0usize;
        let bytes = code_part.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i].is_ascii_whitespace() {
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                job.append(&code_part[start..i], 0.0, fmt_plain.clone());
            } else if bytes[i] == b';' {
                job.append(&code_part[i..i + 1], 0.0, fmt_sep.clone());
                i += 1;
                token_idx = 0;
            } else if bytes[i] == b'"' {
                let start = i;
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'"' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                let fmt = if token_idx == 0 {
                    fmt_cmd.clone()
                } else {
                    fmt_arg.clone()
                };
                job.append(&code_part[start..i], 0.0, fmt);
                token_idx += 1;
            } else {
                let start = i;
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && bytes[i] != b';'
                    && bytes[i] != b'"'
                {
                    i += 1;
                }
                let fmt = if token_idx == 0 {
                    fmt_cmd.clone()
                } else {
                    fmt_arg.clone()
                };
                job.append(&code_part[start..i], 0.0, fmt);
                token_idx += 1;
            }
        }

        if let Some(cmt) = comment_part {
            job.append(cmt, 0.0, fmt_comment.clone());
        }
        if !ending.is_empty() {
            job.append(ending, 0.0, fmt_plain.clone());
        }
    }

    job
}

/// Параметры подсветки и перехода к совпадению поиска в редакторе «Манускрипт».
#[derive(Default, Clone, Copy)]
pub struct CodeEditorSearch<'a> {
    pub matches: &'a [TextMatch],
    pub active_match: Option<usize>,
    /// Прокрутить область кода к указанной строке (0-based) на этом кадре.
    pub scroll_to_line: Option<usize>,
    /// Установить выделение диапазона символов `(start_char, end_char)` на этом кадре.
    pub select_char_range: Option<(usize, usize)>,
}

/// Кэш готового `Galley` для редактора «Манускрипт», исключающий повторную подсветку и вёрстку 45 КБ текста на каждом кадре.
#[derive(Default)]
pub struct ManuscriptGalleyCache {
    mode: Option<HighlightMode>,
    font_bits: u32,
    search_sig: u64,
    accent_rgb: [u8; 4],
    source_text: String,
    galley: Option<Arc<egui::Galley>>,
}

/// Дополнительные опции шапки и статусов редактора «Манускрипт».
#[derive(Default)]
pub struct CodeEditorOptions<'a> {
    pub search: CodeEditorSearch<'a>,
    /// Результат проверки баланса скобок для VDF-конфигов (`gameinfo.gi`, `video.txt`).
    pub brace_check: Option<BraceCheck>,
    /// Можно ли откатить несохранённые правки к исходному виду.
    pub can_revert: bool,
    /// Предварительно вычисленное число строк, чтобы не сканировать текст на каждом кадре.
    pub line_count: Option<usize>,
    /// Внешний кэш вёрстки текста `Galley`.
    pub galley_cache: Option<&'a mut ManuscriptGalleyCache>,
    /// Подпись текущего поиска/активного совпадения для инвалидации `galley_cache`.
    pub galley_search_sig: u64,
}

/// Действия в шапке редактора кода «Манускрипт».
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct EditorHeaderAction {
    pub save: bool,
    pub save_and_apply: bool,
    pub font_smaller: bool,
    pub font_larger: bool,
    pub revert: bool,
    /// Переход к строке (0-based) при клике на индикатор ошибки скобок.
    pub jump_to_brace_error: Option<usize>,
}

/// Накладывает подсветку всех совпадений поиска (и активного совпадения) поверх секций `LayoutJob`,
/// не нарушая монотонность и покрытие байтовых диапазонов.
pub fn apply_search_highlights(
    job: &mut egui::text::LayoutJob,
    matches: &[TextMatch],
    active_match: Option<usize>,
    pal: &Palette,
) {
    if matches.is_empty() || job.sections.is_empty() {
        return;
    }

    let orig_sections = std::mem::take(&mut job.sections);
    let mut new_sections = Vec::with_capacity(orig_sections.len() + matches.len() * 2);
    let mut m_idx = 0usize;

    let passive_bg = Color32::from_rgba_unmultiplied(pal.accent.r(), pal.accent.g(), pal.accent.b(), 68);
    let active_bg = pal.accent_light;
    let active_fg = BG_NOIR;

    for sec in orig_sections {
        let sec_start = sec.byte_range.start;
        let sec_end = sec.byte_range.end;
        let mut cur = sec_start;
        let mut is_first_piece = true;

        if sec_start == sec_end {
            new_sections.push(sec);
            continue;
        }

        while let Some(m) = matches.get(m_idx) {
            if m.end_byte <= sec_start {
                m_idx += 1;
                continue;
            }
            if m.start_byte >= sec_end {
                break;
            }

            let ov_start = m.start_byte.max(cur);
            let ov_end = m.end_byte.min(sec_end);

            if ov_start > cur {
                new_sections.push(egui::text::LayoutSection {
                    leading_space: if is_first_piece { sec.leading_space } else { 0.0 },
                    byte_range: cur..ov_start,
                    format: sec.format.clone(),
                });
                is_first_piece = false;
                cur = ov_start;
            }

            if ov_end > ov_start {
                let mut hl_fmt = sec.format.clone();
                if active_match == Some(m_idx) {
                    hl_fmt.background = active_bg;
                    hl_fmt.color = active_fg;
                    hl_fmt.underline = Stroke::new(1.5_f32, TEXT_CREAM);
                } else {
                    hl_fmt.background = passive_bg;
                    hl_fmt.underline = Stroke::new(1.0_f32, pal.accent_border);
                }
                new_sections.push(egui::text::LayoutSection {
                    leading_space: if is_first_piece { sec.leading_space } else { 0.0 },
                    byte_range: ov_start..ov_end,
                    format: hl_fmt,
                });
                is_first_piece = false;
                cur = ov_end;
            }

            if m.end_byte <= sec_end {
                m_idx += 1;
            } else {
                break;
            }
        }

        if cur < sec_end {
            new_sections.push(egui::text::LayoutSection {
                leading_space: if is_first_piece { sec.leading_space } else { 0.0 },
                byte_range: cur..sec_end,
                format: sec.format,
            });
        }
    }

    job.sections = new_sections;
}

/// Вычисляет смещение `ScrollArea`, при котором целевая точка в экранных координатах
/// оказывается ровно в центре видимой области `inner_rect`, с ограничением по границам контента.
fn compute_centered_scroll_offset(
    cur_offset: egui::Vec2,
    target_screen_x: Option<f32>,
    target_screen_y: f32,
    inner_rect: Rect,
    content_size: egui::Vec2,
) -> egui::Vec2 {
    let max_offset = (content_size - inner_rect.size()).max(egui::Vec2::ZERO);
    let view_center = inner_rect.center();
    let new_y = (cur_offset.y + (target_screen_y - view_center.y)).clamp(0.0, max_offset.y);
    let new_x = match target_screen_x {
        Some(tx) => (cur_offset.x + (tx - view_center.x)).clamp(0.0, max_offset.x),
        None => 0.0,
    };
    vec2(new_x, new_y)
}

/// Старинный технический манускрипт: тёмный фон `#0E1012`, шапка архива, номера строк и подсветка VDF / CFG.
#[allow(clippy::too_many_arguments)]
pub fn code_editor_manuscript(
    ui: &mut Ui,
    id_salt: &str,
    title: &str,
    text: &mut dyn egui::TextBuffer,
    interactive: bool,
    desired_rows: usize,
    max_scroll_height: f32,
    font_size: f32,
    mode: HighlightMode,
    is_dirty: bool,
    show_save_buttons: bool,
    can_save: bool,
    hint_text: Option<&str>,
    opts: CodeEditorOptions<'_>,
    pal: &Palette,
) -> (EditorHeaderAction, bool) {
    let mut header_act = EditorHeaderAction::default();
    let mut text_changed = false;

    let bg_idx = ui.painter().add(Shape::Noop);
    let mut prepared = egui::Frame::new()
        .inner_margin(Margin::same(1))
        .begin(ui);
    prepared
        .content_ui
        .set_min_width(prepared.content_ui.available_width());

    {
        let cui = &mut prepared.content_ui;
        let line_count = opts
            .line_count
            .unwrap_or_else(|| text.as_str().bytes().filter(|&b| b == b'\n').count() + 1);

        // Шапка манускрипта: 2 чистые строки без right_to_left-наложений
        egui::Frame::new()
            .fill(SURFACE_CARD)
            .inner_margin(Margin::symmetric(12, 8))
            .show(cui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    let (ic_rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                    paint_icon(ui.painter(), ic_rect, Icon::Archive, pal.accent_light, 1.5);
                    ui.add(
                        egui::Label::new(serif_heading(title, 12.5, pal.accent_light)).wrap(),
                    );
                    ui.label(mono_text(format!("• {line_count} СТР."), 11.0, TEXT_MUTED));
                    if is_dirty {
                        badge_pill(ui, "НЕ СОХРАНЕНО", pal.accent_light, false);
                    }
                    if !interactive {
                        badge_pill(ui, "ЧТЕНИЕ", TEXT_MUTED, false);
                    }

                    // Индикатор баланса скобок {} для VDF
                    if let Some(bc) = opts.brace_check {
                        if bc.balanced {
                            badge_pill(ui, "{} OK", SOUL_TEAL, false);
                        } else {
                            let label = match bc.first_error_line {
                                Some(ln) => format!("⚠ СКОБКИ: стр. {ln}"),
                                None => format!("⚠ {} / {}", bc.open_count, bc.close_count),
                            };
                            let btn = egui::Button::new(mono_text(label, 10.5, CRIMSON))
                                .fill(Color32::from_rgba_unmultiplied(
                                    CRIMSON.r(),
                                    CRIMSON.g(),
                                    CRIMSON.b(),
                                    28,
                                ))
                                .stroke(Stroke::new(1.0_f32, CRIMSON))
                                .corner_radius(CornerRadius::same(2));
                            let resp = ui
                                .add(btn)
                                .on_hover_text("Нажмите, чтобы перейти к проблемной строке со скобкой");
                            if resp.clicked() {
                                header_act.jump_to_brace_error =
                                    Some(bc.first_error_line.unwrap_or(1).saturating_sub(1));
                            }
                        }
                    }

                    // Кнопки масштаба шрифта A− / A+
                    if interactive {
                        ui.add_space(4.0);
                        let minus = ui
                            .add_enabled(
                                font_size > 11.1,
                                egui::Button::new(mono_text("A−", 10.5, TEXT_CREAM))
                                    .fill(SURFACE_ELEVATED)
                                    .stroke(Stroke::new(1.0_f32, BORDER_SUBTLE))
                                    .corner_radius(CornerRadius::same(2)),
                            )
                            .on_hover_text("Уменьшить шрифт");
                        if minus.clicked() {
                            header_act.font_smaller = true;
                        }
                        let plus = ui
                            .add_enabled(
                                font_size < 17.9,
                                egui::Button::new(mono_text("A+", 10.5, TEXT_CREAM))
                                    .fill(SURFACE_ELEVATED)
                                    .stroke(Stroke::new(1.0_f32, BORDER_SUBTLE))
                                    .corner_radius(CornerRadius::same(2)),
                            )
                            .on_hover_text("Увеличить шрифт");
                        if plus.clicked() {
                            header_act.font_larger = true;
                        }
                    }
                });

                if show_save_buttons {
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        if brass_primary_button(
                            ui,
                            Some(Icon::Save),
                            "СОХРАНИТЬ И ПРИМЕНИТЬ",
                            vec2(0.0, 26.0),
                            can_save,
                            pal,
                        )
                        .clicked()
                        {
                            header_act.save_and_apply = true;
                        }
                        if brass_secondary_button(
                            ui,
                            Some(Icon::Save),
                            "СОХРАНИТЬ",
                            vec2(0.0, 26.0),
                            can_save,
                            pal,
                        )
                        .clicked()
                        {
                            header_act.save = true;
                        }
                        if is_dirty
                            && opts.can_revert
                            && brass_secondary_button(
                                ui,
                                Some(Icon::Restore),
                                "ОТКАТИТЬ",
                                vec2(0.0, 26.0),
                                interactive,
                                pal,
                            )
                            .on_hover_text("Отменить несохранённые правки")
                            .clicked()
                        {
                            header_act.revert = true;
                        }
                        ui.add_space(4.0);
                        ui.label(mono_text("Ctrl+S", 10.5, TEXT_MUTED));
                    });
                }
            });

        // Тонкий латунный разделитель под шапкой
        let div_w = cui.available_width();
        let (div_rect, _) = cui.allocate_exact_size(vec2(div_w, 1.0), Sense::hover());
        cui.painter().line_segment(
            [pos2(div_rect.left(), div_rect.top()), pos2(div_rect.right(), div_rect.top())],
            Stroke::new(1.0_f32, pal.accent_border),
        );

        let font_size = font_size.clamp(11.0, 18.0);
        let bits = font_size.to_bits();
        let search_matches = opts.search.matches;
        let search_active = opts.search.active_match;
        let mut galley_cache = opts.galley_cache;
        let search_sig = opts.galley_search_sig;
        let accent_rgb = pal.accent.to_array();

        let mut layouter = |ui: &Ui, string: &str, _wrap_width: f32| {
            if let Some(ref mut gc) = galley_cache {
                if let Some(ref cached) = gc.galley {
                    if gc.mode == Some(mode)
                        && gc.font_bits == bits
                        && gc.search_sig == search_sig
                        && gc.accent_rgb == accent_rgb
                        && gc.source_text.len() == string.len()
                        && gc.source_text == string
                    {
                        return Arc::clone(cached);
                    }
                }
            }
            let mut job = match mode {
                HighlightMode::Vdf => ui.ctx().memory_mut(|mem| {
                    mem.caches
                        .cache::<FrameCache<egui::text::LayoutJob, VdfHighlighter>>()
                        .get((string, bits))
                }),
                HighlightMode::Cfg => ui.ctx().memory_mut(|mem| {
                    mem.caches
                        .cache::<FrameCache<egui::text::LayoutJob, CfgHighlighter>>()
                        .get((string, bits))
                }),
            };
            // Без переноса длинных строк: 1 логическая строка = 1 визуальная строка, с горизонтальным скроллом
            job.wrap.max_width = f32::INFINITY;
            if !search_matches.is_empty() {
                apply_search_highlights(&mut job, search_matches, search_active, pal);
            }
            let galley = ui.fonts(|f| f.layout_job(job));
            if let Some(ref mut gc) = galley_cache {
                gc.mode = Some(mode);
                gc.font_bits = bits;
                gc.search_sig = search_sig;
                gc.accent_rgb = accent_rgb;
                gc.source_text.clear();
                gc.source_text.push_str(string);
                gc.galley = Some(Arc::clone(&galley));
            }
            galley
        };

        let digits = line_count.to_string().len().max(3);
        let mono_font = FontId::monospace(font_size);
        let glyph_w = cui.fonts(|f| f.glyph_width(&mono_font, '0')).max(6.5);
        let gutter_w = (digits as f32) * glyph_w + 18.0;
        let highlight_line = opts
            .search
            .active_match
            .and_then(|idx| opts.search.matches.get(idx))
            .map(|m| m.line_idx);

        egui::Frame::new()
            .fill(BG_MANUSCRIPT)
            .inner_margin(Margin::symmetric(8, 8))
            .show(cui, |ui| {
                let scroll_out = egui::ScrollArea::both()
                    .id_salt(("manuscript_scroll", id_salt))
                    .max_height(max_scroll_height)
                    .show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            let gutter_bg_shape = ui.painter().add(Shape::Noop);
                            let (gutter_anchor, _) =
                                ui.allocate_exact_size(vec2(gutter_w, 1.0), Sense::hover());

                            let mut edit = egui::TextEdit::multiline(text)
                                .id_salt(("manuscript_edit", id_salt))
                                .font(mono_font.clone())
                                .code_editor()
                                .frame(false)
                                .interactive(interactive)
                                .background_color(BG_MANUSCRIPT)
                                .desired_rows(desired_rows)
                                .desired_width(f32::INFINITY)
                                .layouter(&mut layouter);
                            if let Some(hint) = hint_text {
                                edit = edit.hint_text(hint);
                            }

                            let mut te_out = edit.show(ui);
                            if te_out.response.changed() {
                                text_changed = true;
                            }

                            // Если нужно выделить найденное совпадение (в символах char)
                            if let Some((start_c, end_c)) = opts.search.select_char_range {
                                te_out.state.cursor.set_char_range(Some(
                                    egui::text::CCursorRange::two(
                                        egui::text::CCursor::new(start_c),
                                        egui::text::CCursor::new(end_c),
                                    ),
                                ));
                                te_out.state.store(ui.ctx(), te_out.response.id);
                            }

                            // Определяем текущую строку курсора для подсветки номера строки
                            let cursor_row = te_out.cursor_range.map(|cr| cr.primary.rcursor.row);

                            // Вычисляем точные экранные координаты цели для центрирования скролла
                            let mut scroll_target: Option<(Option<f32>, f32)> = None;
                            if opts.search.scroll_to_line.is_some() {
                                if let Some((start_c, end_c)) = opts.search.select_char_range {
                                    let c_start =
                                        te_out.galley.from_ccursor(egui::text::CCursor::new(start_c));
                                    let c_end =
                                        te_out.galley.from_ccursor(egui::text::CCursor::new(end_c));
                                    let r_start = te_out.galley.pos_from_cursor(&c_start);
                                    let r_end = te_out.galley.pos_from_cursor(&c_end);
                                    let word_x = te_out.galley_pos.x
                                        + (r_start.center().x + r_end.center().x) * 0.5;
                                    let word_y = te_out.galley_pos.y
                                        + (r_start.center().y + r_end.center().y) * 0.5;
                                    scroll_target = Some((Some(word_x), word_y));
                                } else if let Some(target_row) = opts.search.scroll_to_line {
                                    let clamped =
                                        target_row.min(te_out.galley.rows.len().saturating_sub(1));
                                    if let Some(placed_row) = te_out.galley.rows.get(clamped) {
                                        let row_y =
                                            te_out.galley_pos.y + placed_row.rect.center().y;
                                        scroll_target = Some((None, row_y));
                                    }
                                }
                            }

                            // Фон и правый разделитель столбца номеров строк
                            let full_gutter_rect = Rect::from_min_max(
                                gutter_anchor.min,
                                pos2(
                                    gutter_anchor.min.x + gutter_w - 6.0,
                                    te_out.response.rect.bottom().max(gutter_anchor.max.y),
                                ),
                            );
                            ui.painter().set(
                                gutter_bg_shape,
                                Shape::Rect(RectShape::new(
                                    full_gutter_rect,
                                    CornerRadius::ZERO,
                                    Color32::from_rgb(0x0B, 0x0D, 0x0F),
                                    Stroke::NONE,
                                    StrokeKind::Inside,
                                )),
                            );
                            ui.painter().line_segment(
                                [
                                    pos2(full_gutter_rect.right(), full_gutter_rect.top()),
                                    pos2(full_gutter_rect.right(), full_gutter_rect.bottom()),
                                ],
                                Stroke::new(1.0_f32, BORDER_SUBTLE),
                            );

                            // Отрисовка номеров строк строго O(видимых_строк)
                            let clip = ui.clip_rect();
                            let num_font = FontId::monospace((font_size - 1.5).max(9.5));
                            for (row_idx, placed_row) in te_out.galley.rows.iter().enumerate() {
                                let local_r = placed_row.rect;
                                let y_top = te_out.galley_pos.y + local_r.min.y;
                                let y_bottom = te_out.galley_pos.y + local_r.max.y;
                                if y_bottom < clip.top() - 24.0 {
                                    continue;
                                }
                                if y_top > clip.bottom() + 24.0 {
                                    break;
                                }
                                let is_active =
                                    highlight_line == Some(row_idx) || cursor_row == Some(row_idx);
                                if is_active {
                                    let band = Rect::from_min_max(
                                        pos2(full_gutter_rect.left(), y_top),
                                        pos2(full_gutter_rect.right(), y_bottom),
                                    );
                                    ui.painter().rect_filled(
                                        band,
                                        CornerRadius::ZERO,
                                        Color32::from_rgba_unmultiplied(
                                            pal.accent.r(),
                                            pal.accent.g(),
                                            pal.accent.b(),
                                            32,
                                        ),
                                    );
                                }
                                let num_color = if is_active {
                                    pal.accent_light
                                } else {
                                    TEXT_MUTED.gamma_multiply(0.7)
                                };
                                ui.painter().text(
                                    pos2(full_gutter_rect.right() - 6.0, (y_top + y_bottom) * 0.5),
                                    egui::Align2::RIGHT_CENTER,
                                    format!("{}", row_idx + 1),
                                    num_font.clone(),
                                    num_color,
                                );
                            }

                            scroll_target
                        })
                        .inner
                    });

                // Применяем точное центрирование в state скролла сразу после расчёта размеров
                if let Some((opt_x, target_y)) = scroll_out.inner {
                    let mut state = scroll_out.state;
                    state.offset = compute_centered_scroll_offset(
                        state.offset,
                        opt_x,
                        target_y,
                        scroll_out.inner_rect,
                        scroll_out.content_size,
                    );
                    state.store(ui.ctx(), scroll_out.id);
                    ui.ctx().request_repaint();
                }
            });
    }

    let resp = prepared.allocate_space(ui);
    let rect = resp.rect;
    if ui.is_rect_visible(rect) {
        ui.painter().set(
            bg_idx,
            Shape::Rect(RectShape::new(
                rect,
                CornerRadius::same(2),
                BG_MANUSCRIPT,
                Stroke::new(1.0_f32, BORDER_SUBTLE),
                StrokeKind::Inside,
            )),
        );
        paint_corner_brackets(ui.painter(), rect, 8.0, Stroke::new(1.4_f32, pal.accent_border));
    }

    (header_act, text_changed)
}

#[cfg(test)]
mod manuscript_search_tests {
    use super::*;
    use crate::config_editor::{find_all_matches, ConfigKind};

    #[test]
    fn test_apply_search_highlights_splits_sections_cleanly() {
        let code = "\"GameInfo\"\n{\n\t\"ConVars\"\n\t{\n\t\t\"fps_max\"\t\t\"0\"\n\t\t\"fps_max_ui\"\t\t\"60\"\n\t}\n}\n";
        let mut job = highlight_vdf(code, 13.0);
        let matches = find_all_matches(code, "fps_max", false);
        assert_eq!(matches.len(), 2);

        let pal = Palette::from_theme(Theme::ShaurMeaw);
        apply_search_highlights(&mut job, &matches, Some(1), &pal);

        assert!(!job.sections.is_empty());
        assert_eq!(job.sections[0].byte_range.start, 0);
        for pair in job.sections.windows(2) {
            assert_eq!(pair[0].byte_range.end, pair[1].byte_range.start);
        }
        assert_eq!(
            job.sections.last().map(|s| s.byte_range.end),
            Some(job.text.len())
        );

        let passive_bg =
            Color32::from_rgba_unmultiplied(pal.accent.r(), pal.accent.g(), pal.accent.b(), 68);
        let has_passive = job.sections.iter().any(|s| s.format.background == passive_bg);
        let has_active = job
            .sections
            .iter()
            .any(|s| s.format.background == pal.accent_light && s.format.color == BG_NOIR);
        assert!(has_passive);
        assert!(has_active);
        let _ = ConfigKind::GameInfo;
    }

    #[test]
    fn test_compute_centered_scroll_offset_centers_and_clamps() {
        let inner = Rect::from_min_size(pos2(100.0, 200.0), vec2(500.0, 300.0));
        // Центр экрана: x = 350.0, y = 350.0
        let content = vec2(1200.0, 3000.0);
        // max_offset = (700.0, 2700.0)

        // 1. Цель ниже центра экрана (cur_offset.y = 400, слово в экранной y = 600 -> delta +250 -> 650)
        // Цель правее центра по горизонтали для длинной строки (cur_offset.x = 50, слово x = 700 -> delta +350 -> 400)
        let centered = compute_centered_scroll_offset(
            vec2(50.0, 400.0),
            Some(700.0),
            600.0,
            inner,
            content,
        );
        assert_eq!(centered, vec2(400.0, 650.0));

        // 2. Цель у самого начала файла — зажимается в 0.0
        let clamped_min = compute_centered_scroll_offset(
            vec2(10.0, 20.0),
            Some(120.0),
            210.0,
            inner,
            content,
        );
        assert_eq!(clamped_min, vec2(0.0, 0.0));

        // 3. Цель у самого конца файла — зажимается в max_offset, None по X сбрасывает X в 0.0
        let clamped_max = compute_centered_scroll_offset(
            vec2(300.0, 2600.0),
            None,
            800.0,
            inner,
            content,
        );
        assert_eq!(clamped_max, vec2(0.0, 2700.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vdf_highlighter_preserves_exact_text_and_colors() {
        let sample = "// Комментарий VDF\r\n\"GameInfo\"\r\n{\r\n    SearchPaths\r\n    {\r\n        Game                \"citadel/addons\" // моды\r\n        r_shadows           0\r\n    }\r\n}\r\n";
        let job = highlight_vdf(sample, 13.0);
        assert_eq!(job.text, sample, "highlight_vdf обязан побайтово сохранять текст");

        let colors: Vec<Color32> = job.sections.iter().map(|s| s.format.color).collect();
        assert!(colors.contains(&VDF_COMMENT), "Ожидался цвет комментария #686D66");
        assert!(colors.contains(&BRASS), "Ожидался цвет скобок #D4A359");
        assert!(colors.contains(&BRASS_LIGHT), "Ожидался цвет ключей #E5B869");
        assert!(colors.contains(&SOUL_VALUE), "Ожидался цвет значений #48D1B5");
    }

    #[test]
    fn test_cfg_highlighter_preserves_exact_text_and_colors() {
        let sample = "// autoexec\nfps_max 240; toggle cl_showfps 0 1\nbind \"F7\" \"toggleconsole\"\n";
        let job = highlight_cfg(sample, 13.0);
        assert_eq!(job.text, sample, "highlight_cfg обязан побайтово сохранять текст");

        let colors: Vec<Color32> = job.sections.iter().map(|s| s.format.color).collect();
        assert!(colors.contains(&VDF_COMMENT));
        assert!(colors.contains(&BRASS_LIGHT));
        assert!(colors.contains(&SOUL_VALUE));
        assert!(colors.contains(&BRASS));
    }
}
