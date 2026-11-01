use gpui::{AssetSource, SharedString};
use std::borrow::Cow;
pub struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let bytes: Option<&'static [u8]> = match path {
            "providers/immich.svg" => Some(include_bytes!("../../assets/providers/immich.svg")),
            "providers/nextcloud.svg" => {
                Some(include_bytes!("../../assets/providers/nextcloud.svg"))
            }
            "providers/apple-photos.svg" => {
                Some(include_bytes!("../../assets/providers/apple-photos.svg"))
            }
            _ => None,
        };
        if let Some(bytes) = bytes {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        desert_ui::library::LibraryAssets.load(path)
    }
    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        desert_ui::library::LibraryAssets.list(path)
    }
}
