//! Движок удобного редактора конфигов (`gameinfo.gi`, `autoexec.cfg`, `video.txt`):
//! - UTF-8-безопасный поиск и замена по всему тексту;
//! - структурный разбор команд и VDF-параметров для визуального режима «Команды»;
//! - точечное изменение значений, комментирование строк и умная вставка с сохранением `\r\n` и комментариев;
//! - проверка баланса фигурных скобок `{}` для VDF (`gameinfo.gi` и `video.txt`);
//! - встроенный русскоязычный справочник популярных команд Deadlock.

/// Тип редактируемого конфига.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ConfigKind {
    GameInfo,
    Autoexec,
    Video,
}

impl ConfigKind {
    /// Человекочитаемое имя файла конфига.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::GameInfo => "gameinfo.gi",
            Self::Autoexec => "autoexec.cfg",
            Self::Video => "video.txt",
        }
    }

    /// Является ли конфиг файлом в VDF-формате со скобками `{}`.
    pub fn is_vdf(self) -> bool {
        matches!(self, Self::GameInfo | Self::Video)
    }
}

/// Запись во встроенном справочнике команд Deadlock.
#[derive(Clone, Copy, Debug)]
pub struct CommandCatalogItem {
    pub category: &'static str,
    pub target_section: &'static str,
    pub key: &'static str,
    pub default_value: &'static str,
    pub recommended_value: &'static str,
    pub summary_ru: &'static str,
}

/// Результат проверки баланса скобок `{}` в VDF-файле.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct BraceCheck {
    pub balanced: bool,
    pub open_count: usize,
    pub close_count: usize,
    /// Номер строки (с 1), на которой обнаружена первая лишняя или незакрытая скобка.
    pub first_error_line: Option<usize>,
}

/// Найденное совпадение при поиске в тексте.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TextMatch {
    pub start_byte: usize,
    pub end_byte: usize,
    /// Номер строки с нуля.
    pub line_idx: usize,
    /// Номер символа (char) в строке с нуля.
    pub col_char: usize,
}

/// Разобранная команда или параметр из конфига для визуального режима «Команды».
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ParsedEntry {
    /// Номер строки в файле (с нуля).
    pub line_idx: usize,
    /// Раздел (например, `ConVars`, `RenderSystem`, `Настройки графики` или заголовок секции в `.cfg`).
    pub section: String,
    /// Имя параметра / консольной команды.
    pub key: String,
    /// Текущее значение (без внешних кавычек).
    pub value: String,
    /// Было ли значение заключено в кавычки.
    pub quoted: bool,
    /// Закомментирована ли строка (`//`).
    pub commented: bool,
    /// Встроенный комментарий в конце строки (текст после `//`), если есть.
    pub inline_comment: Option<String>,
}

// ---------------------------------------------------------------------------
// Справочник команд Deadlock
// ---------------------------------------------------------------------------

static CATALOG_GAMEINFO: &[CommandCatalogItem] = &[
    // Графика и FPS (ConVars)
    CommandCatalogItem {
        category: "Графика и FPS",
        target_section: "ConVars",
        key: "r_aspectratio",
        default_value: "1.77",
        recommended_value: "2.15",
        summary_ru: "Угол обзора (FOV) камеры от 3-го лица: 1.77 — стандарт (~80 FOV), 2.15 — ~90 FOV, 2.3–2.49 — шире (~95–100 FOV).",
    },
    CommandCatalogItem {
        category: "Графика и FPS",
        target_section: "ConVars",
        key: "r_indirectlighting",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Непрямое глобальное освещение. Отключение заметно повышает FPS без потери читаемости.",
    },
    CommandCatalogItem {
        category: "Графика и FPS",
        target_section: "ConVars",
        key: "r_directlighting",
        default_value: "true",
        recommended_value: "false",
        summary_ru: "Прямое освещение мира (false упрощает свет для максимального FPS и видимости героев).",
    },
    CommandCatalogItem {
        category: "Графика и FPS",
        target_section: "ConVars",
        key: "r_ssao",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Затенение окружения (SSAO). Отключение снижает нагрузку на видеокарту.",
    },
    CommandCatalogItem {
        category: "Графика и FPS",
        target_section: "ConVars",
        key: "r_shadows",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Динамические тени в мире. Отключение даёт ощутимый прирост производительности.",
    },
    CommandCatalogItem {
        category: "Графика и FPS",
        target_section: "ConVars",
        key: "lb_enable_shadow_casting",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Отрисовка теней от источников света. Стабилизирует фреймтайм в массовых замесах.",
    },
    CommandCatalogItem {
        category: "Графика и FPS",
        target_section: "ConVars",
        key: "lb_enable_lights",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Дополнительное динамическое освещение карты.",
    },
    CommandCatalogItem {
        category: "Графика и FPS",
        target_section: "ConVars",
        key: "r_depth_of_field",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Глубина резкости (размытие дальнего фона). Отключение делает картинку чётче.",
    },
    CommandCatalogItem {
        category: "Графика и FPS",
        target_section: "ConVars",
        key: "r_effects_bloom",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Эффект свечения (Bloom) на ярких поверхностях и умениях.",
    },
    CommandCatalogItem {
        category: "Графика и FPS",
        target_section: "ConVars",
        key: "r_citadel_fog_quality",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Качество атмосферного тумана (0 убирает лишнюю дымку и повышает видимость).",
    },
    CommandCatalogItem {
        category: "Графика и FPS",
        target_section: "ConVars",
        key: "r_grass_quality",
        default_value: "2",
        recommended_value: "0",
        summary_ru: "Качество растительности и травы (0 — минимум визуального шума).",
    },
    CommandCatalogItem {
        category: "Графика и FPS",
        target_section: "ConVars",
        key: "r_grass_alpha_test",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Пропуск сложного расчёта прозрачности листвы (0 экономит ресурсы GPU).",
    },
    CommandCatalogItem {
        category: "Графика и FPS",
        target_section: "ConVars",
        key: "r_particle_max_detail_level",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Детализация частиц и эффектов способностей (0 — облегчённый режим).",
    },
    CommandCatalogItem {
        category: "Графика и FPS",
        target_section: "ConVars",
        key: "r_texture_stream_max_resolution",
        default_value: "4096",
        recommended_value: "1024",
        summary_ru: "Максимальное разрешение потоковых текстур (1024 снижает потребление VRAM).",
    },
    CommandCatalogItem {
        category: "Графика и FPS",
        target_section: "ConVars",
        key: "mat_viewportscale",
        default_value: "1.0",
        recommended_value: "1.0",
        summary_ru: "Масштаб внутреннего 3D-рендера (0.85–1.0; снижение поднимает FPS на слабых GPU).",
    },
    CommandCatalogItem {
        category: "Графика и FPS",
        target_section: "ConVars",
        key: "r_farz",
        default_value: "-1",
        recommended_value: "6000",
        summary_ru: "Дальность плоскости отсечения геометрии (6000–8000 отсекает далёкий фон за картой).",
    },
    CommandCatalogItem {
        category: "Графика и FPS",
        target_section: "ConVars",
        key: "r_drawropes",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Отрисовка тросов/проводов на карте (0 экономит отрисовку мелкой геометрии).",
    },
    CommandCatalogItem {
        category: "Графика и FPS",
        target_section: "ConVars",
        key: "r_post_bloom",
        default_value: "true",
        recommended_value: "false",
        summary_ru: "Постобработка свечения (Post-Bloom). Отключение делает картинку чище и повышает FPS.",
    },
    CommandCatalogItem {
        category: "Графика и FPS",
        target_section: "ConVars",
        key: "r_arealights",
        default_value: "true",
        recommended_value: "false",
        summary_ru: "Площадные источники света (false разгружает видеокарту в бою).",
    },
    CommandCatalogItem {
        category: "Графика и FPS",
        target_section: "ConVars",
        key: "r_citadel_distancefield_farfield_enable",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Дальнее глобальное освещение на полях расстояний (0 заметно экономит ресурсы GPU).",
    },
    // Камера, HUD и ввод
    CommandCatalogItem {
        category: "Камера, HUD и ввод",
        target_section: "ConVars",
        key: "citadel_camera_wobble_disable",
        default_value: "0",
        recommended_value: "1",
        summary_ru: "Отключает покачивание камеры от 3-го лица при движении и попаданиях (1 — стабильная камера).",
    },
    CommandCatalogItem {
        category: "Камера, HUD и ввод",
        target_section: "ConVars",
        key: "citadel_camera_soft_collision",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Плавное приближение камеры возле стен (0 — более предсказуемое поведение у укрытий).",
    },
    CommandCatalogItem {
        category: "Камера, HUD и ввод",
        target_section: "ConVars",
        key: "citadel_portrait_world_renderer_off",
        default_value: "false",
        recommended_value: "true",
        summary_ru: "Отключает фоновый 3D-рендер портрета героя в HUD (true снижает лишнюю нагрузку).",
    },
    CommandCatalogItem {
        category: "Камера, HUD и ввод",
        target_section: "ConVars",
        key: "citadel_hud_objective_health_enabled",
        default_value: "2",
        recommended_value: "2",
        summary_ru: "Показ здоровья объектов в HUD (0 — выкл, 1 — святилища, 2 — башни T1/T2, 3 — все объекты).",
    },
    CommandCatalogItem {
        category: "Камера, HUD и ввод",
        target_section: "ConVars",
        key: "citadel_boss_glow_disabled",
        default_value: "0",
        recommended_value: "1",
        summary_ru: "Отключает обводку/свечение боссов и стражей (1 убирает лишний засвет и экономит FPS).",
    },
    CommandCatalogItem {
        category: "Камера, HUD и ввод",
        target_section: "ConVars",
        key: "panorama_disable_box_shadow",
        default_value: "0",
        recommended_value: "1",
        summary_ru: "Отключает тени у элементов HUD Panorama для более плавного интерфейса.",
    },
    CommandCatalogItem {
        category: "Камера, HUD и ввод",
        target_section: "ConVars",
        key: "panorama_use_backbuffer_directly",
        default_value: "0",
        recommended_value: "1",
        summary_ru: "Выводит UI Panorama напрямую в backbuffer без промежуточного копирования памяти.",
    },
    CommandCatalogItem {
        category: "Камера, HUD и ввод",
        target_section: "ConVars",
        key: "r_low_latency",
        default_value: "1",
        recommended_value: "1",
        summary_ru: "Режим низкой задержки ввода NVIDIA Reflex / AMD Anti-Lag (1 — Вкл, 2 — Boost).",
    },
    CommandCatalogItem {
        category: "Камера, HUD и ввод",
        target_section: "ConVars",
        key: "v8_maximum_heap_size_mb",
        default_value: "512",
        recommended_value: "1024",
        summary_ru: "Лимит памяти JS-движка интерфейса (1024 снижает микрофризы сборки мусора в UI).",
    },
    // Сеть
    CommandCatalogItem {
        category: "Сеть",
        target_section: "ConVars",
        key: "cl_interp_ratio",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Множитель сетевой интерполяции (0–1 — минимальная задержка на стабильном интернете).",
    },
    CommandCatalogItem {
        category: "Сеть",
        target_section: "ConVars",
        key: "cl_async_usercmd_send",
        default_value: "1",
        recommended_value: "1",
        summary_ru: "Асинхронная отправка пользовательских команд ввода для уменьшения задержки отклика.",
    },
    CommandCatalogItem {
        category: "Сеть",
        target_section: "ConVars",
        key: "cq_buffer_bloat_msecs_max",
        default_value: "120",
        recommended_value: "120",
        summary_ru: "Ограничение буфера сетевых команд клиента при скачках пинга (мс).",
    },
    // Система и RenderSystem
    CommandCatalogItem {
        category: "RenderSystem",
        target_section: "RenderSystem",
        key: "LowLatency",
        default_value: "1",
        recommended_value: "1",
        summary_ru: "Включает минимальную очередь кадров на уровне подсистемы RenderSystem.",
    },
    CommandCatalogItem {
        category: "RenderSystem",
        target_section: "RenderSystem",
        key: "MinStreamingPoolSizeMB",
        default_value: "1024",
        recommended_value: "1024",
        summary_ru: "Минимальный размер пула потоковых текстур в видеопамяти (МБ).",
    },
];

static CATALOG_AUTOEXEC: &[CommandCatalogItem] = &[
    // FPS и задержка
    CommandCatalogItem {
        category: "FPS и задержка",
        target_section: "Autoexec",
        key: "fps_max",
        default_value: "300",
        recommended_value: "0",
        summary_ru: "Ограничение FPS в матче (0 — без лимита, либо задайте частоту вашего монитора).",
    },
    CommandCatalogItem {
        category: "FPS и задержка",
        target_section: "Autoexec",
        key: "fps_max_ui",
        default_value: "120",
        recommended_value: "60",
        summary_ru: "Лимит FPS в меню и лобби — снижает нагрев видеокарты во время поиска матча.",
    },
    CommandCatalogItem {
        category: "FPS и задержка",
        target_section: "Autoexec",
        key: "engine_no_focus_sleep",
        default_value: "20",
        recommended_value: "0",
        summary_ru: "Задержка движка при свёрнутом окне в мс (0 — игра не урезает кадры при Alt+Tab).",
    },
    CommandCatalogItem {
        category: "FPS и задержка",
        target_section: "Autoexec",
        key: "engine_low_latency_sleep_after_client_tick",
        default_value: "false",
        recommended_value: "true",
        summary_ru: "Синхронизирует сон движка после клиентского тика для ровного фреймтайма.",
    },
    CommandCatalogItem {
        category: "FPS и задержка",
        target_section: "Autoexec",
        key: "r_low_latency",
        default_value: "1",
        recommended_value: "1",
        summary_ru: "Режим низкой задержки кадра GPU (1 — Вкл, 2 — Boost).",
    },
    CommandCatalogItem {
        category: "FPS и задержка",
        target_section: "Autoexec",
        key: "mat_viewportscale",
        default_value: "1.0",
        recommended_value: "1.0",
        summary_ru: "Масштаб разрешения 3D-рендера (HUD при этом остаётся в родном разрешении).",
    },
    // Эффекты и читаемость
    CommandCatalogItem {
        category: "Эффекты и читаемость",
        target_section: "Autoexec",
        key: "r_particle_max_detail_level",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Детализация частиц способностей и окружения (0 — минимум лишнего визуального шума).",
    },
    CommandCatalogItem {
        category: "Эффекты и читаемость",
        target_section: "Autoexec",
        key: "r_particle_max_draw_distance",
        default_value: "4000",
        recommended_value: "2000",
        summary_ru: "Максимальная дистанция отрисовки частиц и эффектов (2000 снижает нагрузку в дальних файтах).",
    },
    CommandCatalogItem {
        category: "Эффекты и читаемость",
        target_section: "Autoexec",
        key: "cl_particle_fallback_base",
        default_value: "0",
        recommended_value: "3",
        summary_ru: "Базовый уровень упрощения частиц (2–3 переключает тяжёлые эффекты в облегчённый режим).",
    },
    CommandCatalogItem {
        category: "Эффекты и читаемость",
        target_section: "Autoexec",
        key: "r_drawdecals",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Отрисовка декалей (следов выстрелов и взрывов на стенах; 0 повышает FPS).",
    },
    CommandCatalogItem {
        category: "Эффекты и читаемость",
        target_section: "Autoexec",
        key: "r_decals",
        default_value: "2048",
        recommended_value: "10",
        summary_ru: "Максимум одновременно видимых декалей на карте (0–10 для стабильного FPS).",
    },
    CommandCatalogItem {
        category: "Эффекты и читаемость",
        target_section: "Autoexec",
        key: "r_light_sensitivity_mode",
        default_value: "false",
        recommended_value: "true",
        summary_ru: "Щадящий режим ярких вспышек и эффектов умений (true смягчает резкие засветы).",
    },
    CommandCatalogItem {
        category: "Эффекты и читаемость",
        target_section: "Autoexec",
        key: "r_texture_stream_mip_bias",
        default_value: "0",
        recommended_value: "2",
        summary_ru: "Упрощение детализации текстур в реальном времени (0 — максимум, 2..8 — мыльнее, но выше FPS).",
    },
    CommandCatalogItem {
        category: "Эффекты и читаемость",
        target_section: "Autoexec",
        key: "cl_ragdoll_limit",
        default_value: "10",
        recommended_value: "1",
        summary_ru: "Максимальное число одновременно просчитываемых тел (ragdoll) на карте.",
    },
    // Сеть и телеметрия
    CommandCatalogItem {
        category: "Сеть и телеметрия",
        target_section: "Autoexec",
        key: "rate",
        default_value: "786432",
        recommended_value: "786432",
        summary_ru: "Лимит пропускной способности сети (байт/сек). Оптимально 786432 (до 1000000 на быстром канале).",
    },
    CommandCatalogItem {
        category: "Сеть и телеметрия",
        target_section: "Autoexec",
        key: "cl_interp",
        default_value: "0.015625",
        recommended_value: "0",
        summary_ru: "Задержка сетевой интерполяции в секундах (0 минимально задерживает позиции целей).",
    },
    CommandCatalogItem {
        category: "Сеть и телеметрия",
        target_section: "Autoexec",
        key: "cl_interp_ratio",
        default_value: "1",
        recommended_value: "1",
        summary_ru: "Множитель интерполяции пакетов (1 при стабильном соединении без потерь).",
    },
    CommandCatalogItem {
        category: "Сеть и телеметрия",
        target_section: "Autoexec",
        key: "cl_showfps",
        default_value: "0",
        recommended_value: "0",
        summary_ru: "Встроенный счётчик FPS в углу экрана (0 — выкл, 1 — простой FPS, 2 — фреймтайм).",
    },
    CommandCatalogItem {
        category: "Сеть и телеметрия",
        target_section: "Autoexec",
        key: "cl_hud_telemetry_frametime_show",
        default_value: "0",
        recommended_value: "2",
        summary_ru: "Показ телеметрии FPS / фреймтайма в HUD (0 — скрыть, 1 — при проблемах, 2 — всегда).",
    },
    CommandCatalogItem {
        category: "Сеть и телеметрия",
        target_section: "Autoexec",
        key: "cl_hud_telemetry_ping_show",
        default_value: "0",
        recommended_value: "2",
        summary_ru: "Отображение пинга в HUD (0 — скрыть, 1 — при скачках, 2 — всегда).",
    },
    CommandCatalogItem {
        category: "Сеть и телеметрия",
        target_section: "Autoexec",
        key: "cl_hud_telemetry_net_misdelivery_show",
        default_value: "0",
        recommended_value: "1",
        summary_ru: "Индикатор потерь и джиттера пакетов сети (1 — показывать при проблемах, 2 — всегда).",
    },
    // HUD, звук и сохранение
    CommandCatalogItem {
        category: "HUD, звук и сохранение",
        target_section: "Autoexec",
        key: "sensitivity",
        default_value: "1.5",
        recommended_value: "1.5",
        summary_ru: "Чувствительность мыши в матче.",
    },
    CommandCatalogItem {
        category: "HUD, звук и сохранение",
        target_section: "Autoexec",
        key: "citadel_minimap_player_width",
        default_value: "7",
        recommended_value: "7",
        summary_ru: "Размер иконок игроков на миникарте.",
    },
    CommandCatalogItem {
        category: "HUD, звук и сохранение",
        target_section: "Autoexec",
        key: "citadel_show_all_purchase_toasts",
        default_value: "true",
        recommended_value: "true",
        summary_ru: "Показывать уведомления о покупках предметов в левом интерфейсе (true/false).",
    },
    CommandCatalogItem {
        category: "HUD, звук и сохранение",
        target_section: "Autoexec",
        key: "snd_mute_losefocus",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Выключать ли звук при сворачивании игры (0 — звук остаётся слышен в фоне).",
    },
    CommandCatalogItem {
        category: "HUD, звук и сохранение",
        target_section: "Autoexec",
        key: "snd_mixahead",
        default_value: "0.05",
        recommended_value: "0.025",
        summary_ru: "Размер аудиобуфера в секундах (0.025 снижает задержку звука без треска на современном ПК).",
    },
    CommandCatalogItem {
        category: "HUD, звук и сохранение",
        target_section: "Autoexec",
        key: "volume",
        default_value: "1.0",
        recommended_value: "0.5",
        summary_ru: "Общая громкость игры от 0.0 до 1.0.",
    },
    CommandCatalogItem {
        category: "HUD, звук и сохранение",
        target_section: "Autoexec",
        key: "host_writeconfig",
        default_value: "",
        recommended_value: "",
        summary_ru: "Сохраняет применённые команды в конфиг игры в самом конце autoexec.cfg.",
    },
];

static CATALOG_VIDEO: &[CommandCatalogItem] = &[
    // Экран и развёртка
    CommandCatalogItem {
        category: "Экран и развёртка",
        target_section: "video.cfg",
        key: "setting.defaultres",
        default_value: "1920",
        recommended_value: "1920",
        summary_ru: "Ширина разрешения экрана в пикселях (например, 1920, 2560, 1280).",
    },
    CommandCatalogItem {
        category: "Экран и развёртка",
        target_section: "video.cfg",
        key: "setting.defaultresheight",
        default_value: "1080",
        recommended_value: "1080",
        summary_ru: "Высота разрешения экрана в пикселях (например, 1080, 1440, 960).",
    },
    CommandCatalogItem {
        category: "Экран и развёртка",
        target_section: "video.cfg",
        key: "setting.refreshrate_numerator",
        default_value: "144",
        recommended_value: "240",
        summary_ru: "Частота обновления монитора в Гц (числитель; в безрамочном режиме может быть 0).",
    },
    CommandCatalogItem {
        category: "Экран и развёртка",
        target_section: "video.cfg",
        key: "setting.refreshrate_denominator",
        default_value: "1",
        recommended_value: "1",
        summary_ru: "Делитель частоты обновления экрана (обычно 1, либо 0 вместе с numerator=0).",
    },
    CommandCatalogItem {
        category: "Экран и развёртка",
        target_section: "video.cfg",
        key: "setting.fullscreen",
        default_value: "1",
        recommended_value: "1",
        summary_ru: "Эксклюзивный полноэкранный режим (1 — минимальная задержка вывода).",
    },
    CommandCatalogItem {
        category: "Экран и развёртка",
        target_section: "video.cfg",
        key: "setting.nowindowborder",
        default_value: "0",
        recommended_value: "1",
        summary_ru: "Оконный режим без рамки (1 — мгновенное Alt+Tab, 0 при fullscreen=1).",
    },
    CommandCatalogItem {
        category: "Экран и развёртка",
        target_section: "video.cfg",
        key: "setting.mat_vsync",
        default_value: "0",
        recommended_value: "0",
        summary_ru: "Вертикальная синхронизация V-Sync (0 убирает задержку ввода).",
    },
    CommandCatalogItem {
        category: "Экран и развёртка",
        target_section: "video.cfg",
        key: "setting.fullscreen_min_on_focus_loss",
        default_value: "1",
        recommended_value: "1",
        summary_ru: "Сворачивать ли полноэкранное окно при потере фокуса (Alt+Tab).",
    },
    // Качество графики
    CommandCatalogItem {
        category: "Качество графики",
        target_section: "video.cfg",
        key: "setting.r_low_latency",
        default_value: "1",
        recommended_value: "2",
        summary_ru: "Режим низкой задержки NVIDIA Reflex / AMD Anti-Lag (1 — Вкл, 2 — Вкл + Boost).",
    },
    CommandCatalogItem {
        category: "Качество графики",
        target_section: "video.cfg",
        key: "setting.r_citadel_shadow_quality",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Качество теней в Deadlock (0 — минимум/выкл для максимального FPS).",
    },
    CommandCatalogItem {
        category: "Качество графики",
        target_section: "video.cfg",
        key: "setting.r_citadel_ssao_quality",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Качество затенения окружения SSAO (0 отключает эффект для прироста FPS).",
    },
    CommandCatalogItem {
        category: "Качество графики",
        target_section: "video.cfg",
        key: "setting.r_citadel_fog_quality",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Качество объёмного тумана (0 — чистая картинка на дистанции).",
    },
    CommandCatalogItem {
        category: "Качество графики",
        target_section: "video.cfg",
        key: "setting.r_effects_bloom",
        default_value: "true",
        recommended_value: "false",
        summary_ru: "Свечение эффектов способностей (false уменьшает засветы в бою).",
    },
    CommandCatalogItem {
        category: "Качество графики",
        target_section: "video.cfg",
        key: "setting.r_post_bloom",
        default_value: "true",
        recommended_value: "false",
        summary_ru: "Постобработка свечения (false повышает чёткость и FPS).",
    },
    CommandCatalogItem {
        category: "Качество графики",
        target_section: "video.cfg",
        key: "setting.r_depth_of_field",
        default_value: "true",
        recommended_value: "false",
        summary_ru: "Глубина резкости / размытие дальнего фона (false убирает размытие).",
    },
    CommandCatalogItem {
        category: "Качество графики",
        target_section: "video.cfg",
        key: "setting.r_arealights",
        default_value: "true",
        recommended_value: "false",
        summary_ru: "Площадные источники света (false разгружает видеокарту).",
    },
    CommandCatalogItem {
        category: "Качество графики",
        target_section: "video.cfg",
        key: "setting.r_texture_stream_mip_bias",
        default_value: "0",
        recommended_value: "2",
        summary_ru: "Сдвиг чёткости текстур (0 — максимум, 2 — экономия видеопамяти).",
    },
    CommandCatalogItem {
        category: "Качество графики",
        target_section: "video.cfg",
        key: "setting.shaderquality",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Качество шейдеров (0 — низкое, 1 — высокое).",
    },
    CommandCatalogItem {
        category: "Качество графики",
        target_section: "video.cfg",
        key: "setting.r_citadel_antialiasing",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Сглаживание картинки (0 — без сглаживания, минимум мыла и задержки).",
    },
    CommandCatalogItem {
        category: "Качество графики",
        target_section: "video.cfg",
        key: "setting.r_citadel_upscaling",
        default_value: "0",
        recommended_value: "0",
        summary_ru: "Режим апскейлинга FSR / DLSS (0 — отключено / нативное разрешение).",
    },
    CommandCatalogItem {
        category: "Качество графики",
        target_section: "video.cfg",
        key: "setting.r_dashboard_render_quality",
        default_value: "1",
        recommended_value: "0",
        summary_ru: "Качество 3D-фона в главном меню (0 снижает нагрузку вне матча).",
    },
    CommandCatalogItem {
        category: "Качество графики",
        target_section: "video.cfg",
        key: "setting.r_fullscreen_gamma",
        default_value: "2.2",
        recommended_value: "1.6",
        summary_ru: "Яркость/гамма в игре (1.6 — светлее, лучше видно врагов в тёмных зонах).",
    },
    CommandCatalogItem {
        category: "Качество графики",
        target_section: "video.cfg",
        key: "setting.r_light_sensitivity_mode",
        default_value: "false",
        recommended_value: "true",
        summary_ru: "Щадящий режим ярких вспышек (true смягчает резкие эффекты умений).",
    },
    CommandCatalogItem {
        category: "Качество графики",
        target_section: "video.cfg",
        key: "setting.mat_viewportscale",
        default_value: "1",
        recommended_value: "1",
        summary_ru: "Масштаб внутреннего рендера (от 0.5 до 1).",
    },
    CommandCatalogItem {
        category: "Качество графики",
        target_section: "video.cfg",
        key: "setting.useadvanced",
        default_value: "1",
        recommended_value: "1",
        summary_ru: "Использование расширенных настроек графики вместо автоматического пресета.",
    },
];

/// Справочник команд для указанного типа конфига.
pub fn catalog_for(kind: ConfigKind) -> &'static [CommandCatalogItem] {
    match kind {
        ConfigKind::GameInfo => CATALOG_GAMEINFO,
        ConfigKind::Autoexec => CATALOG_AUTOEXEC,
        ConfigKind::Video => CATALOG_VIDEO,
    }
}

/// Уникальный список категорий справочника в порядке появления.
pub fn categories_for(kind: ConfigKind) -> Vec<&'static str> {
    let mut cats = Vec::new();
    for item in catalog_for(kind) {
        if !cats.contains(&item.category) {
            cats.push(item.category);
        }
    }
    cats
}

/// Поиск описания команды в справочнике: сначала для текущего конфига, затем запасной поиск по остальным.
pub fn describe_command(kind: ConfigKind, key: &str) -> Option<&'static CommandCatalogItem> {
    let target = key.trim();
    if target.is_empty() {
        return None;
    }
    if let Some(found) = catalog_for(kind)
        .iter()
        .find(|item| item.key.eq_ignore_ascii_case(target))
    {
        return Some(found);
    }
    for other in [ConfigKind::GameInfo, ConfigKind::Autoexec, ConfigKind::Video] {
        if other == kind {
            continue;
        }
        if let Some(found) = catalog_for(other)
            .iter()
            .find(|item| item.key.eq_ignore_ascii_case(target))
        {
            return Some(found);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Вспомогательные функции работы со строками и отступами
// ---------------------------------------------------------------------------

/// Метаданные одной строки исходного текста без изменения окончания (`\r\n` или `\n`).
#[derive(Clone, Copy)]
struct LineSpan {
    content_start: usize,
    content_end: usize,
}

fn collect_line_spans(text: &str) -> Vec<LineSpan> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut pos = 0usize;
    let len = bytes.len();

    while pos < len {
        let content_start = pos;
        let mut content_end = pos;
        while content_end < len {
            let b = bytes[content_end];
            if b == b'\n' || b == b'\r' {
                break;
            }
            content_end += 1;
        }
        let mut eol_end = content_end;
        if eol_end < len && bytes[eol_end] == b'\r' {
            eol_end += 1;
        }
        if eol_end < len && bytes[eol_end] == b'\n' {
            eol_end += 1;
        }
        spans.push(LineSpan {
            content_start,
            content_end,
        });
        pos = eol_end;
    }

    if text.ends_with('\n') || text.ends_with('\r') || spans.is_empty() {
        spans.push(LineSpan {
            content_start: len,
            content_end: len,
        });
    }
    spans
}

fn detect_eol(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// Убирает из значения символы перевода строки, чтобы не ломать однострочный синтаксис.
fn sanitize_single_line_value(value: &str) -> String {
    value
        .chars()
        .filter(|&c| c != '\r' && c != '\n')
        .collect()
}

/// Экранирует кавычки для записи в двойных кавычках VDF.
fn escape_quoted_value(value: &str) -> String {
    let clean = sanitize_single_line_value(value);
    clean.replace('"', "'")
}

// ---------------------------------------------------------------------------
// UTF-8-безопасный поиск и замена
// ---------------------------------------------------------------------------

/// Вычисляет байтовые смещения начала каждой строки.
fn compute_line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (idx, b) in text.bytes().enumerate() {
        if b == b'\n' {
            starts.push(idx + 1);
        }
    }
    starts
}

/// Позиция байта -> (line_idx, col_char) с гарантией границ UTF-8.
fn byte_to_line_col(text: &str, line_starts: &[usize], byte_pos: usize) -> (usize, usize) {
    let clamped = byte_pos.min(text.len());
    let line_idx = match line_starts.binary_search(&clamped) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };
    let line_start = line_starts.get(line_idx).copied().unwrap_or(0);
    let col_char = text
        .get(line_start..clamped)
        .map(|s| s.chars().count())
        .unwrap_or(0);
    (line_idx, col_char)
}

/// Поиск всех непересекающихся вхождений подстроки в `text`.
/// Не допускает срез посередине многобайтовых UTF-8 символов.
pub fn find_all_matches(text: &str, query: &str, case_sensitive: bool) -> Vec<TextMatch> {
    if query.is_empty() || text.is_empty() {
        return Vec::new();
    }

    let line_starts = compute_line_starts(text);
    let mut matches = Vec::new();

    if case_sensitive {
        let mut cursor = 0usize;
        while cursor < text.len() {
            let Some(tail) = text.get(cursor..) else {
                break;
            };
            let Some(rel) = tail.find(query) else {
                break;
            };
            let start = cursor + rel;
            let end = start + query.len();
            if text.is_char_boundary(start) && text.is_char_boundary(end) {
                let (line_idx, col_char) = byte_to_line_col(text, &line_starts, start);
                matches.push(TextMatch {
                    start_byte: start,
                    end_byte: end,
                    line_idx,
                    col_char,
                });
            }
            cursor = end.max(start + 1);
        }
        return matches;
    }

    // Быстрый и полностью безопасный путь для ASCII-запроса:
    // В UTF-8 ни один байт < 0x80 не встречается внутри многобайтового символа.
    if query.is_ascii() {
        let q_bytes = query.as_bytes();
        let t_bytes = text.as_bytes();
        let q_len = q_bytes.len();
        if q_len > t_bytes.len() {
            return matches;
        }
        let mut i = 0usize;
        let max_i = t_bytes.len() - q_len;
        while i <= max_i {
            if let Some(slice) = t_bytes.get(i..i + q_len) {
                if slice.eq_ignore_ascii_case(q_bytes)
                    && text.is_char_boundary(i)
                    && text.is_char_boundary(i + q_len)
                {
                    let (line_idx, col_char) = byte_to_line_col(text, &line_starts, i);
                    matches.push(TextMatch {
                        start_byte: i,
                        end_byte: i + q_len,
                        line_idx,
                        col_char,
                    });
                    i += q_len;
                    continue;
                }
            }
            i += 1;
        }
        return matches;
    }

    // Общий Unicode-путь (например, поиск по русским комментариям без учёта регистра).
    let query_lower: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
    if query_lower.is_empty() {
        return matches;
    }
    let text_chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut idx = 0usize;
    while idx < text_chars.len() {
        let start_byte = text_chars[idx].0;
        let mut t_idx = idx;
        let mut q_idx = 0usize;
        let mut matched_end_byte = None;

        while q_idx < query_lower.len() && t_idx < text_chars.len() {
            let mut char_lower = text_chars[t_idx].1.to_lowercase();
            let mut failed = false;
            for lc in &mut char_lower {
                if q_idx >= query_lower.len() || lc != query_lower[q_idx] {
                    failed = true;
                    break;
                }
                q_idx += 1;
            }
            if failed {
                break;
            }
            t_idx += 1;
            if q_idx == query_lower.len() {
                matched_end_byte = Some(if t_idx < text_chars.len() {
                    text_chars[t_idx].0
                } else {
                    text.len()
                });
                break;
            }
        }

        if let Some(end_byte) = matched_end_byte {
            let (line_idx, col_char) = byte_to_line_col(text, &line_starts, start_byte);
            matches.push(TextMatch {
                start_byte,
                end_byte,
                line_idx,
                col_char,
            });
            idx = t_idx.max(idx + 1);
        } else {
            idx += 1;
        }
    }

    matches
}

/// Заменяет одно конкретное вхождение `m` в `text`.
pub fn replace_match(text: &str, m: TextMatch, replacement: &str) -> String {
    if m.start_byte > m.end_byte
        || m.end_byte > text.len()
        || !text.is_char_boundary(m.start_byte)
        || !text.is_char_boundary(m.end_byte)
    {
        return text.to_string();
    }
    let Some(before) = text.get(..m.start_byte) else {
        return text.to_string();
    };
    let Some(after) = text.get(m.end_byte..) else {
        return text.to_string();
    };
    let mut out = String::with_capacity(
        before.len() + replacement.len() + after.len(),
    );
    out.push_str(before);
    out.push_str(replacement);
    out.push_str(after);
    out
}

/// Заменяет все совпадения `query` на `replacement` и возвращает новый текст и число замен.
pub fn replace_all_matches(
    text: &str,
    query: &str,
    replacement: &str,
    case_sensitive: bool,
) -> (String, usize) {
    let matches = find_all_matches(text, query, case_sensitive);
    if matches.is_empty() {
        return (text.to_string(), 0);
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    let mut count = 0usize;

    for m in matches {
        if m.start_byte < cursor
            || m.end_byte > text.len()
            || !text.is_char_boundary(m.start_byte)
            || !text.is_char_boundary(m.end_byte)
        {
            continue;
        }
        if let Some(chunk) = text.get(cursor..m.start_byte) {
            out.push_str(chunk);
            out.push_str(replacement);
            cursor = m.end_byte;
            count += 1;
        }
    }
    if let Some(tail) = text.get(cursor..) {
        out.push_str(tail);
    }
    (out, count)
}

// ---------------------------------------------------------------------------
// Проверка скобок VDF (`gameinfo.gi` и `video.txt`)
// ---------------------------------------------------------------------------

/// Проверяет баланс фигурных скобок `{}` с учётом строковых литералов `"..."` и комментариев `//`.
pub fn validate_vdf_braces(text: &str) -> BraceCheck {
    let mut open_stack: Vec<usize> = Vec::new();
    let mut check = BraceCheck::default();

    for (idx, line) in text.lines().enumerate() {
        let current_line = idx + 1;
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0usize;
        let mut line_in_quotes = false;
        let mut line_escaped = false;

        while i < chars.len() {
            let c = chars[i];
            if line_escaped {
                line_escaped = false;
                i += 1;
                continue;
            }
            if c == '\\' && line_in_quotes {
                line_escaped = true;
                i += 1;
                continue;
            }
            if c == '"' {
                line_in_quotes = !line_in_quotes;
                i += 1;
                continue;
            }
            if !line_in_quotes {
                if c == '/' && chars.get(i + 1) == Some(&'/') {
                    break;
                }
                if c == '{' {
                    check.open_count += 1;
                    open_stack.push(current_line);
                } else if c == '}' {
                    check.close_count += 1;
                    if open_stack.pop().is_none() && check.first_error_line.is_none() {
                        check.first_error_line = Some(current_line);
                    }
                }
            }
            i += 1;
        }
    }

    if check.first_error_line.is_none() {
        check.first_error_line = open_stack.last().copied();
    }
    check.balanced =
        check.open_count == check.close_count && check.first_error_line.is_none() && open_stack.is_empty();
    check
}

// ---------------------------------------------------------------------------
// Разбор конфигов для режима «Команды» и точечного редактирования строк
// ---------------------------------------------------------------------------

/// Результат разбора одной VDF-строки вида `"key" "value" // comment`.
struct VdfLineParts<'a> {
    commented: bool,
    key: String,
    value: Option<String>,
    /// Начало значения (включая открывающую кавычку, если есть) относительно `line`.
    val_start: usize,
    /// Конец значения (включая закрывающую кавычку, если есть) относительно `line`.
    val_end: usize,
    quoted: bool,
    inline_comment: Option<&'a str>,
    has_open_brace: bool,
    has_close_brace: bool,
}

/// Выделяет первую пару токенов VDF из одной строки (учитывая кавычки и `//`).
fn parse_vdf_line<'a>(line: &'a str) -> Option<VdfLineParts<'a>> {
    let trimmed_start = line.len() - line.trim_start_matches([' ', '\t']).len();
    let rest = line.get(trimmed_start..)?;
    if rest.is_empty() {
        return None;
    }

    let (commented, work_offset, work) = if let Some(after_slash) = rest.strip_prefix("//") {
        let extra = after_slash.len() - after_slash.trim_start_matches([' ', '\t']).len();
        let off = trimmed_start + 2 + extra;
        let w = line.get(off..)?;
        (true, off, w)
    } else {
        (false, trimmed_start, rest)
    };

    if work.is_empty() {
        return None;
    }

    // Проверка на одиночную фигурную скобку (если строка активна)
    let first_ch = work.chars().next()?;
    if first_ch == '{' || first_ch == '}' {
        return Some(VdfLineParts {
            commented,
            key: String::new(),
            value: None,
            val_start: work_offset,
            val_end: work_offset,
            quoted: false,
            inline_comment: None,
            has_open_brace: first_ch == '{',
            has_close_brace: first_ch == '}',
        });
    }

    // Первый токен в VDF обязан начинаться с кавычки `"` (иначе для закомментированной строки это текстовый комментарий).
    if first_ch != '"' {
        if commented {
            return None;
        }
        // Допускаем незакавыченный идентификатор на случай нестандартных строк
        let key_end = work
            .find(|c: char| c.is_whitespace() || c == '{' || c == '}')
            .unwrap_or(work.len());
        let key = work.get(..key_end)?.to_string();
        if key.is_empty() {
            return None;
        }
        let after_key = work.get(key_end..)?.trim_start_matches([' ', '\t']);
        let has_open_brace = after_key.starts_with('{');
        return Some(VdfLineParts {
            commented,
            key,
            value: None,
            val_start: work_offset + key_end,
            val_end: work_offset + key_end,
            quoted: false,
            inline_comment: None,
            has_open_brace,
            has_close_brace: false,
        });
    }

    // Извлекаем ключ в кавычках `"..."`
    let (key, after_key_rel) = read_quoted_token(work)?;
    let abs_after_key = work_offset + after_key_rel;
    let tail = line.get(abs_after_key..)?;
    let ws_len = tail.len() - tail.trim_start_matches([' ', '\t']).len();
    let after_ws_abs = abs_after_key + ws_len;
    let after_ws = line.get(after_ws_abs..)?;

    if after_ws.is_empty() || after_ws.starts_with("//") {
        let inline_comment = after_ws
            .strip_prefix("//")
            .map(|c| c.trim());
        return Some(VdfLineParts {
            commented,
            key,
            value: None,
            val_start: after_ws_abs,
            val_end: after_ws_abs,
            quoted: true,
            inline_comment,
            has_open_brace: false,
            has_close_brace: false,
        });
    }

    if after_ws.starts_with('{') {
        return Some(VdfLineParts {
            commented,
            key,
            value: None,
            val_start: after_ws_abs,
            val_end: after_ws_abs,
            quoted: true,
            inline_comment: None,
            has_open_brace: true,
            has_close_brace: false,
        });
    }

    if after_ws.starts_with('"') {
        let (val, val_end_rel) = read_quoted_token(after_ws)?;
        let val_start = after_ws_abs;
        let val_end = after_ws_abs + val_end_rel;
        let trailing = line.get(val_end..)?.trim_start_matches([' ', '\t']);
        let inline_comment = trailing
            .strip_prefix("//")
            .map(|c| c.trim())
            .filter(|c| !c.is_empty());
        return Some(VdfLineParts {
            commented,
            key,
            value: Some(val),
            val_start,
            val_end,
            quoted: true,
            inline_comment,
            has_open_brace: false,
            has_close_brace: false,
        });
    }

    // Значение без кавычек до `//`
    let (code_part, comment_part) = split_inline_comment(after_ws);
    let val_trimmed = code_part.trim_end_matches([' ', '\t']);
    let val_start = after_ws_abs;
    let val_end = after_ws_abs + val_trimmed.len();
    Some(VdfLineParts {
        commented,
        key,
        value: Some(val_trimmed.to_string()),
        val_start,
        val_end,
        quoted: false,
        inline_comment: comment_part.map(str::trim).filter(|s| !s.is_empty()),
        has_open_brace: false,
        has_close_brace: false,
    })
}

/// Считывает строку в двойных кавычках с начала `s` (где `s` начинается с `"`).
/// Возвращает `(содержимое_без_кавычек, байтовая_длина_включая_закрывающую_кавычку)`.
fn read_quoted_token(s: &str) -> Option<(String, usize)> {
    let mut chars = s.char_indices();
    let (_, first) = chars.next()?;
    if first != '"' {
        return None;
    }
    let mut out = String::new();
    let mut escaped = false;
    for (byte_idx, ch) in chars {
        if escaped {
            out.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            return Some((out, byte_idx + ch.len_utf8()));
        }
        out.push(ch);
    }
    None
}

/// Делит остаток строки на код и встроенный комментарий `//` вне кавычек.
fn split_inline_comment(s: &str) -> (&str, Option<&str>) {
    let mut in_quotes = false;
    let mut escaped = false;
    let bytes = s.as_bytes();
    for (idx, ch) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && in_quotes {
            escaped = true;
            continue;
        }
        if ch == '"' {
            in_quotes = !in_quotes;
            continue;
        }
        if !in_quotes && ch == '/' && bytes.get(idx + 1) == Some(&b'/') {
            let left = s.get(..idx).unwrap_or(s);
            let right = s.get(idx + 2..).unwrap_or("");
            return (left, Some(right));
        }
    }
    (s, None)
}

/// Разбор одной строки `autoexec.cfg`.
struct CfgLineParts<'a> {
    commented: bool,
    key: String,
    value: String,
    quoted: bool,
    has_value_sep: bool,
    /// Начало значения (включая кавычки, если есть) относительно строки.
    val_start: usize,
    /// Конец значения (включая кавычки, если есть) относительно строки.
    val_end: usize,
    inline_comment: Option<&'a str>,
}

fn looks_like_cfg_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}

fn looks_like_commented_cfg_command(key: &str, raw_val: &str, kind: ConfigKind) -> bool {
    if !looks_like_cfg_key(key) {
        return false;
    }
    if describe_command(kind, key).is_some() {
        return true;
    }
    let known_prefixes = [
        "cl_", "r_", "mat_", "fps_", "snd_", "voice_", "citadel_", "sv_", "net_", "engine_",
        "panorama_", "cpu_", "gpu_", "m_", "hud_", "lb_", "sc_", "con_", "host_",
    ];
    if known_prefixes.iter().any(|p| key.starts_with(p))
        || matches!(key, "bind" | "alias" | "exec" | "rate" | "volume" | "echo")
    {
        return true;
    }
    // Если после имени команды идёт явное число/булево/кавычки
    let v = raw_val.trim();
    if v.is_empty() {
        return false;
    }
    let first = v.chars().next().unwrap_or(' ');
    first.is_ascii_digit()
        || first == '"'
        || first == '-'
        || first == '+'
        || v.eq_ignore_ascii_case("true")
        || v.eq_ignore_ascii_case("false")
}

fn parse_cfg_line<'a>(line: &'a str) -> Option<CfgLineParts<'a>> {
    let trimmed_start = line.len() - line.trim_start_matches([' ', '\t']).len();
    let rest = line.get(trimmed_start..)?;
    if rest.is_empty() {
        return None;
    }

    let (commented, work_offset, work) = if let Some(after_slash) = rest.strip_prefix("//") {
        let extra = after_slash.len() - after_slash.trim_start_matches([' ', '\t']).len();
        let off = trimmed_start + 2 + extra;
        let w = line.get(off..)?;
        (true, off, w)
    } else {
        (false, trimmed_start, rest)
    };

    if work.is_empty() || work.starts_with('=') || work.starts_with('-') || work.starts_with('#') {
        return None;
    }

    let (code_part, inline_comment_raw) = split_inline_comment(work);
    let inline_comment = inline_comment_raw
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let key_len = code_part
        .find(|c: char| c.is_whitespace())
        .unwrap_or(code_part.len());
    let key = code_part.get(..key_len)?;
    if !looks_like_cfg_key(key) {
        return None;
    }

    let after_key_in_code = code_part.get(key_len..)?;
    let ws_len = after_key_in_code.len() - after_key_in_code.trim_start_matches([' ', '\t']).len();
    let val_start = work_offset + key_len + ws_len;
    let raw_val_and_ws = code_part.get(key_len + ws_len..)?;
    let raw_val = raw_val_and_ws.trim_end_matches([' ', '\t']);
    let val_end = val_start + raw_val.len();

    if commented && !looks_like_commented_cfg_command(key, raw_val, ConfigKind::Autoexec) {
        return None;
    }

    let (value, quoted) = if raw_val.starts_with('"') && raw_val.ends_with('"') && raw_val.len() >= 2
    {
        if let Some((inner, used)) = read_quoted_token(raw_val) {
            if used == raw_val.len() {
                (inner, true)
            } else {
                (raw_val.to_string(), false)
            }
        } else {
            (raw_val.to_string(), false)
        }
    } else {
        (raw_val.to_string(), false)
    };

    Some(CfgLineParts {
        commented,
        key: key.to_string(),
        value,
        quoted,
        has_value_sep: ws_len > 0,
        val_start,
        val_end,
        inline_comment,
    })
}

/// Извлекает заголовок раздела из декоративного комментария `autoexec.cfg`, например `// === 1. СЕТЬ ===`.
fn parse_cfg_section_banner(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let after = trimmed.strip_prefix("//")?.trim();
    if !(after.contains("===") || after.contains("---")) {
        return None;
    }
    let clean = after.trim_matches(|c: char| c == '=' || c == '-' || c.is_whitespace());
    if clean.is_empty() {
        None
    } else {
        Some(clean.to_string())
    }
}

/// Разбирает конфиг на список редактируемых параметров для режима «Команды».
pub fn parse_config_entries(text: &str, kind: ConfigKind) -> Vec<ParsedEntry> {
    match kind {
        ConfigKind::Autoexec => parse_autoexec_entries(text),
        ConfigKind::GameInfo | ConfigKind::Video => parse_vdf_entries(text, kind),
    }
}

fn parse_autoexec_entries(text: &str) -> Vec<ParsedEntry> {
    let mut entries = Vec::new();
    let mut current_section = "Основные команды".to_string();

    for (line_idx, line) in text.lines().enumerate() {
        if let Some(banner) = parse_cfg_section_banner(line) {
            current_section = banner;
            continue;
        }
        if let Some(parts) = parse_cfg_line(line) {
            entries.push(ParsedEntry {
                line_idx,
                section: current_section.clone(),
                key: parts.key,
                value: parts.value,
                quoted: parts.quoted,
                commented: parts.commented,
                inline_comment: parts.inline_comment.map(str::to_string),
            });
        }
    }
    entries
}

fn is_gameinfo_editable_scope(scope: &[String]) -> bool {
    if scope
        .iter()
        .any(|s| s.eq_ignore_ascii_case("FileSystem") || s.eq_ignore_ascii_case("SearchPaths"))
    {
        return false;
    }
    scope.iter().any(|s| {
        s.eq_ignore_ascii_case("ConVars")
            || s.eq_ignore_ascii_case("RenderSystem")
            || s.eq_ignore_ascii_case("Engine2")
            || s.eq_ignore_ascii_case("Memory")
    })
}

fn format_vdf_section_label(kind: ConfigKind, scope: &[String], key: &str) -> String {
    if kind == ConfigKind::Video {
        if key.starts_with("setting.") {
            return "Настройки графики".to_string();
        }
        return "Устройство и версия".to_string();
    }
    // Для GameInfo собираем путь без внешнего «GameInfo»
    let meaningful: Vec<&str> = scope
        .iter()
        .map(String::as_str)
        .filter(|s| !s.eq_ignore_ascii_case("GameInfo"))
        .collect();
    if meaningful.is_empty() {
        "GameInfo".to_string()
    } else {
        meaningful.join(" / ")
    }
}

fn parse_vdf_entries(text: &str, kind: ConfigKind) -> Vec<ParsedEntry> {
    let mut entries = Vec::new();
    let mut scope: Vec<String> = Vec::new();
    let mut pending_section: Option<String> = None;

    for (line_idx, line) in text.lines().enumerate() {
        let Some(parts) = parse_vdf_line(line) else {
            // Проверка: вдруг активная строка заканчивается одиночным `}`
            let trimmed = line.trim();
            if trimmed == "}" {
                scope.pop();
                pending_section = None;
            }
            continue;
        };

        if !parts.commented {
            if parts.has_close_brace {
                scope.pop();
                pending_section = None;
                continue;
            }
            if parts.has_open_brace {
                if !parts.key.is_empty() {
                    scope.push(parts.key);
                } else if let Some(sec) = pending_section.take() {
                    scope.push(sec);
                } else {
                    scope.push(String::new());
                }
                continue;
            }
            if parts.value.is_none() && !parts.key.is_empty() {
                pending_section = Some(parts.key);
                continue;
            }
        }

        if let Some(value) = parts.value {
            if !parts.key.is_empty() {
                let include = match kind {
                    ConfigKind::GameInfo => is_gameinfo_editable_scope(&scope),
                    ConfigKind::Video => !scope.is_empty(),
                    ConfigKind::Autoexec => false,
                };
                if include {
                    let section = format_vdf_section_label(kind, &scope, &parts.key);
                    entries.push(ParsedEntry {
                        line_idx,
                        section,
                        key: parts.key,
                        value,
                        quoted: parts.quoted,
                        commented: parts.commented,
                        inline_comment: parts.inline_comment.map(str::to_string),
                    });
                }
            }
        }
    }

    entries
}

/// Заменяет значение параметра на конкретной строке `line_idx`, сохраняя отступы, комментарии и окончания строк (`\r\n`/`\n`).
pub fn set_line_value(text: &str, kind: ConfigKind, line_idx: usize, new_value: &str) -> String {
    let spans = collect_line_spans(text);
    let Some(span) = spans.get(line_idx).copied() else {
        return text.to_string();
    };
    let Some(line) = text.get(span.content_start..span.content_end) else {
        return text.to_string();
    };

    let updated_line = if kind.is_vdf() {
        let Some(parts) = parse_vdf_line(line) else {
            return text.to_string();
        };
        if parts.value.is_none() {
            return text.to_string();
        }
        let formatted_val = if parts.quoted {
            format!("\"{}\"", escape_quoted_value(new_value))
        } else {
            sanitize_single_line_value(new_value)
        };
        let before = line.get(..parts.val_start).unwrap_or("");
        let after = line.get(parts.val_end..).unwrap_or("");
        format!("{before}{formatted_val}{after}")
    } else {
        let Some(parts) = parse_cfg_line(line) else {
            return text.to_string();
        };
        let formatted_val = if parts.quoted {
            format!("\"{}\"", escape_quoted_value(new_value))
        } else {
            sanitize_single_line_value(new_value)
        };
        let before = line.get(..parts.val_start).unwrap_or("");
        let after = line.get(parts.val_end..).unwrap_or("");
        if !parts.has_value_sep && !formatted_val.is_empty() {
            format!("{before} {formatted_val}{after}")
        } else {
            format!("{before}{formatted_val}{after}")
        }
    };

    let prefix = text.get(..span.content_start).unwrap_or("");
    let suffix = text.get(span.content_end..).unwrap_or("");
    let mut out = String::with_capacity(prefix.len() + updated_line.len() + suffix.len());
    out.push_str(prefix);
    out.push_str(&updated_line);
    out.push_str(suffix);
    out
}

/// Включает или отключает комментарий `//` на строке `line_idx`, сохраняя отступы и окончания строк.
pub fn toggle_line_comment(text: &str, line_idx: usize) -> String {
    let spans = collect_line_spans(text);
    let Some(span) = spans.get(line_idx).copied() else {
        return text.to_string();
    };
    let Some(line) = text.get(span.content_start..span.content_end) else {
        return text.to_string();
    };

    let indent_len = line.len() - line.trim_start_matches([' ', '\t']).len();
    let indent = line.get(..indent_len).unwrap_or("");
    let rest = line.get(indent_len..).unwrap_or("");

    let updated_line = if let Some(after_slash_space) = rest.strip_prefix("// ") {
        format!("{indent}{after_slash_space}")
    } else if let Some(after_slash) = rest.strip_prefix("//") {
        format!("{indent}{after_slash}")
    } else {
        format!("{indent}//{rest}")
    };

    let prefix = text.get(..span.content_start).unwrap_or("");
    let suffix = text.get(span.content_end..).unwrap_or("");
    let mut out = String::with_capacity(prefix.len() + updated_line.len() + suffix.len());
    out.push_str(prefix);
    out.push_str(&updated_line);
    out.push_str(suffix);
    out
}

/// Вставляет команду в правильное место конфига (в блок `ConVars`/`RenderSystem`/`video.cfg` или перед `host_writeconfig`).
/// Возвращает `(новый_текст, номер_добавленной_строки_с_нуля)`.
pub fn insert_command_smart(
    text: &str,
    kind: ConfigKind,
    target_section: &str,
    key: &str,
    value: &str,
    comment: Option<&str>,
) -> (String, usize) {
    let eol = detect_eol(text);
    let clean_key = sanitize_single_line_value(key).trim().to_string();
    let clean_val = escape_quoted_value(value);

    match kind {
        ConfigKind::Autoexec => {
            let formatted = if clean_val.is_empty() {
                if let Some(c) = comment.map(str::trim).filter(|s| !s.is_empty()) {
                    format!("{clean_key} // {c}")
                } else {
                    clean_key
                }
            } else if let Some(c) = comment.map(str::trim).filter(|s| !s.is_empty()) {
                format!("{clean_key} {clean_val} // {c}")
            } else {
                format!("{clean_key} {clean_val}")
            };

            let spans = collect_line_spans(text);
            // Если в конце autoexec есть host_writeconfig — вставляем прямо перед ним
            for (idx, span) in spans.iter().enumerate() {
                let line = text.get(span.content_start..span.content_end).unwrap_or("");
                if let Some(parts) = parse_cfg_line(line) {
                    if !parts.commented && parts.key.eq_ignore_ascii_case("host_writeconfig") {
                        return insert_line_at_byte(text, span.content_start, idx, &formatted, eol, false);
                    }
                }
            }

            let line_idx = if text.is_empty() {
                0
            } else if text.ends_with('\n') || text.ends_with('\r') {
                spans.len().saturating_sub(1)
            } else {
                spans.len()
            };
            let need_leading_eol = !text.is_empty() && !text.ends_with('\n') && !text.ends_with('\r');
            insert_line_at_byte(text, text.len(), line_idx, &formatted, eol, need_leading_eol)
        }
        ConfigKind::Video => {
            let formatted_line = if let Some(c) = comment.map(str::trim).filter(|s| !s.is_empty()) {
                format!("\t\"{clean_key}\"\t\t\"{clean_val}\"\t// {c}")
            } else {
                format!("\t\"{clean_key}\"\t\t\"{clean_val}\"")
            };
            if let Some((close_byte, close_idx, indent)) = find_vdf_section_close(text, "video.cfg") {
                let line_text = if indent.is_empty() {
                    formatted_line
                } else if let Some(c) = comment.map(str::trim).filter(|s| !s.is_empty()) {
                    format!("{indent}\"{clean_key}\"\t\t\"{clean_val}\"\t// {c}")
                } else {
                    format!("{indent}\"{clean_key}\"\t\t\"{clean_val}\"")
                };
                return insert_line_at_byte(text, close_byte, close_idx, &line_text, eol, false);
            }
            let spans = collect_line_spans(text);
            let line_idx = spans.len().saturating_sub(1);
            let need_leading = !text.is_empty() && !text.ends_with('\n') && !text.ends_with('\r');
            insert_line_at_byte(text, text.len(), line_idx, &formatted_line, eol, need_leading)
        }
        ConfigKind::GameInfo => {
            let section_name = if target_section.is_empty() {
                "ConVars"
            } else {
                target_section
            };
            if let Some((close_byte, close_idx, indent)) = find_vdf_section_close(text, section_name)
            {
                let use_indent = if indent.is_empty() { "\t\t" } else { indent.as_str() };
                let line_text = if let Some(c) = comment.map(str::trim).filter(|s| !s.is_empty()) {
                    format!("{use_indent}\"{clean_key}\"\t\t\"{clean_val}\"\t// {c}")
                } else {
                    format!("{use_indent}\"{clean_key}\"\t\t\"{clean_val}\"")
                };
                return insert_line_at_byte(text, close_byte, close_idx, &line_text, eol, false);
            }
            // Если нужная секция (например ConVars) отсутствует, попробуем вставить перед закрытием GameInfo.
            if let Some((close_byte, close_idx, _)) = find_vdf_section_close(text, "GameInfo") {
                let block = format!(
                    "\t\"{section_name}\"{eol}\t{{{eol}\t\t\"{clean_key}\"\t\t\"{clean_val}\"{eol}\t}}"
                );
                return insert_line_at_byte(text, close_byte, close_idx, &block, eol, false);
            }
            let fallback = format!("\t\"{clean_key}\"\t\t\"{clean_val}\"");
            let spans = collect_line_spans(text);
            let line_idx = spans.len().saturating_sub(1);
            let need_leading = !text.is_empty() && !text.ends_with('\n') && !text.ends_with('\r');
            insert_line_at_byte(text, text.len(), line_idx, &fallback, eol, need_leading)
        }
    }
}

fn insert_line_at_byte(
    text: &str,
    byte_pos: usize,
    inserted_line_idx: usize,
    line_content: &str,
    eol: &str,
    need_leading_eol: bool,
) -> (String, usize) {
    let clamped = byte_pos.min(text.len());
    let prefix = text.get(..clamped).unwrap_or(text);
    let suffix = text.get(clamped..).unwrap_or("");
    let mut out =
        String::with_capacity(text.len() + line_content.len() + eol.len() * 2);
    out.push_str(prefix);
    if need_leading_eol {
        out.push_str(eol);
    }
    out.push_str(line_content);
    if clamped < text.len() || !line_content.ends_with('\n') {
        out.push_str(eol);
    }
    out.push_str(suffix);
    (out, inserted_line_idx)
}

/// Ищет закрывающую `}` нужного блока VDF и рекомендуемый отступ строк внутри блока.
/// Возвращает `(byte_offset_начала_строки_с_}, line_idx_с_}, отступ_соседней_строки)`.
fn find_vdf_section_close(text: &str, section_name: &str) -> Option<(usize, usize, String)> {
    let spans = collect_line_spans(text);
    let mut scope: Vec<String> = Vec::new();
    let mut pending_section: Option<String> = None;
    let mut target_depth: Option<usize> = None;
    let mut sample_indent = String::new();

    for (line_idx, span) in spans.iter().enumerate() {
        let line = text.get(span.content_start..span.content_end).unwrap_or("");
        let parts = parse_vdf_line(line);

        if let Some(ref p) = parts {
            if !p.commented {
                if p.has_close_brace {
                    if let Some(td) = target_depth {
                        if scope.len() == td {
                            return Some((span.content_start, line_idx, sample_indent));
                        }
                    }
                    scope.pop();
                    pending_section = None;
                    continue;
                }
                if p.has_open_brace {
                    let pushed = if !p.key.is_empty() {
                        p.key.clone()
                    } else {
                        pending_section.take().unwrap_or_default()
                    };
                    let just_entered_target =
                        target_depth.is_none() && pushed.eq_ignore_ascii_case(section_name);
                    scope.push(pushed);
                    if just_entered_target {
                        target_depth = Some(scope.len());
                    }
                    continue;
                }
                if p.value.is_none() && !p.key.is_empty() {
                    pending_section = Some(p.key.clone());
                    continue;
                }
                if target_depth == Some(scope.len()) && p.value.is_some() && sample_indent.is_empty()
                {
                    let ind_len = line.len() - line.trim_start_matches([' ', '\t']).len();
                    sample_indent = line.get(..ind_len).unwrap_or("").to_string();
                }
            }
        } else if line.trim() == "}" {
            if let Some(td) = target_depth {
                if scope.len() == td {
                    return Some((span.content_start, line_idx, sample_indent));
                }
            }
            scope.pop();
            pending_section = None;
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Тесты
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_and_replace_utf8_safe_and_case_insensitive() {
        let text = "// Настройка FPS\r\nfps_max 144\r\nfps_max_ui 60\r\n// Повтор fps_MAX здесь";
        let matches_ci = find_all_matches(text, "fps_max", false);
        assert_eq!(matches_ci.len(), 3);
        assert_eq!(matches_ci[0].line_idx, 1);
        assert_eq!(matches_ci[1].line_idx, 2);
        assert_eq!(matches_ci[2].line_idx, 3);

        // Кириллический поиск без учёта регистра
        let ru_matches = find_all_matches(text, "настройка", false);
        assert_eq!(ru_matches.len(), 1);
        assert_eq!(ru_matches[0].line_idx, 0);
        assert_eq!(ru_matches[0].col_char, 3);

        let replaced_one = replace_match(text, matches_ci[0], "fps_limit");
        assert!(replaced_one.contains("fps_limit 144\r\nfps_max_ui 60"));

        let (replaced_all, count) = replace_all_matches(text, "FPS_MAX", "cap_fps", false);
        assert_eq!(count, 3);
        assert!(!replaced_all.to_lowercase().contains("fps_max"));
        assert!(replaced_all.contains("// Настройка FPS\r\ncap_fps 144\r\ncap_fps_ui 60"));
    }

    #[test]
    fn test_parse_gameinfo_entries_ignores_filesystem_and_reads_commented() {
        let gi = r#""GameInfo"
{
	FileSystem
	{
		SearchPaths
		{
			Game		deadlock
		}
	}
	"RenderSystem"
	{
		"r_low_latency"		"1" // латентность
	}
	"ConVars"
	{
		"r_indirectlighting"		"0"
		//"r_ssao"		"1"
		"rate"
		{
			"default"		"786432"
		}
	}
}"#;

        let entries = parse_config_entries(gi, ConfigKind::GameInfo);
        assert!(!entries.iter().any(|e| e.key == "Game"));
        assert_eq!(entries.len(), 4);

        assert_eq!(entries[0].key, "r_low_latency");
        assert_eq!(entries[0].value, "1");
        assert_eq!(entries[0].section, "RenderSystem");
        assert_eq!(entries[0].inline_comment.as_deref(), Some("латентность"));

        assert_eq!(entries[1].key, "r_indirectlighting");
        assert!(!entries[1].commented);

        assert_eq!(entries[2].key, "r_ssao");
        assert!(entries[2].commented);
        assert_eq!(entries[2].value, "1");

        assert_eq!(entries[3].key, "default");
        assert_eq!(entries[3].section, "ConVars / rate");
    }

    #[test]
    fn test_parse_autoexec_skips_prose_keeps_commands_and_empty_value() {
        let cfg = "// === 1. ПРОИЗВОДИТЕЛЬНОСТЬ ===\n// Просто текстовое пояснение про кадры\nfps_max 0 // без лимита\n// fps_max_ui 60\nvolume \nhost_writeconfig\n";
        let entries = parse_config_entries(cfg, ConfigKind::Autoexec);

        let keys: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, vec!["fps_max", "fps_max_ui", "volume", "host_writeconfig"]);
        assert!(!keys.contains(&"Просто"));
        assert_eq!(entries[0].section, "1. ПРОИЗВОДИТЕЛЬНОСТЬ");
        assert_eq!(entries[0].inline_comment.as_deref(), Some("без лимита"));
        assert!(entries[1].commented);
        assert_eq!(entries[2].value, "");
    }

    #[test]
    fn test_set_line_value_and_toggle_comment_preserve_crlf_and_comments() {
        let gi = "\"GameInfo\"\r\n{\r\n\t\"ConVars\"\r\n\t{\r\n\t\t\"r_shadows\"\t\t\"1\"\t// тени\r\n\t}\r\n}\r\n";
        let edited = set_line_value(gi, ConfigKind::GameInfo, 4, "0");
        assert_eq!(
            edited,
            "\"GameInfo\"\r\n{\r\n\t\"ConVars\"\r\n\t{\r\n\t\t\"r_shadows\"\t\t\"0\"\t// тени\r\n\t}\r\n}\r\n"
        );

        // Очистка значения до пустой строки сохраняет кавычки ""
        let cleared = set_line_value(&edited, ConfigKind::GameInfo, 4, "");
        assert!(cleared.contains("\t\t\"r_shadows\"\t\t\"\"\t// тени\r\n"));
        let reparsed = parse_config_entries(&cleared, ConfigKind::GameInfo);
        assert_eq!(reparsed.len(), 1);
        assert_eq!(reparsed[0].value, "");

        let commented = toggle_line_comment(&edited, 4);
        assert!(commented.contains("\t\t//\"r_shadows\"\t\t\"0\"\t// тени\r\n"));
        let uncommented = toggle_line_comment(&commented, 4);
        assert_eq!(uncommented, edited);
    }

    #[test]
    fn test_insert_command_smart_for_all_kinds() {
        // Autoexec: перед host_writeconfig
        let cfg = "fps_max 0\nhost_writeconfig\n";
        let (cfg_out, cfg_line) =
            insert_command_smart(cfg, ConfigKind::Autoexec, "Autoexec", "rate", "1000000", None);
        assert_eq!(cfg_line, 1);
        assert_eq!(cfg_out, "fps_max 0\nrate 1000000\nhost_writeconfig\n");

        // GameInfo: в конец ConVars
        let gi = "\"GameInfo\"\n{\n\t\"ConVars\"\n\t{\n\t\t\"fps_max\"\t\t\"0\"\n\t}\n}\n";
        let (gi_out, gi_line) =
            insert_command_smart(gi, ConfigKind::GameInfo, "ConVars", "r_ssao", "0", None);
        assert_eq!(gi_line, 5);
        assert!(gi_out.contains("\t\t\"r_ssao\"\t\t\"0\"\n\t}\n"));
        assert!(validate_vdf_braces(&gi_out).balanced);

        // Video: перед закрытием video.cfg
        let vid = "\"video.cfg\"\n{\n\t\"Version\"\t\t\"20\"\n}\n";
        let (vid_out, vid_line) = insert_command_smart(
            vid,
            ConfigKind::Video,
            "video.cfg",
            "setting.r_ssao",
            "0",
            None,
        );
        assert_eq!(vid_line, 3);
        assert!(vid_out.contains("\t\"setting.r_ssao\"\t\t\"0\"\n}\n"));
        assert!(validate_vdf_braces(&vid_out).balanced);
    }

    #[test]
    fn test_validate_vdf_braces_ignores_comments_and_strings() {
        let valid = "\"GameInfo\"\n{\n\t// { лишняя в комментарии\n\t\"path\"\t\t\"{и в строке}\"\n\t\"ConVars\"\n\t{\n\t\t\"a\" \"1\"\n\t}\n}\n";
        let ok = validate_vdf_braces(valid);
        assert!(ok.balanced);
        assert_eq!(ok.open_count, 2);
        assert_eq!(ok.close_count, 2);
        assert_eq!(ok.first_error_line, None);

        let missing_close = "\"GameInfo\"\n{\n\t\"ConVars\"\n\t{\n\t\t\"a\" \"1\"\n}\n";
        let bad = validate_vdf_braces(missing_close);
        assert!(!bad.balanced);
        assert_eq!(bad.open_count, 2);
        assert_eq!(bad.close_count, 1);
        assert_eq!(bad.first_error_line, Some(2));
    }

    #[test]
    fn test_catalog_contains_entries_for_all_kinds() {
        let removed_keys = [
            "r_drawtracers_firstperson",
            "panorama_max_fps",
            "GraphicsPipelineLibrary",
            "r_citadel_outlines",
            "r_citadel_half_res_noisy_effects",
            "r_particle_shadows",
            "minimap_update_rate_hz",
            "setting.r_citadel_motion_blur",
        ];
        for kind in [ConfigKind::GameInfo, ConfigKind::Autoexec, ConfigKind::Video] {
            assert!(!catalog_for(kind).is_empty());
            assert!(!categories_for(kind).is_empty());
            for bad_key in removed_keys {
                assert!(
                    !catalog_for(kind)
                        .iter()
                        .any(|item| item.key.eq_ignore_ascii_case(bad_key)),
                    "Неподтверждённый ключ {bad_key} не должен присутствовать в справочнике Deadlock"
                );
            }
        }
        assert!(describe_command(ConfigKind::GameInfo, "r_aspectratio").is_some());
        assert!(describe_command(ConfigKind::GameInfo, "R_SSAO").is_some());
        assert!(describe_command(ConfigKind::GameInfo, "citadel_boss_glow_disabled").is_some());
        assert!(describe_command(ConfigKind::Autoexec, "rate").is_some());
        assert!(describe_command(ConfigKind::Autoexec, "r_drawdecals").is_some());
        assert!(describe_command(ConfigKind::Video, "setting.r_citadel_shadow_quality").is_some());
        assert!(describe_command(ConfigKind::Video, "setting.mat_vsync").is_some());
    }
}
