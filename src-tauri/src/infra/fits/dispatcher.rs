use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use tempfile::TempDir;

use super::gzip::{
    copy_capped, gunzip_or_copy, resolve_gzip, split_gz_suffix, temp_folder_cap, Cap, SharedTempDir,
};
use super::reader::{list_extensions, HduInfo};

pub fn resolve_single_image(path: &str) -> Result<(PathBuf, Option<SharedTempDir>)> {
    let p = Path::new(path);
    if !is_zip_path(p) {
        return resolve_gzip(p);
    }
    let (first, tmp) = extract_first_zip_image(p, &temp_folder_cap)?;
    Ok((first, Some(SharedTempDir::new(tmp))))
}

pub fn list_source_extensions(source: &str) -> Result<Vec<HduInfo>> {
    let (readable, _tmp) = resolve_single_image(source)?;
    let file = File::open(&readable).with_context(|| format!("Failed to open {}", readable.display()))?;
    list_extensions(&file)
}

fn is_zip_path(p: &Path) -> bool {
    p.extension()
        .map(|ext| ext.eq_ignore_ascii_case("zip"))
        .unwrap_or(false)
}

const MAX_ZIP_DEPTH: u32 = 4;

const ZIP_IMAGE_SUFFIXES: [&str; 8] =
    [".fits", ".fit", ".fts", ".fz", ".asdf", ".fits.gz", ".fit.gz", ".fts.gz"];

struct ZipMember {
    label: String,
    archive: PathBuf,
    index: usize,
    relative: PathBuf,
    out_dir: PathBuf,
}

struct ZipExtraction<'a> {
    root: &'a Path,
    cap: &'a dyn Fn() -> Cap,
    images: Vec<ZipMember>,
    nested_copies: Vec<PathBuf>,
    nested_errors: Vec<String>,
}

fn extract_first_zip_image(zip_path: &Path, cap: &dyn Fn() -> Cap) -> Result<(PathBuf, TempDir)> {
    if zip_path.is_dir() {
        bail!(
            "{} is a folder, not a ZIP archive; open the image files inside it instead",
            zip_path.display()
        );
    }
    let tmp_dir = TempDir::new().context("Failed to create temp directory")?;
    let mut extraction = ZipExtraction {
        root: zip_path,
        cap,
        images: Vec::new(),
        nested_copies: Vec::new(),
        nested_errors: Vec::new(),
    };

    extraction.catalogue(zip_path, tmp_dir.path(), "", 0)?;

    let Some(first) = extraction.images.iter().min_by(|a, b| a.label.cmp(&b.label)) else {
        if extraction.nested_errors.is_empty() {
            bail!("No supported image files found inside ZIP {:?} (checked nested ZIPs too)", zip_path);
        }
        bail!(
            "No supported image files found inside ZIP {:?}; nested ZIPs could not be read: {}",
            zip_path,
            extraction.nested_errors.join("; ")
        );
    };

    let image = extraction.extract(first, None)?;
    if first.relative.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("asdf")) {
        if let Some(companion) = extraction.companion_of(first) {
            extraction.extract(companion, Some(&image.with_extension("fits")))?;
        }
    }
    for copy in &extraction.nested_copies {
        let _ = fs::remove_file(copy);
    }
    Ok((image, tmp_dir))
}

impl ZipExtraction<'_> {
    fn catalogue(&mut self, zip_path: &Path, out_dir: &Path, prefix: &str, depth: u32) -> Result<()> {
        if depth > MAX_ZIP_DEPTH {
            bail!("Nested ZIP depth exceeds limit ({})", MAX_ZIP_DEPTH);
        }

        let file = File::open(zip_path)
            .with_context(|| format!("Failed to open ZIP {:?}", zip_path))?;
        let mut archive = zip::ZipArchive::new(file)
            .with_context(|| format!("Failed to read ZIP archive {:?}", zip_path))?;

        let mut nested_zips: Vec<(usize, String, PathBuf)> = Vec::new();
        for i in 0..archive.len() {
            let entry = archive
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

            let label = format!("{prefix}{entry_name}");
            if is_image {
                self.images.push(ZipMember {
                    label,
                    archive: zip_path.to_path_buf(),
                    index: i,
                    relative: without_gz_suffix(relative),
                    out_dir: out_dir.to_path_buf(),
                });
            } else {
                nested_zips.push((i, label, relative));
            }
        }

        for (i, label, relative) in nested_zips {
            if self.images.iter().any(|image| image.label < label) {
                continue;
            }
            let mut entry = archive
                .by_index(i)
                .with_context(|| format!("Failed to read ZIP entry {}", i))?;
            let (copy, mut out_file) = create_extracted_file(out_dir, &relative, i)?;
            let copied = copy_capped(&mut entry, &mut out_file, &(self.cap)(), &self.describe(&label));
            drop(out_file);
            if let Err(e) = copied {
                let _ = fs::remove_file(&copy);
                self.skip_nested(e);
                continue;
            }
            self.nested_copies.push(copy.clone());
            let nested_dir = copy.with_extension("zip.contents");
            let nested = fs::create_dir_all(&nested_dir)
                .with_context(|| format!("Failed to create extraction folder {:?}", nested_dir))
                .and_then(|_| self.catalogue(&copy, &nested_dir, &format!("{label}/"), depth + 1))
                .with_context(|| label.clone());
            if let Err(e) = nested {
                self.skip_nested(e);
            }
        }

        Ok(())
    }

    fn skip_nested(&mut self, error: anyhow::Error) {
        let reason = format!("{error:#}");
        log::warn!("skipping nested ZIP: {reason}");
        self.nested_errors.push(reason);
    }

    fn describe(&self, label: &str) -> String {
        format!("{label} in {}", self.root.display())
    }

    fn companion_of(&self, image: &ZipMember) -> Option<&ZipMember> {
        let wanted = image.relative.with_extension("fits");
        self.images
            .iter()
            .filter(|m| m.archive == image.archive && m.relative.parent() == wanted.parent())
            .filter(|m| {
                m.relative
                    .file_name()
                    .zip(wanted.file_name())
                    .is_some_and(|(name, fits)| name.eq_ignore_ascii_case(fits))
            })
            .min_by(|a, b| a.label.cmp(&b.label))
    }

    fn extract(&self, member: &ZipMember, target: Option<&Path>) -> Result<PathBuf> {
        let file = File::open(&member.archive)
            .with_context(|| format!("Failed to open ZIP {:?}", member.archive))?;
        let mut archive = zip::ZipArchive::new(file)
            .with_context(|| format!("Failed to read ZIP archive {:?}", member.archive))?;
        let entry = archive
            .by_index(member.index)
            .with_context(|| format!("Failed to read ZIP entry {}", member.index))?;
        let (path, mut out_file) = match target {
            Some(path) => {
                let file = create_new_file(path)
                    .with_context(|| format!("Failed to create extracted file {:?}", path))?;
                (path.to_path_buf(), file)
            }
            None => create_extracted_file(&member.out_dir, &member.relative, member.index)?,
        };
        gunzip_or_copy(entry, &mut out_file, self.cap, &self.describe(&member.label))?;
        Ok(path)
    }
}

fn without_gz_suffix(relative: PathBuf) -> PathBuf {
    let name = relative
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match split_gz_suffix(&name) {
        (stem, Some(_)) => relative.with_file_name(stem),
        _ => relative,
    }
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
pub(crate) mod test_support {
    use std::fs::File;
    use std::io::Write;
    use std::path::Path;

    pub(crate) fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut writer = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, bytes) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::write_zip;
    use super::*;

    use crate::core::stacking::memory::{format_bytes, MEMORY_BUDGET_ENV};
    use crate::infra::fits::gzip::test_support::{gzip_bytes, memory_budget};
    use crate::infra::fits::reader::test_fixtures::{empty_primary_cards, plane_hdu, write_raw_hdus};
    use crate::infra::image_source::load_plane;
    use crate::types::ImageRef;

    fn first_image(zip_path: &Path, cap: u64) -> Result<(PathBuf, TempDir)> {
        extract_first_zip_image(zip_path, &|| memory_budget(cap))
    }

    fn files_under(dir: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut pending = vec![dir.to_path_buf()];
        while let Some(folder) = pending.pop() {
            for entry in fs::read_dir(&folder).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    found.push(path);
                }
            }
        }
        found.sort();
        found
    }

    const SMALL_CAP: u64 = 1 << 20;
    const BOMB_BYTES: usize = 8 << 20;

    #[test]
    fn a_damaged_gzip_member_after_the_first_image_is_never_read() {
        let dir = tempfile::tempdir().unwrap();
        let gz = gzip_bytes(&vec![7u8; 4096]);
        let zip_path = dir.path().join("partial.zip");
        write_zip(&zip_path, &[("z/cut.fits.gz", &gz[..gz.len() / 2]), ("a/light.fits", b"SIMPLE light")]);

        let (first, tmp) = first_image(&zip_path, u64::MAX).unwrap();
        assert_eq!(fs::read(&first).unwrap(), b"SIMPLE light");
        assert_eq!(files_under(tmp.path()), [first]);
    }

    #[test]
    fn an_over_cap_gzip_member_after_the_first_image_is_neither_refused_nor_written() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("mixed.zip");
        write_zip(
            &zip_path,
            &[("z/big.fits.gz", &gzip_bytes(&vec![0u8; BOMB_BYTES])), ("a/light.fits", b"SIMPLE light")],
        );

        for cap in [SMALL_CAP, u64::MAX] {
            let (first, tmp) = first_image(&zip_path, cap).unwrap();
            assert_eq!(fs::read(&first).unwrap(), b"SIMPLE light", "cap {cap}");
            assert_eq!(files_under(tmp.path()), [first], "cap {cap}");
        }
    }

    #[test]
    fn an_over_cap_gzip_member_of_a_nested_zip_fails_the_open_when_it_is_the_first_image() {
        let dir = tempfile::tempdir().unwrap();
        let inner = dir.path().join("inner_src.zip");
        write_zip(&inner, &[("frame.fits.gz", &gzip_bytes(&vec![0u8; BOMB_BYTES]))]);
        let zip_path = dir.path().join("outer.zip");
        write_zip(&zip_path, &[("a_night.zip", &fs::read(&inner).unwrap()), ("z_ref.fits", b"SIMPLE ref")]);

        let (first, _tmp) = first_image(&zip_path, BOMB_BYTES as u64).unwrap();
        assert_eq!(first.file_name().unwrap(), "frame.fits");
        assert_eq!(fs::metadata(&first).unwrap().len(), BOMB_BYTES as u64);

        let err = first_image(&zip_path, SMALL_CAP)
            .map(|(first, _)| first)
            .expect_err("the first image is refused, not swapped for z_ref.fits")
            .to_string();
        for expected in ["a_night.zip/frame.fits.gz", "outer.zip", format_bytes(SMALL_CAP).as_str()] {
            assert!(err.contains(expected), "missing {expected}: {err}");
        }
    }

    #[test]
    fn a_member_without_the_gzip_magic_is_held_to_the_same_cap() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["bomb.fits.gz", "bomb.fits"] {
            let zip_path = dir.path().join(format!("misnamed_{name}.zip"));
            write_zip(&zip_path, &[(name, &vec![0u8; BOMB_BYTES])]);
            let on_disk = fs::metadata(&zip_path).unwrap().len();
            assert!(on_disk < SMALL_CAP / 8, "the member must be a bomb: {on_disk} bytes");

            let err = first_image(&zip_path, SMALL_CAP)
                .map(|(first, _)| first)
                .expect_err("plain bytes past the cap are refused")
                .to_string();
            for expected in [name, format_bytes(SMALL_CAP).as_str(), MEMORY_BUDGET_ENV] {
                assert!(err.contains(expected), "missing {expected}: {err}");
            }
            assert!(!err.contains("gzip stream"), "{err}");

            let (first, _tmp) = first_image(&zip_path, BOMB_BYTES as u64).unwrap();
            assert_eq!(fs::metadata(&first).unwrap().len(), BOMB_BYTES as u64, "a member exactly at the cap passes");
        }
    }

    #[test]
    fn an_over_cap_nested_zip_is_reported_with_the_cap_and_leaves_no_partial_copy() {
        let dir = tempfile::tempdir().unwrap();
        let alone = dir.path().join("alone.zip");
        write_zip(&alone, &[("inner.zip", &vec![0u8; BOMB_BYTES])]);
        let err = format!(
            "{:#}",
            first_image(&alone, SMALL_CAP).map(|(first, _)| first).expect_err("nothing to open")
        );
        for expected in ["nested ZIPs could not be read", "inner.zip", format_bytes(SMALL_CAP).as_str()] {
            assert!(err.contains(expected), "missing {expected}: {err}");
        }

        let beside = dir.path().join("beside.zip");
        write_zip(&beside, &[("inner.zip", &vec![0u8; BOMB_BYTES]), ("z.fits", b"SIMPLE z")]);
        let (first, tmp) = first_image(&beside, SMALL_CAP).unwrap();
        assert_eq!(fs::read(&first).unwrap(), b"SIMPLE z");
        assert_eq!(files_under(tmp.path()), [first]);
    }

    #[test]
    fn an_asdf_first_image_brings_only_its_companion_fits() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("asdf_pair.zip");
        write_zip(
            &zip_path,
            &[
                ("z/other.fits", b"other"),
                ("pair/JW_CAL.FITS.GZ", &gzip_bytes(b"fits")),
                ("pair/JW_CAL.ASDF", b"asdf"),
            ],
        );

        let (first, tmp) = first_image(&zip_path, u64::MAX).unwrap();
        assert_eq!(fs::read(&first).unwrap(), b"asdf");
        let companion = first.with_extension("fits");
        assert_eq!(fs::read(&companion).unwrap(), b"fits", "found where companion_fits_path looks");
        assert_eq!(files_under(tmp.path()), [first, companion]);
    }

    #[test]
    fn gzip_compressed_fits_members_open_under_their_name_without_gz() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("frame.fits");
        write_raw_hdus(&plain, &[(empty_primary_cards(), Vec::new()), plane_hdu("SCI", 6, 5)]);
        let zip_path = dir.path().join("compressed.zip");
        write_zip(
            &zip_path,
            &[
                ("a/logs.tar.gz", &gzip_bytes(b"tar")),
                ("a/readme.gz", &gzip_bytes(b"text")),
                ("b/frame.fits.gz", &gzip_bytes(&fs::read(&plain).unwrap())),
            ],
        );

        let (first, tmp) = resolve_single_image(zip_path.to_str().unwrap()).unwrap();
        assert!(tmp.is_some());
        assert_eq!(first.file_name().unwrap(), "frame.fits");
        assert_eq!(
            load_plane(&ImageRef::parse(zip_path.to_str().unwrap())).unwrap().arr,
            load_plane(&ImageRef::parse(plain.to_str().unwrap())).unwrap().arr
        );

        let inner = dir.path().join("inner_src.zip");
        write_zip(&inner, &[("deep.fits.gz", &gzip_bytes(b"nested"))]);
        for (member, bytes, name, content) in [
            ("b/LUM.FIT.GZ", gzip_bytes(b"upper"), "LUM.FIT", b"upper".as_slice()),
            ("c/old.fts.gz", gzip_bytes(b"fts"), "old.fts", b"fts".as_slice()),
            ("d/inner.zip", fs::read(&inner).unwrap(), "deep.fits", b"nested".as_slice()),
        ] {
            let single = dir.path().join(format!("{name}.zip"));
            write_zip(&single, &[(member, &bytes)]);
            let (first, _tmp) = first_image(&single, u64::MAX).unwrap();
            assert_eq!(first.file_name().unwrap(), name);
            assert_eq!(fs::read(&first).unwrap(), content, "{member}");
        }
    }

    #[test]
    fn members_are_decompressed_by_their_bytes_like_a_standalone_fits_gz() {
        let dir = tempfile::tempdir().unwrap();
        for (member, bytes, name, content) in [
            ("misnamed.fits.gz", b"SIMPLE plain".to_vec(), "misnamed.fits", b"SIMPLE plain".as_slice()),
            ("renamed.fits", gzip_bytes(b"SIMPLE packed"), "renamed.fits", b"SIMPLE packed".as_slice()),
        ] {
            let zip_path = dir.path().join(format!("{member}.zip"));
            write_zip(&zip_path, &[(member, &bytes)]);
            let (first, _tmp) = first_image(&zip_path, u64::MAX).unwrap();
            assert_eq!(first.file_name().unwrap(), name);
            assert_eq!(fs::read(&first).unwrap(), content, "{member}");
        }
    }

    #[test]
    fn an_oversized_gzip_member_is_refused_by_the_decompression_cap() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("bomb.zip");
        let bomb = gzip_bytes(&vec![0u8; BOMB_BYTES]);
        assert!((bomb.len() as u64) < SMALL_CAP / 8, "the member must be a bomb: {} bytes", bomb.len());
        write_zip(&zip_path, &[("frames/bomb.fits.gz", &bomb)]);

        let err = first_image(&zip_path, SMALL_CAP).map(|(first, _)| first).unwrap_err();
        let display = err.to_string();
        for expected in ["frames/bomb.fits.gz", "bomb.zip", format_bytes(SMALL_CAP).as_str(), MEMORY_BUDGET_ENV] {
            assert!(display.contains(expected), "missing {expected}: {display}");
        }
        let alternate = format!("{err:#}");
        assert_eq!(alternate.matches("frames/bomb.fits.gz").count(), 1, "{alternate}");

        let (first, _tmp) = first_image(&zip_path, BOMB_BYTES as u64).unwrap();
        assert_eq!(fs::metadata(&first).unwrap().len(), BOMB_BYTES as u64, "a member exactly at the cap passes");
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
    fn only_the_first_image_by_archive_path_is_extracted() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("two_nights.zip");
        write_zip(
            &zip_path,
            &[("b/frame.fits", b"second"), ("a/frame.fits", b"first"), ("notes.txt", b"skip")],
        );

        let (first, tmp) = first_image(&zip_path, u64::MAX).unwrap();
        assert_eq!(fs::read(&first).unwrap(), b"first");
        assert_eq!(files_under(tmp.path()), [first]);

        let (first, tmp) = resolve_single_image(zip_path.to_str().unwrap()).unwrap();
        assert!(tmp.is_some());
        assert_eq!(first.file_name().unwrap(), "frame.fits");
        assert_eq!(fs::read(&first).unwrap(), b"first");
    }

    #[test]
    fn an_asdf_takes_its_first_companion_by_archive_path_and_escaping_entries_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("collide.zip");
        write_zip(
            &zip_path,
            &[
                ("../escape.fits", b"outside"),
                ("pair/jw_cal.fits.gz", &gzip_bytes(b"packed")),
                ("pair/jw_cal.asdf", b"asdf"),
                ("pair/jw_cal.fits", b"plain"),
            ],
        );

        let (first, tmp) = first_image(&zip_path, u64::MAX).unwrap();
        assert_eq!(fs::read(&first).unwrap(), b"asdf");
        let companion = first.with_extension("fits");
        assert_eq!(fs::read(&companion).unwrap(), b"plain");
        assert_eq!(files_under(tmp.path()), [first, companion]);
    }

    #[test]
    fn an_image_keeps_its_path_when_a_nested_zip_sits_under_it_and_survives_a_folder_collision() {
        let dir = tempfile::tempdir().unwrap();
        let inner = dir.path().join("inner_src.zip");
        write_zip(&inner, &[("deep.fits", b"inner")]);
        let inner_bytes = fs::read(&inner).unwrap();
        let zip_path = dir.path().join("blocked.zip");
        write_zip(&zip_path, &[("scan.fits", b"file"), ("scan.fits/inner.zip", &inner_bytes)]);

        let (first, tmp) = first_image(&zip_path, u64::MAX).unwrap();
        assert_eq!(fs::read(&first).unwrap(), b"file");
        assert_eq!(first, tmp.path().join("scan.fits"), "the nested ZIP under the image sorts after it and is never extracted");
        assert_eq!(files_under(tmp.path()), [first]);

        let imageless = dir.path().join("imageless_src.zip");
        write_zip(&imageless, &[("notes.txt", b"skip")]);
        let colliding = dir.path().join("colliding.zip");
        write_zip(&colliding, &[("Scan.fits/inner.zip", &fs::read(&imageless).unwrap()), ("scan.fits", b"file")]);
        let (first, tmp) = first_image(&colliding, u64::MAX).unwrap();
        assert_eq!(fs::read(&first).unwrap(), b"file");
        assert_eq!(first.file_name().unwrap(), "scan.fits");
        assert_eq!(files_under(tmp.path()), [first], "the nested copy that sorted first is removed once the image is out");
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
    fn a_nested_zip_that_cannot_hold_the_first_image_is_never_copied_out() {
        let dir = tempfile::tempdir().unwrap();
        let inner = dir.path().join("inner_src.zip");
        write_zip(&inner, &[("frame.fits", b"SIMPLE nested")]);
        let inner_bytes = fs::read(&inner).unwrap();

        for (name, entries) in [
            ("image_first.zip", vec![("a_light.fits", b"SIMPLE light".as_slice()), ("z_night.zip", inner_bytes.as_slice())]),
            ("zip_first.zip", vec![("z_night.zip", inner_bytes.as_slice()), ("a_light.fits", b"SIMPLE light".as_slice())]),
        ] {
            let zip_path = dir.path().join(name);
            write_zip(&zip_path, &entries);
            let (first, tmp) = first_image(&zip_path, u64::MAX).unwrap();
            assert_eq!(fs::read(&first).unwrap(), b"SIMPLE light", "{name}");
            assert!(
                !tmp.path().join("z_night.zip.contents").exists(),
                "{name}: z_night.zip sorts after a_light.fits, so none of its members can be the first image; it was still extracted: {:?}",
                files_under(tmp.path())
            );
        }

        let winning = dir.path().join("nested_wins.zip");
        write_zip(&winning, &[("z_light.fits", b"SIMPLE light"), ("a_night.zip", &inner_bytes)]);
        let (first, _tmp) = first_image(&winning, u64::MAX).unwrap();
        assert_eq!(fs::read(&first).unwrap(), b"SIMPLE nested", "a nested ZIP that sorts first is still searched");
    }

    #[test]
    fn fpack_entries_inside_a_zip_are_opened() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("fpacked.zip");
        write_zip(
            &zip_path,
            &[("notes.txt", b"skip"), ("jw_i2d.fits.fz", b"packed"), ("old/frame.FZ", b"upper")],
        );

        let (first, tmp) = resolve_single_image(zip_path.to_str().unwrap()).unwrap();
        assert!(tmp.is_some());
        assert_eq!(first.file_name().unwrap(), "jw_i2d.fits.fz");
        assert_eq!(fs::read(&first).unwrap(), b"packed");

        let upper = dir.path().join("upper.zip");
        write_zip(&upper, &[("notes.txt", b"skip"), ("old/frame.FZ", b"upper")]);
        let (first, _tmp) = first_image(&upper, u64::MAX).unwrap();
        assert_eq!(fs::read(&first).unwrap(), b"upper");
    }

    #[test]
    fn non_zip_paths_pass_through_untouched() {
        let (path, tmp) = resolve_single_image("C:/data/frame.fits").unwrap();
        assert_eq!(path, PathBuf::from("C:/data/frame.fits"));
        assert!(tmp.is_none());
    }
}
