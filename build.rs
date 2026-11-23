use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const ASSETS_TO_COMPRESS: [(&str, &str); 8] = [
    ("assets/icon_128.rgba", "icon_128.rgba.deflate"),
    (
        "assets/valve_official_gameinfo.gi",
        "valve_official_gameinfo.gi.deflate",
    ),
    ("assets/shaurmeaw_preset.gi", "shaurmeaw_preset.gi.deflate"),
    (
        "assets/performance_fps_preset.gi",
        "performance_fps_preset.gi.deflate",
    ),
    ("assets/ultra_fps_preset.gi", "ultra_fps_preset.gi.deflate"),
    (
        "assets/shaurmeaw_autoexec.cfg",
        "shaurmeaw_autoexec.cfg.deflate",
    ),
    ("assets/shaurmeaw_video.txt", "shaurmeaw_video.txt.deflate"),
    (
        "assets/valve_official_video.txt",
        "valve_official_video.txt.deflate",
    ),
];

/// Сжимает встроенные ассеты алгоритмом DEFLATE (уровень 10) в `OUT_DIR` для уменьшения `.exe`.
fn compress_embedded_assets(out_dir: &PathBuf) {
    for (src, dst) in ASSETS_TO_COMPRESS {
        println!("cargo:rerun-if-changed={src}");
        let raw = fs::read(src).unwrap_or_else(|e| panic!("Не удалось прочитать {src}: {e}"));
        let compressed = miniz_oxide::deflate::compress_to_vec(&raw, 10);
        let dst_path = out_dir.join(dst);
        fs::write(&dst_path, &compressed)
            .unwrap_or_else(|e| panic!("Не удалось записать {}: {e}", dst_path.display()));
    }
}

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR не задан"));
    compress_embedded_assets(&out_dir);

    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/app.manifest");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    if target_os == "windows" && target_env == "gnu" {
        let obj = out_dir.join("app_icon.o");

        let windres = if PathBuf::from(r"C:\WinLibs\mingw64\bin\windres.exe").exists() {
            r"C:\WinLibs\mingw64\bin\windres.exe"
        } else {
            "windres.exe"
        };

        let status = Command::new(windres)
            .args([
                "--use-temp-file",
                "--include-dir",
                "assets",
                "--input",
                "assets/app.rc",
                "--input-format",
                "rc",
                "--output",
                obj.to_str().expect("некорректный путь OUT_DIR"),
                "--output-format",
                "coff",
                "--target",
                "pe-x86-64",
            ])
            .status()
            .expect("Не удалось запустить windres.exe для встраивания иконки");

        assert!(
            status.success(),
            "windres.exe завершился с ошибкой: {status}"
        );

        println!("cargo:rustc-link-arg-bin=ShaurMeaw={}", obj.display());
    }
}
