//! Шпаргалка таймингов карты Deadlock и интерактивный таймер матча.

use std::time::{Duration, Instant};

/// Объект или событие на карте Deadlock с расписанием появления.
#[derive(Clone, Copy, Debug)]
pub struct MapObjective {
    pub name: &'static str,
    pub category: &'static str,
    pub first_spawn_sec: u64,
    pub respawn_sec: u64,
    pub schedule_hint: &'static str,
    pub note: &'static str,
}

/// Ключевые тайминги объектов Deadlock (линии, руны, лес, Урна Душ и Мид-Босс).
pub const OBJECTIVES: &[MapObjective] = &[
    MapObjective {
        name: "Крипы линий",
        category: "Линии",
        first_spawn_sec: 0,
        respawn_sec: 30,
        schedule_hint: "0:00 → каждые 30 сек",
        note: "Волны выходят синхронно по всем линиям",
    },
    MapObjective {
        name: "Малые нейтралы",
        category: "Лес",
        first_spawn_sec: 2 * 60,
        respawn_sec: 4 * 60,
        schedule_hint: "2:00 → каждые 4 мин",
        note: "Быстрые души в перерывах между волнами",
    },
    MapObjective {
        name: "Руны (Баффы моста)",
        category: "Карта",
        first_spawn_sec: 5 * 60,
        respawn_sec: 5 * 60,
        schedule_hint: "5:00 → каждые 5 мин",
        note: "Усиления на боковых мостах карты",
    },
    MapObjective {
        name: "Средние нейтралы",
        category: "Лес",
        first_spawn_sec: 5 * 60,
        respawn_sec: 6 * 60,
        schedule_hint: "5:00 → каждые 6 мин",
        note: "Окупаются лучше всего к середине стадии линий",
    },
    MapObjective {
        name: "Сильные нейтралы",
        category: "Лес",
        first_spawn_sec: 8 * 60,
        respawn_sec: 8 * 60,
        schedule_hint: "8:00 → каждые 8 мин",
        note: "Крупные лагеря с максимальным числом душ",
    },
    MapObjective {
        name: "Автомат Грешника",
        category: "Лес",
        first_spawn_sec: 8 * 60,
        respawn_sec: 5 * 60,
        schedule_hint: "8:00 → каждые 5 мин",
        note: "В ближнем бою даёт золотые баффы характеристик",
    },
    MapObjective {
        name: "Урна Душ (Soul Urn)",
        category: "Ключевые",
        first_spawn_sec: 10 * 60,
        respawn_sec: 5 * 60,
        schedule_hint: "10:00 → каждые 5 мин",
        note: "После доставки таймер идёт 5 минут до следующей",
    },
    MapObjective {
        name: "Мид-Босс (Rejuvenator)",
        category: "Ключевые",
        first_spawn_sec: 10 * 60,
        respawn_sec: 7 * 60,
        schedule_hint: "10:00 → респавн 7 / 6 / 5 мин",
        note: "Возвращает в строй быстрее и усиливает крипов",
    },
];

/// Интерактивный секундомер матча с возможностью сдвига времени.
#[derive(Clone, Debug, Default)]
pub struct MatchTimerState {
    running: bool,
    accumulated: Duration,
    started_at: Option<Instant>,
}

impl MatchTimerState {
    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn toggle(&mut self) {
        if self.running {
            if let Some(st) = self.started_at.take() {
                self.accumulated = self.accumulated.saturating_add(st.elapsed());
            }
            self.running = false;
        } else {
            self.started_at = Some(Instant::now());
            self.running = true;
        }
    }

    pub fn reset(&mut self) {
        self.running = false;
        self.accumulated = Duration::ZERO;
        self.started_at = None;
    }

    /// Сдвигает текущее время матча (например, чтобы синхронизироваться после старта игры).
    pub fn shift_secs(&mut self, delta: i64) {
        let cur = self.elapsed_secs() as i64;
        let next = (cur + delta).max(0) as u64;
        self.accumulated = Duration::from_secs(next);
        if self.running {
            self.started_at = Some(Instant::now());
        }
    }

    pub fn elapsed_secs(&self) -> u64 {
        let extra = self
            .started_at
            .map(|st| st.elapsed())
            .unwrap_or(Duration::ZERO);
        self.accumulated.saturating_add(extra).as_secs()
    }
}

/// Форматирует секунды в `MM:SS`.
pub fn format_mm_ss(total_secs: u64) -> String {
    format!("{:02}:{:02}", total_secs / 60, total_secs % 60)
}

/// Вычисляет, сколько секунд осталось до следующего появления объекта на карте.
pub fn next_spawn_in_secs(elapsed_secs: u64, first_spawn_sec: u64, interval_sec: u64) -> u64 {
    if elapsed_secs < first_spawn_sec {
        return first_spawn_sec - elapsed_secs;
    }
    if interval_sec == 0 {
        return 0;
    }
    let since_first = elapsed_secs - first_spawn_sec;
    let rem = since_first % interval_sec;
    if rem == 0 {
        0
    } else {
        interval_sec - rem
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_timers_formatting_and_next_spawn() {
        assert_eq!(format_mm_ss(0), "00:00");
        assert_eq!(format_mm_ss(125), "02:05");
        assert_eq!(format_mm_ss(600), "10:00");

        // До первого появления (Урна на 10:00)
        assert_eq!(next_spawn_in_secs(540, 600, 300), 60);
        // Точно в момент спавна
        assert_eq!(next_spawn_in_secs(600, 600, 300), 0);
        // Через 40 секунд после спавна (осталось 260 до следующего)
        assert_eq!(next_spawn_in_secs(640, 600, 300), 260);
        // Руны на 5:00 каждые 5 минут -> в 9:40 осталось 20 сек
        assert_eq!(next_spawn_in_secs(580, 300, 300), 20);
    }
}
