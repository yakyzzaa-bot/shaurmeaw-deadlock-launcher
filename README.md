<div align="center">

# ShaurMeaw — Deadlock Launcher

Лёгкий нативный лаунчер на Rust для управления `gameinfo.gi`, модами и комфортной игры в Deadlock.

[![Latest Release](https://img.shields.io/github/v/release/yakyzzaa-bot/shaurmeaw-deadlock-launcher?style=for-the-badge&color=d4a359)](https://github.com/yakyzzaa-bot/shaurmeaw-deadlock-launcher/releases/latest)
![Windows 10/11](https://img.shields.io/badge/Windows-10%20%2F%2011-0078D6?style=for-the-badge)
![Written in Rust](https://img.shields.io/badge/Written%20in-Rust-DEA584?style=for-the-badge)

**[⬇ Скачать последнюю версию (ShaurMeaw.exe)](https://github.com/yakyzzaa-bot/shaurmeaw-deadlock-launcher/releases/latest)**

</div>

<!-- ![Скриншот лаунчера](assets/preview.png) -->

---

## Основные возможности

- **Каталог профилей `gameinfo.gi` и Smart Merge** — готовые пресеты и умное слияние настроек после каждого патча Valve без ручной правки файлов.
- **Живой Discord Rich Presence** — отображает статус матча, игровой режим и всех актуальных героев Deadlock.
- **Сверхлёгкий фоновый режим в трее** — ~0% нагрузки на ЦП и ~20 МБ ОЗУ в простое, автозапуск и авто-переключение раскладки на EN при входе в Deadlock.
- **Очистка кэша шейдеров и тактические утилиты** — обслуживание игры в один клик и минималистичный интерфейс в эстетике Deadlock.
- **Встроенное автообновление** — проверка новых версий и обновление лаунчера прямо из GitHub Releases в один клик.

---

## Быстрая установка

1. Скачайте **`ShaurMeaw.exe`** со страницы [Releases](https://github.com/yakyzzaa-bot/shaurmeaw-deadlock-launcher/releases/latest).
2. Положите файл в отдельную папку (например, `C:\Games\ShaurMeaw\`) и запустите.
3. Выберите нужный пресет и играйте — дальнейшие обновления будут приходить автоматически.

---

## Безопасность: Windows SmartScreen и VirusTotal

- **Предупреждение Windows SmartScreen:** у open-source проекта нет платной цифровой подписи Microsoft, поэтому при первом запуске Windows может показать синее окно защиты. Нажмите **«Подробнее» → «Выполнить в любом случае»**.
- **Открытая автоматическая сборка:** весь исходный код полностью открыт прямо в этом репозитории, а исполняемый файл `ShaurMeaw.exe` собирается автоматически на чистых серверах GitHub Actions ([`.github/workflows/release.yml`](.github/workflows/release.yml)) без ручных сборок и подмен.
- **Отчёт VirusTotal (v1.0.0):** [посмотреть результаты проверки](https://www.virustotal.com/gui/file/cb642cbf43e0b9cfc51c68b80e15887734c847f5a7735f1ca5acb580f81e0466) — **Microsoft Defender, Kaspersky, Avast, AVG** и остальные основные антивирусы показывают **0 угроз**. Единичные срабатывания эвристических ML-сканеров — стандартный ложный детект на неподписанные бинарники на Rust.
