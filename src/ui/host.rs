//! The site around Mitten when it runs in a web page (its `host.js`, see
//! web/README.md): branding, whether people manage libraries themselves, the
//! topbar's height and room kept for the site's own controls. On the desktop
//! everything is Mitten's own defaults.
use gpui::SharedString;
use std::sync::OnceLock;

pub struct Host {
    /// The topbar's logo (a Mitten asset path) and name.
    pub brand_icon: SharedString,
    pub brand_name: SharedString,
    /// The library switcher and library settings. Off where the site
    /// serves one library of its own.
    pub libraries: bool,
    pub topbar_height: f32,
    /// Where to go when the photo service ends the session, and to sign out.
    #[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
    pub sign_in: Option<String>,
    #[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
    pub sign_out: Option<String>,
}

impl Default for Host {
    fn default() -> Self {
        Self {
            brand_icon: "brand/mitten.svg".into(),
            brand_name: "Mitten".into(),
            libraries: true,
            topbar_height: desert_ui::metrics::TITLEBAR,
            sign_in: None,
            sign_out: None,
        }
    }
}

static HOST: OnceLock<Host> = OnceLock::new();

pub fn get() -> &'static Host {
    HOST.get_or_init(|| {
        #[cfg(target_family = "wasm")]
        {
            super::browser::host()
        }
        #[cfg(not(target_family = "wasm"))]
        {
            Host::default()
        }
    })
}

/// Room kept free at the topbar's trailing edge, in pixels.
pub fn topbar_inset() -> f32 {
    #[cfg(target_family = "wasm")]
    {
        super::browser::topbar_inset()
    }
    #[cfg(not(target_family = "wasm"))]
    {
        0.
    }
}
