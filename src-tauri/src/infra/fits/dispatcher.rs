use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use tempfile::TempDir;

pub fn resolve_single_image(path: &str) -> Result<(PathBuf, Option<TempDir>)> {
    let p = Path::new(path);
    if !is_zip_path(p) {
        return Ok((PathBuf::from(path), None));
    }
    let (files, tmp) = extract_zip_images(p)?;
    let first = files
        .into_iter()
        .next()
        .context("No supported image files inside ZIP")?;
    Ok((first, Some(tmp)))
}

fn is_zip_path(p: &Path) -> bool {
    p.extension()
        .map(|ext| ext.eq_ignore_ascii_case("zip"))
        .unwrap_or(false)
}

const MAX_ZIP_DEPTH: u32 = 4;

const ZIP_IMAGE_SUFFIXES: [&str; 5] = [".fits", ".fit", ".fts", ".fz", ".asdf"];

struct ZipExtraction {
    images: Vec<(String, PathBuf)>,
    nested_errors: Vec<String>,
}

fn extract_zip_images(zip_path: &Path) -> Result<(Vec<PathBuf>, TempDir)> {
    if zip_path.is_dir() {
        bail!(
            "{} is a folder, not a ZIP archive; open the image files inside it instead",
            zip_path.display()
        );
    }
    let tmp_dir = TempDir::new().context("Failed to create temp directory")?;
    let mut extraction = ZipExtraction { images: Vec::new(), nested_errors: Vec::new() };

    extract_zip_recursive(zip_path, tmp_dir.path(), "", &mut extraction, 0)?;

    if extraction.images.is_empty() {
        if extraction.nested_errors.is_empty() {
            bail!("No supported image files found inside ZIP {:?} (checked nested ZIPs too)", zip_path);
        }
        bail!(
            "No supported image files found inside ZIP {:?}; nested ZIPs could not be read: {}",
            zip_path,
            extraction.nested_errors.join("; ")
        );
    }

    extraction.images.sort_by(|a, b| a.0.cmp(&b.0));
    let files = extraction.images.into_iter().map(|(_, path)| path).collect();
    Ok((files, tmp_dir))
}

fn extract_zip_recursive(
    zip_path: &Path,
    out_dir: &Path,
    prefix: &str,
    extraction: &mut ZipExtraction,
    depth: u32,
) -> Result<()> {
    if depth > MAX_ZIP_DEPTH {
        bail!("Nested ZIP depth exceeds limit ({})", MAX_ZIP_DEPTH);
    }

    let file = File::open(zip_path)
        .with_context(|| format!("Failed to open ZIP {:?}", zip_path))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("Failed to read ZIP archive {:?}", zip_path))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .with_context(|| format!("Failed to read ZIP entry {}", i))?;

        if entry.is_dir() {
            continue;
        }

        let entry_name = entry.name().to_string();
        let entry_lower = entry_name.to_lowercase();
        let is_image = ZIP_IMAGE_SUFFIXES.iter().any(|s| entry_lower.ends_with(s));
        let is_nested_zip = entry_lower.ends_with(".zip");
        if !is_image && !is_nested_zip {
            continue;
        }
        let Some(relative) = entry.enclosed_name().filter(|p| p.file_name().is_some()) else {
            log::warn!("skipping ZIP entry {:?}: its path leaves the archive", entry_name);
            continue;
        };

        let (out_path, mut out_file) = create_extracted_file(out_dir, &relative, i)?;
        io::copy(&mut entry, &mut out_file)
            .with_context(|| format!("Failed to extract {:?}", entry_name))?;
        drop(out_file);

        let label = format!("{prefix}{entry_name}");
        if is_image {
            extraction.images.push((label, out_path));
            continue;
        }

        let nested_dir = out_path.with_extension("zip.contents");
        let nested = fs::create_dir_all(&nested_dir)
            .with_context(|| format!("Failed to create extraction folder {:?}", nested_dir))
            .and_then(|_| {
                extract_zip_recursive(&out_path, &nested_dir, &format!("{label}/"), extraction, depth + 1)
            });
        if let Err(e) = nested {
            log::warn!("skipping nested ZIP {}: {:#}", label, e);
            extraction.nested_errors.push(format!("{label}: {e:#}"));
        }
        let _ = fs::remove_file(&out_path);
    }

    Ok(())
}

fn create_new_file(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    File::options().write(true).create_new(true).open(path)
}

fn create_extracted_file(out_dir: &Path, relative: &Path, index: usize) -> Result<(PathBuf, File)> {
    let preferred = out_dir.join(relative);
    if let Ok(file) = create_new_file(&preferred) {
        return Ok((preferred, file));
    }
    let file_name = relative.file_name().unwrap_or(relative.as_os_str());
    let fallback = out_dir.join(format!(".entry_{index}")).join(file_name);
    let file = create_new_file(&fallback)
        .with_context(|| format!("Failed to create extracted file {:?}", fallback))?;
    Ok((fallback, file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut writer = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, bytes) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn test_is_zip_path() {
        assert!(is_zip_path(Path::new("archive.zip")));
        assert!(is_zip_path(Path::new("archive.ZIP")));
        assert!(!is_zip_path(Path::new("data.fits")));
    }

    #[test]
    fn a_folder_named_like_a_zip_is_refused_instead_of_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("night1.zip");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("frame.fits"), b"SIMPLE").unwrap();

        let err = resolve_single_image(folder.to_str().unwrap()).unwrap_err();
        assert!(err.to_string().contains("is a folder, not a ZIP archive"), "{err}");
    }

    #[test]
    fn same_named_entries_in_different_folders_do_not_overwrite_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("two_nights.zip");
        write_zip(
            &zip_path,
            &[("b/frame.fits", b"second"), ("a/frame.fits", b"first"), ("notes.txt", b"skip")],
        );

        let (files, _tmp) = extract_zip_images(&zip_path).unwrap();
        assert_eq!(files.len(), 2);
        assert_ne!(files[0], files[1]);
        let contents: Vec<Vec<u8>> = files.iter().map(|p| fs::read(p).unwrap()).collect();
        assert_eq!(contents, vec![b"first".to_vec(), b"second".to_vec()]);

        let (first, tmp) = resolve_single_image(zip_path.to_str().unwrap()).unwrap();
        assert!(tmp.is_some());
        assert_eq!(first.file_name().unwrap(), "frame.fits");
        assert_eq!(fs::read(&first).unwrap(), b"first");
    }

    #[test]
    fn colliding_names_fall_back_instead_of_overwriting_and_siblings_stay_together() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("collide.zip");
        write_zip(
            &zip_path,
            &[
                ("dup/Frame.fits", b"upper"),
                ("dup/frame.fits", b"lower"),
                ("pair/jw_cal.asdf", b"asdf"),
                ("pair/jw_cal.fits", b"fits"),
                ("../escape.fits", b"outside"),
            ],
        );
        let (files, tmp) = extract_zip_images(&zip_path).unwrap();
        let contents: Vec<Vec<u8>> = files.iter().map(|p| fs::read(p).unwrap()).collect();
        assert_eq!(
            contents,
            vec![b"upper".to_vec(), b"lower".to_vec(), b"asdf".to_vec(), b"fits".to_vec()]
        );
        assert!(files.iter().all(|p| p.starts_with(tmp.path())));
        assert_eq!(files[2].parent(), files[3].parent(), "an ASDF keeps its companion .fits beside it");
        assert_eq!(files[2].with_extension("fits"), files[3]);
    }

    #[test]
    fn an_entry_whose_folder_is_taken_by_a_file_falls_back_on_any_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("blocked.zip");
        write_zip(&zip_path, &[("scan.fits", b"file"), ("scan.fits/inner.fits", b"inner")]);

        let (files, tmp) = extract_zip_images(&zip_path).unwrap();
        let contents: Vec<Vec<u8>> = files.iter().map(|p| fs::read(p).unwrap()).collect();
        assert_eq!(contents, vec![b"file".to_vec(), b"inner".to_vec()]);
        assert_eq!(files[0], tmp.path().join("scan.fits"));
        assert_eq!(files[1], tmp.path().join(".entry_1").join("inner.fits"));
    }

    #[test]
    fn a_broken_nested_zip_reports_its_own_error() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("outer.zip");
        write_zip(&zip_path, &[("inner.zip", b"this is not a zip archive")]);

        let err = resolve_single_image(zip_path.to_str().unwrap()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("nested ZIPs could not be read"), "{msg}");
        assert!(msg.contains("inner.zip: Failed to read ZIP archive"), "{msg}");
    }

    #[test]
    fn nested_zip_images_are_found_and_sorted_by_archive_path() {
        let dir = tempfile::tempdir().unwrap();
        let inner = dir.path().join("inner_src.zip");
        write_zip(&inner, &[("deep.fits", b"nested")]);
        let inner_bytes = fs::read(&inner).unwrap();
        let outer = dir.path().join("outer_ok.zip");
        write_zip(&outer, &[("z_last.fits", b"top"), ("a_inner.zip", &inner_bytes)]);

        let (first, _tmp) = resolve_single_image(outer.to_str().unwrap()).unwrap();
        assert_eq!(fs::read(&first).unwrap(), b"nested");
    }

    #[test]
    fn fpack_entries_inside_a_zip_are_opened() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("fpacked.zip");
        write_zip(
            &zip_path,
            &[("notes.txt", b"skip"), ("jw_i2d.fits.fz", b"packed"), ("old/frame.FZ", b"upper")],
        );

        let (files, _tmp) = extract_zip_images(&zip_path).unwrap();
        let contents: Vec<Vec<u8>> = files.iter().map(|p| fs::read(p).unwrap()).collect();
        assert_eq!(contents, vec![b"packed".to_vec(), b"upper".to_vec()]);

        let (first, tmp) = resolve_single_image(zip_path.to_str().unwrap()).unwrap();
        assert!(tmp.is_some());
        assert_eq!(first.file_name().unwrap(), "jw_i2d.fits.fz");
    }

    #[test]
    fn non_zip_paths_pass_through_untouched() {
        let (path, tmp) = resolve_single_image("C:/data/frame.fits").unwrap();
        assert_eq!(path, PathBuf::from("C:/data/frame.fits"));
        assert!(tmp.is_none());
    }
}
