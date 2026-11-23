//! Безопасные файловые операции: атомарная запись, retry при блокировках Steam/ОС и не-UTF8 чтение.

use std::fs::{self, File};
use std::io::{BufReader, ErrorKind, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

static TMP_SEQ: AtomicU64 = AtomicU64::new(1);
const RETRY_DELAYS_MS: [u64; 4] = [150, 250, 350, 450];
const MAX_FILENAME_CHARS: usize = 100;
const MAX_PREALLOC_BYTES: u64 = 64 * 1024 * 1024;
pub const IO_BUF_CAPACITY: usize = 32 * 1024;
pub const STREAM_CHUNK_SIZE: usize = 64 * 1024;

/// Снимает атрибут `Только чтение` (Read-Only), если файл существует и доступен.
pub fn clear_readonly(path: &Path) {
    if let Ok(meta) = fs::metadata(path) {
        let mut perms = meta.permissions();
        if perms.readonly() {
            perms.set_readonly(false);
            let _ = fs::set_permissions(path, perms);
        }
    }
}

fn is_transient_lock_err(err: &std::io::Error) -> bool {
    if err.kind() == ErrorKind::WouldBlock {
        return true;
    }
    #[cfg(windows)]
    {
        // ERROR_ACCESS_DENIED (5), ERROR_SHARING_VIOLATION (32), ERROR_LOCK_VIOLATION (33)
        if matches!(err.raw_os_error(), Some(5 | 32 | 33)) {
            return true;
        }
    }
    false
}

/// Открывает файл на чтение с повторными попытками при временной блокировке Steam/ОС.
pub fn open_file_with_retry(path: &Path) -> std::io::Result<File> {
    for attempt in 0..=RETRY_DELAYS_MS.len() {
        match File::open(path) {
            Ok(file) => return Ok(file),
            Err(e) if attempt < RETRY_DELAYS_MS.len() && is_transient_lock_err(&e) => {
                thread::sleep(Duration::from_millis(RETRY_DELAYS_MS[attempt]));
            }
            Err(e) => return Err(e),
        }
    }
    File::open(path)
}

/// Читает байты файла в переиспользуемый буфер через `BufReader` с retry и предварительным `reserve`.
pub fn read_bytes_into_with_retry(path: &Path, out: &mut Vec<u8>) -> std::io::Result<()> {
    for attempt in 0..=RETRY_DELAYS_MS.len() {
        let res = (|| -> std::io::Result<()> {
            let file = File::open(path)?;
            out.clear();
            if let Ok(meta) = file.metadata() {
                let target = meta.len().min(MAX_PREALLOC_BYTES) as usize;
                if out.capacity() < target {
                    out.reserve(target - out.capacity());
                }
            }
            let mut reader = BufReader::with_capacity(IO_BUF_CAPACITY, file);
            reader.read_to_end(out)?;
            Ok(())
        })();
        match res {
            Ok(()) => return Ok(()),
            Err(e) if attempt < RETRY_DELAYS_MS.len() && is_transient_lock_err(&e) => {
                thread::sleep(Duration::from_millis(RETRY_DELAYS_MS[attempt]));
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!("Цикл retry всегда завершается return")
}

/// Читает байты файла с повторными попытками при временной блокировке (например, обновлении Steam).
pub fn read_bytes_with_retry(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    read_bytes_into_with_retry(path, &mut out)?;
    Ok(out)
}

/// Читает текстовый файл в переиспользуемые буферы `byte_buf` и `text_buf` без лишних аллокаций.
pub fn read_text_lossy_into_with_retry(
    path: &Path,
    byte_buf: &mut Vec<u8>,
    text_buf: &mut String,
) -> std::io::Result<()> {
    read_bytes_into_with_retry(path, byte_buf)?;
    text_buf.clear();
    match std::str::from_utf8(byte_buf) {
        Ok(valid) => text_buf.push_str(valid),
        Err(_) => text_buf.push_str(&String::from_utf8_lossy(byte_buf)),
    }
    Ok(())
}

/// Читает текстовый файл с retry и безопасным восстановлением не-UTF8 символов (без падения на CP1251/битых логах).
pub fn read_text_lossy_with_retry(path: &Path) -> std::io::Result<String> {
    let mut byte_buf = Vec::new();
    let mut text_buf = String::new();
    read_text_lossy_into_with_retry(path, &mut byte_buf, &mut text_buf)?;
    Ok(text_buf)
}

/// Атомарно записывает файл через временный `.tmp` и `fs::rename` с retry при блокировках Steam/антивируса.
pub fn atomic_write_with_retry(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if parent != Path::new(".") {
        fs::create_dir_all(parent)?;
    }

    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_path = parent.join(format!("{file_name}.shaurmeaw_{}_{seq}.tmp", std::process::id()));

    let write_res = (|| -> std::io::Result<()> {
        let mut f = File::create(&tmp_path)?;
        f.write_all(data)?;
        f.flush()?;
        Ok(())
    })();
    if let Err(e) = write_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }

    for attempt in 0..=RETRY_DELAYS_MS.len() {
        clear_readonly(path);
        match fs::rename(&tmp_path, path) {
            Ok(()) => return Ok(()),
            Err(e) if attempt < RETRY_DELAYS_MS.len() && is_transient_lock_err(&e) => {
                thread::sleep(Duration::from_millis(RETRY_DELAYS_MS[attempt]));
            }
            Err(e) => {
                let _ = fs::remove_file(&tmp_path);
                return Err(e);
            }
        }
    }

    let _ = fs::remove_file(&tmp_path);
    Ok(())
}

fn is_reserved_windows_device(stem: &str) -> bool {
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

/// Нормализует и проверяет безопасное имя файла для профилей и пресетов Windows.
pub fn sanitize_filename(raw: &str) -> Result<String, String> {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    let capped: String = trimmed.chars().take(MAX_FILENAME_CHARS).collect();
    let clean = capped.trim().trim_matches('.').trim().to_owned();
    if clean.is_empty() {
        return Err("Введите корректное название".into());
    }
    let device_stem = clean.split('.').next().unwrap_or(&clean).trim();
    if is_reserved_windows_device(device_stem) {
        return Err(format!("Имя «{clean}» зарезервировано системой Windows"));
    }
    Ok(clean)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_atomic_write_replaces_readonly_and_reads_lossy() {
        let root = std::env::temp_dir().join(format!(
            "shaurmeaw_fs_safe_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();

        let target = root.join("конфиг с пробелами.gi");
        fs::write(&target, b"old").unwrap();
        let mut perms = fs::metadata(&target).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&target, perms).unwrap();

        let payload = [b'O', b'K', b'\n', 0xC0, 0xC1, b'\n'];
        atomic_write_with_retry(&target, &payload).unwrap();

        assert_eq!(read_bytes_with_retry(&target).unwrap(), payload);
        let lossy = read_text_lossy_with_retry(&target).unwrap();
        assert!(lossy.starts_with("OK\n"));
        assert!(lossy.contains('\u{FFFD}'));

        let leftover_tmp = fs::read_dir(&root)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().ends_with(".tmp"));
        assert!(!leftover_tmp, "Временный .tmp файл не должен оставаться");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_sanitize_filename_blocks_reserved_windows_names() {
        assert!(sanitize_filename("CON").is_err());
        assert!(sanitize_filename("nul").is_err());
        assert!(sanitize_filename("COM1.txt").is_err());
        assert!(sanitize_filename("  ...  ").is_err());
        assert_eq!(sanitize_filename("Мой пресет 2026\r\n").unwrap(), "Мой пресет 2026");
        let long_name = "А".repeat(140);
        assert_eq!(sanitize_filename(&long_name).unwrap().chars().count(), MAX_FILENAME_CHARS);
    }

    #[test]
    fn test_read_into_reuses_buffers_without_shrinking() {
        let root = std::env::temp_dir().join(format!(
            "shaurmeaw_fs_buf_reuse_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();

        let big_file = root.join("big.txt");
        let small_file = root.join("small.txt");
        let big_payload = "A".repeat(48 * 1024);
        fs::write(&big_file, &big_payload).unwrap();
        fs::write(&small_file, b"hello").unwrap();

        let mut bytes = Vec::new();
        let mut text = String::new();
        read_text_lossy_into_with_retry(&big_file, &mut bytes, &mut text).unwrap();
        let cap_after_big = bytes.capacity();
        let ptr_after_big = bytes.as_ptr();
        assert_eq!(text.len(), big_payload.len());

        read_text_lossy_into_with_retry(&small_file, &mut bytes, &mut text).unwrap();
        assert_eq!(text, "hello");
        assert_eq!(bytes.capacity(), cap_after_big);
        assert_eq!(bytes.as_ptr(), ptr_after_big);

        let _ = fs::remove_dir_all(&root);
    }
}
