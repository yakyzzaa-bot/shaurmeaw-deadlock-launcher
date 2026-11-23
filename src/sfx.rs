//! Лёгкий синтезатор коротких звуков интерфейса через встроенный WinMM (`PlaySoundW`).
//!
//! Не требует сторонних аудио-крейтов и не увеличивает вес бинарника:
//! формирует короткие (20–55 мс) 16-битные моно WAV-буферы в памяти.

use std::sync::Mutex;

/// Типы коротких тактильных звуков интерфейса.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UiSound {
    Click,
    Toggle,
    Launch,
    Success,
    Error,
}

const SAMPLE_RATE: u32 = 22_050;

static ACTIVE_WAV: Mutex<Vec<u8>> = Mutex::new(Vec::new());
static WAV_CACHE: Mutex<[Option<(u8, Vec<u8>)>; 5]> = Mutex::new([const { None }; 5]);

impl UiSound {
    fn index(self) -> usize {
        match self {
            Self::Click => 0,
            Self::Toggle => 1,
            Self::Launch => 2,
            Self::Success => 3,
            Self::Error => 4,
        }
    }
}

/// Воспроизводит звук интерфейса с заданной громкостью (`0.0..=1.0`).
pub fn play(kind: UiSound, enabled: bool, volume: f32) {
    if !enabled || volume <= 0.01 {
        return;
    }
    let clamped = volume.clamp(0.0, 1.0);
    let vol_pct = (clamped * 100.0).round() as u8;
    let wav = {
        let mut cache = WAV_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        let slot = &mut cache[kind.index()];
        if let Some((cached_vol, ref bytes)) = slot {
            if *cached_vol == vol_pct {
                bytes.clone()
            } else {
                let built = build_wav_bytes(kind, clamped);
                *slot = Some((vol_pct, built.clone()));
                built
            }
        } else {
            let built = build_wav_bytes(kind, clamped);
            *slot = Some((vol_pct, built.clone()));
            built
        }
    };
    play_wav_memory(wav);
}

/// Генерирует валидный PCM WAV (16-bit, mono, 22050 Hz) для выбранного события UI.
pub fn build_wav_bytes(kind: UiSound, volume: f32) -> Vec<u8> {
    let vol = volume.clamp(0.0, 1.0);
    let (dur_ms, f_start, f_end, harm_ratio): (u32, f32, f32, f32) = match kind {
        UiSound::Click => (22, 420.0, 310.0, 0.25),
        UiSound::Toggle => (28, 340.0, 520.0, 0.20),
        UiSound::Launch => (55, 330.0, 495.0, 0.45),
        UiSound::Success => (45, 440.0, 660.0, 0.30),
        UiSound::Error => (45, 220.0, 150.0, 0.35),
    };

    let sample_count = ((SAMPLE_RATE * dur_ms) / 1000).max(1) as usize;
    let data_bytes = (sample_count * 2) as u32;
    let riff_size = 36 + data_bytes;

    let mut out = Vec::with_capacity(44 + sample_count * 2);
    // Заголовок RIFF / WAVEfmt / data
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // размер fmt-чанка (PCM)
    out.extend_from_slice(&1u16.to_le_bytes()); // формат PCM = 1
    out.extend_from_slice(&1u16.to_le_bytes()); // 1 канал (моно)
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());

    let mut phase = 0.0_f32;
    let max_amp = 4_200.0_f32 * vol;
    for i in 0..sample_count {
        let t = i as f32 / sample_count as f32;
        // Мягкая атака на первых 8% и экспоненциальное затухание без щелчков
        let attack = (t / 0.08).min(1.0);
        let decay = (-4.2 * t).exp();
        let env = attack * decay * (1.0 - t * 0.25);

        let freq = f_start + (f_end - f_start) * t;
        phase += std::f32::consts::TAU * freq / SAMPLE_RATE as f32;
        let base = phase.sin();
        let overtone = (phase * 1.5).sin() * harm_ratio;
        let sample = ((base + overtone) * env * max_amp).clamp(-32_000.0, 32_000.0) as i16;
        out.extend_from_slice(&sample.to_le_bytes());
    }

    out
}

#[cfg(windows)]
fn play_wav_memory(wav: Vec<u8>) {
    use std::ffi::c_void;

    const SND_ASYNC: u32 = 0x0001;
    const SND_NODEFAULT: u32 = 0x0002;
    const SND_MEMORY: u32 = 0x0004;

    #[link(name = "winmm")]
    extern "system" {
        fn PlaySoundW(psz_sound: *const u16, hmod: *mut c_void, fdw_sound: u32) -> i32;
    }

    let mut guard = ACTIVE_WAV.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        // Останавливаем прежний звук перед заменой буфера в памяти (защита от UAF)
        let _ = PlaySoundW(std::ptr::null(), std::ptr::null_mut(), 0);
        *guard = wav;
        let _ = PlaySoundW(
            guard.as_ptr().cast::<u16>(),
            std::ptr::null_mut(),
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        );
    }
}

#[cfg(not(windows))]
fn play_wav_memory(wav: Vec<u8>) {
    let mut guard = ACTIVE_WAV.lock().unwrap_or_else(|e| e.into_inner());
    *guard = wav;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sfx_generates_valid_wav_header_and_scales_volume() {
        let loud = build_wav_bytes(UiSound::Launch, 1.0);
        let quiet = build_wav_bytes(UiSound::Launch, 0.2);

        assert!(loud.len() > 44);
        assert_eq!(loud.len(), quiet.len());
        assert_eq!(&loud[0..4], b"RIFF");
        assert_eq!(&loud[8..12], b"WAVE");
        assert_eq!(&loud[12..16], b"fmt ");
        assert_eq!(&loud[36..40], b"data");

        let peak = |buf: &[u8]| -> i16 {
            buf[44..]
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]).saturating_abs())
                .max()
                .unwrap_or(0)
        };
        assert!(peak(&loud) > peak(&quiet));
        assert!(peak(&quiet) > 0);
    }
}
