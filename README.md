# ShaurMeaw — Deadlock Launcher v0.7.0 (Rust)

Кастомный автономный лаунчер для Deadlock на Rust + egui в эстетике **оккультного нуара и Art Deco 1930-х** (палитра Deep Noir Slate `#111315`/`#141619`, состаренная латунь `#D4A359`, эктоплазма `#3ED0A5`, Editorial Serif типографика, карточки-досье и подсветка синтаксиса VDF/CFG) с полной переносимостью между любыми ПК с Windows.

## Возможности

- **Работа на любом ПК с Windows и низкая нагрузка**:
  - Нативный Win32-мониторинг процессов (без спавна `tasklist.exe`), нулевой фоновый I/O вотчера `gameinfo.gi` (`Condvar` + кэш `mtime`), дебаунс записи `settings.json`, экономия RAM (без загрузки 15+ МБ системных символов/эмодзи) и DEFLATE-сжатие встроенных ресурсов (`OnceLock`)
  - Один автономный `.exe` (статическая сборка без Visual C++ Redistributable и внешних MinGW DLL, без встроенных лишних шрифтов — минимальный размер)
  - Встроенный Windows Application Manifest (`PerMonitorV2` DPI, поддержка Windows 7–11, Common Controls v6, запуск без лишнего UAC)
  - Портативный режим (`ShaurMeaw_data\` + `portable.txt` рядом с `.exe` или флаг `--portable`) + переключение в 1 клик во вкладке «Пути»
- **Дизайн-система Deadlock (Art Deco & Occult Noir)**:
  - Боковая навигация с оккультной печатью *The Cursed Apple*, векторными иконками `1.5px` и плавными анимациями `180ms ease-out`
  - Картотека пресетов в виде карточек-досье (дата изменения, источник, бейдж `АКТИВЕН`, скошенные латунные кнопки)
  - Редактор кода «Манускрипт» (`#0E1012`) с кэшируемой подсветкой KeyValues (VDF) и консольных `.cfg` (ключи `#E5B869`, значения `#48D1B5`, комментарии `#686D66`)
- **Автопоиск** Steam и Deadlock на всех дисках `C:\..Z:\`, через реестр (HKCU/HKLM + Uninstall Steam App 1422450), `libraryfolders.vdf` (новый и старый формат) и резервный запуск по `steam://run/1422450`
- **Параметры запуска**: `-novid`, `-console`, `-high`, `-dx11` / `-vulkan`, режим окна, разрешение, потоки CPU, язык, свои флаги
- **Графика и FPS**: пресеты «Максимум FPS / Баланс / Качество», лимит FPS, счётчик FPS, FPS в фоне
- **Конструктор прицела** с живым предпросмотром и переключением фонов (Серый / Тёмный / Светлый / Небо)
- **Бинды** и произвольные консольные команды
- **Менеджер пресетов `autoexec.cfg` (`%APPDATA%\ShaurMeaw\autoexec_presets\`)**:
  - Встроенный ваш конфиг `ShaurMeaw (Мой autoexec)`, чистый `Официальный от Valve (Чистый)`, `Конструктор лаунчера`, импорт `.cfg` и встроенный редактор
- **Менеджер пресетов `video.txt` (`%APPDATA%\ShaurMeaw\video_presets\`)**:
  - Встроенный ваш `ShaurMeaw (Мой video.txt)` и `Официальный от Valve (Стандарт)`, автопоиск по всем Steam-аккаунтам (`userdata\<ID>\1422450\local\cfg\video.txt`) и `game\citadel\cfg\video.txt`, сохранение `VendorID`/`DeviceID` и разрешения другого ПК, бэкапы в `backups/`, сброс для автогенерации Valve и встроенный редактор
- **Каталог пресетов `gameinfo.gi` и автозащита после патчей Deadlock**:
  - Хранение профилей в `%APPDATA%\ShaurMeaw\presets\` (стартовые: `ShaurMeaw (Мой конфиг)`, `Перформанс FPS (OptimizationLock 4.0)`, `Ультра FPS (Без теней и тумана)`, `Официальный от Valve (без модов)`, `Для модов`)
  - Полоса быстрого выбора FOV (`Как в конфиге`, `80`, `85`, `90`, `100`, `110`) и 5 FPS/HUD-твиков (скрытие имён над HP, обводка ×1.5, без вспышек выстрелов, без тряски камеры, оптимизация потоков CPU) поверх любого пресета
  - Встроенный официальный оригинал `gameinfo.gi` Valve + обновление свежего оригинала со SteamDB в 1 клик
  - Применение, создание из текущего файла игры, импорт файла, загрузка/обновление по Raw-ссылке (GitHub Gist), переименование, удаление и встроенный редактор текста
  - Автоматический бэкап с датой и временем в `%APPDATA%\ShaurMeaw\backups\` перед любой перезаписью + сохранение оригинала в `gameinfo_vanilla_backup.gi`
  - Проверка SHA-256 / времени модификации / Steam `buildid` при старте и в фоне + 3-way KeyValues Smart Merge при патче Valve (сохраняет свежий `PGIVersion` и новые настройки Valve, аккуратно перенося отличия активного профиля)
  - Тихий режим синхронизации (`--sync-gameinfo`) и прямой быстрый запуск через ярлык `Deadlock (ShaurMeaw)` на Рабочем столе (`--play`)
- **Утилиты**:
  - Профили настроек (сохранение, загрузка, удаление) + экспорт/импорт `.json`
  - Настройка громкости (`volume`), чувствительности мыши (`sensitivity`) и телеметрии HUD Source 2 (FPS / пинг / потеря пакетов)
  - Подсчёт и очистка кэша шейдеров Deadlock и краш-дампов (`*.mdmp`), откат «Вернуть всё как у Valve» для `gameinfo.gi` / `autoexec.cfg` / `video.txt`
  - Инспектор модов `game\citadel\addons\*.vpk` (подсчёт конфликтов `.dmm-conflicts.json`, быстрое отключение/включение `.vpk` <-> `.vpk.disabled`)
  - Восстановление резервной копии `autoexec.cfg.bak` в 1 клик
  - Быстрые ссылки на ресурсы сообщества (Tracklock, Deadlock Tracker, Deadlock Wiki, SteamDB)
- **Актуальная поддержка Deadlock (ноябрь 2026: *City Never Sleeps*, *Ranked Beta Season 2*, Build 26764805 / 6864)**:
  - **Discord Rich Presence** (`src/discord_rpc.rs`) с нативным IPC-подключением и живым парсером `console.log`: все режимы конца 2026 (`Hideout`, `Sandbox / Explore NYC`, `Pregame Vote`, `Ranked Beta Season 2`, `The Cursed Apple`, `Street Brawl`), все **44 героя** и отображение активного пресета `gameinfo.gi`
  - **Трекинг процесса и автоприоритет**: отслеживание `deadlock.exe` (в приоритете) и `project8.exe` (fallback) через Toolhelp32 + автоматическое выставление `HIGH_PRIORITY_CLASS` (`SetPriorityClass`)
  - **Умная очистка кэша шейдеров (Vulkan / DX11)**: проверка даты обновления `shaders_pc_000.vpk` / `shaders_vulkan_000.vpk` и безопасная очистка `Steam\steamapps\shadercache\1422450` (заблокирована, пока запущена игра)
- **Живой мониторинг Steam и Deadlock** в боковой панели + кнопка принудительного закрытия игры, если она зависла
- **Единая вкладка «Настройки»**:
  - Фильтр по разделам (`Все разделы`, `Интерфейс`, `Запуск и ярлык`, `Конфиги и защита`, `Графика и геймплей`, `Прицел`, `Пути и профили`)
  - Настройки интерфейса: масштаб UI (`0.85–1.25`), размер шрифта редактора кода (`11–17 px`), включение/отключение плавных анимаций и выбор оккультной палитры
  - Сводное управление параметрами запуска, яркостью ярлыка на Рабочем столе, автозащитой `gameinfo.gi` и `autoexec.cfg`, графикой, телеметрией HUD, прицелом, путями, портативным режимом и профилями настроек в одном месте
- **5 оттенков оккультной палитры** (`Оккультная латунь`, `Старинная бронза`, `Янтарное золото`, `Патина душ`, `Оккультный бордо`) и встроенная иконка
- **Генерация `autoexec.cfg`** с предпросмотром и бэкапом старого файла в `autoexec.cfg.bak`
- Автосохранение настроек в `%APPDATA%\ShaurMeaw\settings.json`

## Структура

```
assets/
  icon.ico                 — иконка Windows (.exe)
  icon_128.rgba            — иконка окна и аватарка в интерфейсе
  app.manifest             — Windows-манифест (PerMonitorV2 DPI, Windows 7–11, asInvoker)
  app.rc                   — ресурс Windows (иконка, манифест и свойства файла)
  shaurmeaw_preset.gi      — базовый пресет ShaurMeaw для gameinfo.gi
  performance_fps_preset.gi — пресет Перформанс FPS (OptimizationLock 4.0)
  ultra_fps_preset.gi      — пресет Ультра FPS (без теней, тумана и постобработки)
  shaurmeaw_autoexec.cfg   — ваш встроенный autoexec.cfg
  shaurmeaw_video.txt      — ваш встроенный video.txt ("video.cfg")
  valve_official_video.txt — стандартный официальный video.txt от Valve
  valve_official_gameinfo.gi — официальный gameinfo.gi Valve (SteamDB)
build.rs                   — DEFLATE-сжатие встроенных ресурсов и компиляция Windows-ресурсов через windres.exe
src/
  main.rs                  — точка входа, селективная инициализация подсистем, каскад OpenGL, перехват паник, --sync-gameinfo
  embedded.rs              — ленивая распаковка DEFLATE-сжатых ресурсов через OnceLock
  theme.rs                 — дизайн-система Art Deco (токены, шрифты, иконки, карточки, подсветка VDF/CFG)
  app.rs                   — интерфейс (вкладки, системные шрифты Windows)
  settings.rs              — настройки, темы, портативный режим (ShaurMeaw_data), сохранение в JSON
  autoexec_presets.rs      — каталог и менеджер пресетов autoexec.cfg
  video_presets.rs         — каталог и менеджер пресетов video.txt (userdata + cfg)
  config_gen.rs            — генерация autoexec.cfg (включая -condebug) и строки запуска
  discord_rpc.rs           — нативный Discord Rich Presence (IPC Named Pipe, парсер console.log, 44 героя)
  gameinfo_presets.rs      — каталог пресетов gameinfo.gi, Raw-ссылки (нативный WinHTTP) и backups/
  gameinfo_sync.rs         — SHA-256, фоновый наблюдатель и Smart Merge KeyValues
  launcher.rs              — запись autoexec, запуск через Steam / steam:// / напрямую
  steam.rs                 — поиск Steam и Deadlock на всех дисках и в реестре
  tools.rs                 — мониторинг/завершение процессов (Toolhelp32/TerminateProcess), ярлыки .lnk (COM IShellLinkW), кэш, дампы и профили
```

## Запуск на любом ПК и портативный режим

1. Просто скопируйте **`ShaurMeaw.exe`** на любой ПК с Windows (установка и Visual C++ Redistributable не требуются).
2. При первом запуске лаунчер сам найдёт Steam и Deadlock на любом диске (`C:`..`Z:`) и применит подходящий кириллический шрифт.
3. Если на ПК старый или повреждённый драйвер OpenGL, лаунчер сам переключится на совместимый режим OpenGL (`Gl120`) или программный рендер.

Поддерживаемые ключи запуска `.exe`:
- `--portable` — хранить настройки, пресеты и бэкапы в папке `ShaurMeaw_data\` рядом с `ShaurMeaw.exe` (также включается кнопкой во вкладке «Пути» или созданием пустого файла `portable.txt` рядом с `.exe`).
- `--gl120` — принудительно начать с совместимого режима OpenGL 1.2/2.x (`Gl120`) для старых видеокарт.
- `--software` — принудительно запустить в программном режиме (полезно для удалённого рабочего стола или виртуальных машин).
- `--play` — быстро обновить/адаптировать активные конфиги и сразу запустить Deadlock без открытия окна (используется ярлыком `Deadlock (ShaurMeaw)` на Рабочем столе).
- `--sync-gameinfo` — тихая проверка и адаптация `gameinfo.gi` без открытия окна.

## Как собрать и запустить

### Готовый файл
Собранный лаунчер находится по пути:
`C:\DeadlockLauncher\ShaurMeaw.exe`

Его можно запускать двойным кликом в Проводнике (или создать ярлык на Рабочем столе).

### Пересборка (если измените код)
Используется лёгкий GNU-тулчейн Rust в `C:\rustup` и утилиты MinGW (`dlltool`/`as`/`windres`) в `C:\WinLibs` (Visual Studio Build Tools не требуются), а артефакты сборки складываются на диск `C:\` (`C:\DeadlockLauncher\target`).

Самый простой способ пересобрать — дважды кликнуть по файлу **`rebuild.cmd`** в корне проекта.

Или вручную в PowerShell:
```powershell
$env:RUSTUP_HOME = "C:\rustup"
$env:CARGO_TARGET_DIR = "C:\DeadlockLauncher\target"
$env:RUSTFLAGS = "-C link-self-contained=yes"
$env:PATH = "C:\WinLibs\mingw64\bin;$env:PATH"
cd E:\Users\ShaurMeaw\.gemini\antigravity\scratch\deadlock-launcher

# релизная сборка без окна консоли
cargo +stable-x86_64-pc-windows-gnu build --release --offline
Copy-Item -Force C:\DeadlockLauncher\target\release\ShaurMeaw.exe C:\DeadlockLauncher\ShaurMeaw.exe
```

## Публикация релиза, подпись кода и SmartScreen

- Для подготовки чистого дистрибутива используйте скрипт `scripts/package_release.ps1`: он собирает архив `ShaurMeaw-v<версия>-win64.zip`, считает контрольные суммы `SHA256SUMS.txt` и при наличии сертификата Code Signing (`-CertPfx` или `-CertThumbprint`) подписывает `ShaurMeaw.exe` через `signtool.exe` (SHA256 + RFC 3161 метка времени).
- Предупреждения браузеров и синее окно **SmartScreen «Неизвестный издатель»** при скачивании нового `.exe` полностью снимаются только **доверенной цифровой подписью Authenticode** (например, Microsoft Azure Trusted Signing, коммерческий сертификат или бесплатная подпись через SignPath Foundation для open-source проектов на GitHub), отправкой хеша/файла в **Microsoft Security Intelligence (WDSI)** и раздачей релиза через **GitHub Releases** в виде `.zip` + `SHA256SUMS.txt`.

## Важно

- `autoexec.cfg` подхватывается игрой только с параметром `+exec autoexec` — лаунчер добавляет его сам.
- Deadlock ещё в разработке, и Valve может переименовывать консольные команды. Неизвестные команды игра просто игнорирует.
- Параметры запуска, прописанные в Steam (Свойства → Параметры запуска), тоже применяются.
