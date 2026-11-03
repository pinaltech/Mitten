use gpui::{AssetSource, SharedString};
use std::borrow::Cow;
pub struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let bytes: Option<&'static [u8]> = match path {
            "photo-heart.svg" => Some(include_bytes!("../../assets/photo-heart.svg")),
            "photo-heart-filled.svg" => Some(include_bytes!("../../assets/photo-heart-filled.svg")),
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
        // Glyphs Photos adds to the shared `library/` set, drawn the same way.
        let shape = match path {
            "library/video.svg" => {
                "<rect x='2' y='6' width='14' height='12' rx='2.5'/><path d='m16 10 6-3.5v11L16 14'/>"
            }
            "library/play.svg" => "<path d='M7 4.5v15l12.5-7.5Z' fill='currentColor'/>",
            "library/chevrons.svg" => "<path d='m7 9 5-5 5 5M7 15l5 5 5-5'/>",
            "library/plus-circle.svg" => {
                "<circle cx='12' cy='12' r='9'/><path d='M12 8v8M8 12h8'/>"
            }
            _ => "",
        };
        if !shape.is_empty() {
            return Ok(Some(Cow::Owned(format!("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='currentColor' stroke-width='1.6' stroke-linecap='round' stroke-linejoin='round'>{shape}</svg>").into_bytes())));
        }
        desert_ui::library::LibraryAssets.load(path)
    }
    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        desert_ui::library::LibraryAssets.list(path)
    }
}
