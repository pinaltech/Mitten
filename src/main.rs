mod ui;
fn main() -> anyhow::Result<()> {
    if std::env::args().any(|arg| arg == "--verify-photo-permissions") {
        #[cfg(all(target_os = "macos", feature = "apple-photos"))]
        {
            mitten::sources::verify_photokit_configuration()?;
            println!("PhotoKit usage description verified (no library access requested).");
            return Ok(());
        }
        #[cfg(not(all(target_os = "macos", feature = "apple-photos")))]
        anyhow::bail!("PhotoKit is not enabled in this build");
    }
    ui::run()
}
