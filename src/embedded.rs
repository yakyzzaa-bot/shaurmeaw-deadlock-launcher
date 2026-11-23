//! Ленивая распаковка встроенных ресурсов, сжатых на этапе сборки (`DEFLATE`).

use std::sync::OnceLock;

fn inflate_bytes(compressed: &[u8]) -> Vec<u8> {
    miniz_oxide::inflate::decompress_to_vec(compressed).unwrap_or_default()
}

fn inflate_str(compressed: &[u8]) -> String {
    let bytes = inflate_bytes(compressed);
    match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    }
}

macro_rules! embedded_str {
    ($fn_name:ident, $doc:literal, $file:literal) => {
        #[doc = $doc]
        pub fn $fn_name() -> &'static str {
            static CACHE: OnceLock<String> = OnceLock::new();
            CACHE
                .get_or_init(|| {
                    inflate_str(include_bytes!(concat!(env!("OUT_DIR"), "/", $file)))
                })
                .as_str()
        }
    };
}

/// Встроенный ресурс иконки 128×128 RGBA (распаковывается однократно при первом обращении).
pub fn icon_128_rgba() -> &'static [u8] {
    const EXPECTED_LEN: usize = 128 * 128 * 4;
    static CACHE: OnceLock<Vec<u8>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let decoded = inflate_bytes(include_bytes!(concat!(
                env!("OUT_DIR"),
                "/icon_128.rgba.deflate"
            )));
            if decoded.len() == EXPECTED_LEN {
                decoded
            } else {
                vec![0u8; EXPECTED_LEN]
            }
        })
        .as_slice()
}

embedded_str!(
    valve_vanilla_gi,
    "Официальный оригинал gameinfo.gi Valve.",
    "valve_official_gameinfo.gi.deflate"
);
embedded_str!(
    shaurmeaw_gi,
    "Встроенный профиль ShaurMeaw gameinfo.gi.",
    "shaurmeaw_preset.gi.deflate"
);
embedded_str!(
    performance_fps_gi,
    "Встроенный профиль Перформанс FPS gameinfo.gi.",
    "performance_fps_preset.gi.deflate"
);
embedded_str!(
    ultra_fps_gi,
    "Встроенный профиль Ультра FPS gameinfo.gi.",
    "ultra_fps_preset.gi.deflate"
);
embedded_str!(
    shaurmeaw_autoexec,
    "Встроенный autoexec.cfg ShaurMeaw.",
    "shaurmeaw_autoexec.cfg.deflate"
);
embedded_str!(
    shaurmeaw_video,
    "Встроенный video.txt ShaurMeaw.",
    "shaurmeaw_video.txt.deflate"
);
embedded_str!(
    valve_video,
    "Официальный стандартный video.txt Valve.",
    "valve_official_video.txt.deflate"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedded_assets_inflate_cleanly() {
        assert_eq!(icon_128_rgba(), include_bytes!("../assets/icon_128.rgba"));
        assert_eq!(
            valve_vanilla_gi(),
            include_str!("../assets/valve_official_gameinfo.gi")
        );
        assert_eq!(shaurmeaw_gi(), include_str!("../assets/shaurmeaw_preset.gi"));
        assert_eq!(
            performance_fps_gi(),
            include_str!("../assets/performance_fps_preset.gi")
        );
        assert_eq!(ultra_fps_gi(), include_str!("../assets/ultra_fps_preset.gi"));
        assert_eq!(
            shaurmeaw_autoexec(),
            include_str!("../assets/shaurmeaw_autoexec.cfg")
        );
        assert_eq!(shaurmeaw_video(), include_str!("../assets/shaurmeaw_video.txt"));
        assert_eq!(
            valve_video(),
            include_str!("../assets/valve_official_video.txt")
        );
    }
}
