use gpui::{AssetSource, SharedString};
use std::borrow::Cow;
pub struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let bytes: Option<&'static [u8]> = match path {
            "brand/tonto.svg" => Some(include_bytes!("../../assets/brand/tonto.svg")),
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
            "library/calendar.svg" => {
                "<rect x='3' y='5' width='18' height='16' rx='2.5'/><path d='M3 10h18M8 3v4M16 3v4'/>"
            }
            "library/image.svg" => {
                "<rect x='3' y='4' width='18' height='16' rx='2.5'/><circle cx='9' cy='10' r='2'/><path d='m21 16-5-5-9 9'/>"
            }
            "library/camera.svg" => {
                "<path d='M4 8h3l2-3h6l2 3h3a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V9a1 1 0 0 1 1-1Z'/><circle cx='12' cy='13.5' r='3.5'/>"
            }
            "library/aperture.svg" => {
                "<circle cx='12' cy='12' r='9'/><path d='m14.3 3.3-5.7 9.9M20.5 9H9.1M17.7 18.6 12 8.7M9.7 20.7l5.7-9.9M3.5 15h11.4M6.3 5.4 12 15.3'/>"
            }
            "library/location.svg" => {
                "<path d='M12 21s-7-6.2-7-11.5a7 7 0 0 1 14 0C19 14.8 12 21 12 21Z'/><circle cx='12' cy='9.5' r='2.5'/>"
            }
            "library/person.svg" => {
                "<circle cx='12' cy='8' r='4'/><path d='M4 21a8 8 0 0 1 16 0'/>"
            }
            "library/explore.svg" => {
                "<circle cx='12' cy='12' r='9'/><path d='m15.5 8.5-2 5-5 2 2-5Z'/>"
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
