//! Offline screenshots from Android's official documentation. These are real
//! reference screens, never a claim about the currently attached phone's state.
pub fn image_asset(path: &str) -> Option<&'static [u8]> {
    Some(match path {
        "connection-guide/build-number.png" => include_bytes!("../assets/connection-guide/build-number.png"),
        "connection-guide/usb-debugging.png" => include_bytes!("../assets/connection-guide/usb-debugging.png"),
        "connection-guide/allow-usb.png" => include_bytes!("../assets/connection-guide/allow-usb.png"),
        "connection-guide/wireless-setting.png" => include_bytes!("../assets/connection-guide/wireless-setting.png"),
        _ => return None,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn official_reference_screens_decode_without_remote_loading() {
        for name in ["build-number", "usb-debugging", "allow-usb", "wireless-setting"] {
            let bytes = image_asset(&format!("connection-guide/{name}.png")).unwrap();
            assert!(bytes.len() < 512 * 1024);
            let im = image::load_from_memory(bytes).unwrap();
            assert!(im.width() >= 300 && im.height() >= 120);
        }
        assert!(image_asset("../../private.png").is_none());
        assert!(image_asset("https://example.org/image.png").is_none());
        let sources: serde_json::Value = serde_json::from_str(include_str!("../assets/connection-guide/sources.json")).unwrap();
        assert_eq!(sources["author"], "Google / Android Developers");
        assert_eq!(sources["license"], "CC BY 4.0");
        assert_eq!(sources["images"].as_object().unwrap().len(), 4);
    }
}
