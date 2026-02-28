use std::fs::{self, File};
use std::io::{self, Read, Seek};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use tempfile::TempDir;

pub enum ResolvedInput {
    SingleFile(PathBuf),
    MultipleFiles(Vec<PathBuf>),
    ExtractedFromZip {
        files: Vec<PathBuf>,
        _tmp: TempDir,
    },
}

impl ResolvedInput {
    pub fn fits_paths(&self) -> &[PathBuf] {
        match self {
            ResolvedInput::SingleFile(p) => std::slice::from_ref(p),
            ResolvedInput::MultipleFiles(v) => v,
            ResolvedInput::ExtractedFromZip { files, .. } => files,
        }
    }

    
    pub fn first_fits(&self) -> Result<&Path> {
        self.fits_paths()
            .first()
            .map(|p| p.as_path())
            .context("No .fits files found in input")
    }
}

pub fn resolve_input(path: &Path) -> Result<ResolvedInput> {
    if path.is_dir() {
        let mut fits: Vec<PathBuf> = fs::read_dir(path)
            .with_context(|| format!("Failed to read directory {:?}", path))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| is_fits_path(p))
            .collect();
        fits.sort();
        if fits.is_empty() {
            bail!("No .fits files found in directory {:?}", path);
        }
        Ok(ResolvedInput::MultipleFiles(fits))
    } else if is_zip_path(path) {
        extract_fits_from_zip(path)
    } else if is_fits_path(path) {
        Ok(ResolvedInput::SingleFile(path.to_path_buf()))
    } else {
        
        Ok(ResolvedInput::SingleFile(path.to_path_buf()))
    }
}



pub fn resolve_single_fits(path: &str) -> Result<(PathBuf, Option<TempDir>)> {
    let p = Path::new(path);

    if is_zip_path(p) {
        let resolved = resolve_input(p)?;
        match resolved {
            ResolvedInput::ExtractedFromZip { files, _tmp } => {
                let first = files
                    .into_iter()
                    .next()
                    .context("No .fits inside ZIP")?;
                Ok((first, Some(_tmp)))
            }
            _ => unreachable!(),
        }
    } else {
        Ok((PathBuf::from(path), None))
    }
}

fn is_fits_path(p: &Path) -> bool {
    p.extension()
        .map(|ext| {
            let e = ext.to_ascii_lowercase();
            e == "fits" || e == "fit" || e == "fts"
        })
        .unwrap_or(false)
}

fn is_zip_path(p: &Path) -> bool {
    p.extension()
        .map(|ext| ext.eq_ignore_ascii_case("zip"))
        .unwrap_or(false)
}


fn extract_fits_from_zip(zip_path: &Path) -> Result<ResolvedInput> {
    let tmp_dir = TempDir::new().context("Failed to create temp directory")?;
    let mut extracted: Vec<PathBuf> = Vec::new();

    extract_zip_recursive(zip_path, tmp_dir.path(), &mut extracted, 0)?;

    if extracted.is_empty() {
        bail!("No .fits files found inside ZIP {:?} (checked nested ZIPs too)", zip_path);
    }

    extracted.sort();
    Ok(ResolvedInput::ExtractedFromZip {
        files: extracted,
        _tmp: tmp_dir,
    })
}


const MAX_ZIP_DEPTH: u32 = 4;

fn extract_zip_recursive(
    zip_path: &Path,
    out_dir: &Path,
    collected: &mut Vec<PathBuf>,
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

        
        let file_name = Path::new(&entry_name)
            .file_name()
            .unwrap_or_default()
            .to_os_string();

        if entry_lower.ends_with(".fits")
            || entry_lower.ends_with(".fit")
            || entry_lower.ends_with(".fts")
        {
            
            let out_path = out_dir.join(&file_name);
            let mut out_file = File::create(&out_path)
                .with_context(|| format!("Failed to create extracted file {:?}", out_path))?;
            io::copy(&mut entry, &mut out_file)
                .with_context(|| format!("Failed to extract {:?}", entry_name))?;
            collected.push(out_path);
        } else if entry_lower.ends_with(".zip") {
            
            let nested_zip_path = out_dir.join(&file_name);
            let mut nested_file = File::create(&nested_zip_path)
                .with_context(|| format!("Failed to create nested zip {:?}", nested_zip_path))?;
            io::copy(&mut entry, &mut nested_file)?;
            drop(nested_file); 

            let sub_dir = out_dir.join(format!("nested_{}", i));
            fs::create_dir_all(&sub_dir)?;

            if let Err(e) = extract_zip_recursive(&nested_zip_path, &sub_dir, collected, depth + 1)
            {
                eprintln!(
                    "Warning: skipping nested zip {:?}: {}",
                    entry_name, e
                );
            }

            
            let _ = fs::remove_file(&nested_zip_path);
        }
        
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_fits_path() {
        assert!(is_fits_path(Path::new("data.fits")));
        assert!(is_fits_path(Path::new("data.FIT")));
        assert!(is_fits_path(Path::new("data.fts")));
        assert!(!is_fits_path(Path::new("data.zip")));
        assert!(!is_fits_path(Path::new("data.png")));
    }

    #[test]
    fn test_is_zip_path() {
        assert!(is_zip_path(Path::new("archive.zip")));
        assert!(is_zip_path(Path::new("archive.ZIP")));
        assert!(!is_zip_path(Path::new("data.fits")));
    }
}
