//! Original-file fulfillment for an explicit external drag, independent of UI.
use crate::storage::PhotosDriver;
use anyhow::{Context, Result, ensure};
use std::{collections::HashSet, fs, path::Path};

pub fn filenames<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut used = HashSet::new();
    names
        .into_iter()
        .map(|name| {
            let basename = name.rsplit(['/', '\\']).next().unwrap_or_default();
            let basename: String = basename
                .chars()
                .map(|c| if c.is_control() { '_' } else { c })
                .collect();
            let basename = if basename.is_empty() || matches!(basename.as_str(), "." | "..") {
                "Photo.jpg".into()
            } else {
                basename
            };
            let path = Path::new(&basename);
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            let extension = path
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            let mut candidate = basename.clone();
            let mut suffix = 2;
            while !used.insert(candidate.to_lowercase()) {
                candidate = format!("{stem} ({suffix}){extension}");
                suffix += 1;
            }
            candidate
        })
        .collect()
}

/// Called only when a receiving app fulfills the promise. Preserve original
/// bytes, use the receiving app's destination name, and never overwrite a file.
pub fn write_original(driver: &dyn PhotosDriver, id: &str, destination: &Path) -> Result<()> {
    let parent = destination
        .parent()
        .context("Missing drag destination folder")?;
    ensure!(
        parent.is_dir(),
        "The receiving app's destination folder is unavailable"
    );
    ensure!(
        destination.symlink_metadata().is_err(),
        "The receiving app's destination already exists"
    );
    let staging = tempfile::tempdir_in(parent)?;
    ensure!(
        driver.export(&[id.to_owned()], staging.path())? == 1,
        "The original photo could not be exported"
    );
    let files = fs::read_dir(staging.path())?.collect::<std::io::Result<Vec<_>>>()?;
    ensure!(
        files.len() == 1 && files[0].file_type()?.is_file(),
        "Expected one original photo"
    );
    let mut output = tempfile::NamedTempFile::new_in(parent)?;
    std::io::copy(&mut fs::File::open(files[0].path())?, &mut output)?;
    output.as_file().sync_all()?;
    output
        .persist_noclobber(destination)
        .context("Cannot deliver the original photo without overwriting an existing file")?;
    Ok(())
}
