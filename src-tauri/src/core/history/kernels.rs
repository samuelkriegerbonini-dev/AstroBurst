use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ndarray::Array2;

use super::id::{is_hex_id, Fnv1a};
use super::{kernels_dir, write_atomic};

const KERNEL_EXTENSION: &str = "f32";
const BYTES_PER_VALUE: usize = 4;

pub fn kernel_hash(kernel: &Array2<f32>) -> String {
    let (rows, cols) = kernel.dim();
    Fnv1a::default()
        .update(&(rows as u64).to_le_bytes())
        .update(&(cols as u64).to_le_bytes())
        .update(&le_bytes(kernel))
        .hex()
}

fn le_bytes(kernel: &Array2<f32>) -> Vec<u8> {
    kernel.iter().flat_map(|v| v.to_le_bytes()).collect()
}

pub fn kernel_path(image_dir: &Path, hash: &str) -> io::Result<PathBuf> {
    if !is_hex_id(hash) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{hash:?} is not a kernel hash of 16 lowercase hex digits")));
    }
    Ok(kernels_dir(image_dir).join(format!("{hash}.{KERNEL_EXTENSION}")))
}

pub fn write_kernel(image_dir: &Path, kernel: &Array2<f32>) -> io::Result<String> {
    let hash = kernel_hash(kernel);
    let path = kernel_path(image_dir, &hash)?;
    let expected = (kernel.len() * BYTES_PER_VALUE) as u64;
    if fs::metadata(&path).is_ok_and(|meta| meta.len() == expected) {
        return Ok(hash);
    }
    fs::create_dir_all(kernels_dir(image_dir))?;
    write_atomic(&path, &le_bytes(kernel))?;
    Ok(hash)
}

pub fn read_kernel(image_dir: &Path, hash: &str, rows: usize, cols: usize) -> io::Result<Array2<f32>> {
    let bytes = fs::read(kernel_path(image_dir, hash)?)?;
    let expected = rows * cols * BYTES_PER_VALUE;
    if bytes.len() != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("kernel {hash} holds {} bytes, a {rows}×{cols} kernel needs {expected}", bytes.len()),
        ));
    }
    let values = bytes
        .chunks_exact(BYTES_PER_VALUE)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    Array2::from_shape_vec((rows, cols), values).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn kernel(size: usize, scale: f32) -> Array2<f32> {
        let c = (size - 1) as f32 / 2.0;
        Array2::from_shape_fn((size, size), |(r, k)| {
            let d2 = (r as f32 - c).powi(2) + (k as f32 - c).powi(2);
            scale * (-d2 / 50.0).exp()
        })
    }

    #[test]
    fn kernel_side_file_round_trips_by_hash() {
        let dir = tempfile::tempdir().unwrap();
        let psf = kernel(129, 1.0);
        let first = write_kernel(dir.path(), &psf).unwrap();
        let path = kernel_path(dir.path(), &first).unwrap();
        let earlier = SystemTime::now() - Duration::from_secs(3600);
        fs::File::options().write(true).open(&path).unwrap().set_modified(earlier).unwrap();

        let same_values = psf.clone();
        let second = write_kernel(dir.path(), &same_values).unwrap();
        assert_eq!(first, second, "two nodes with one kernel got two hashes");
        assert_eq!(first.len(), 16);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), earlier, "the kernel file was written twice");
        assert_eq!(fs::read_dir(kernels_dir(dir.path())).unwrap().count(), 1);
        assert_eq!(fs::metadata(&path).unwrap().len(), (129 * 129 * BYTES_PER_VALUE) as u64);

        let back = read_kernel(dir.path(), &first, 129, 129).unwrap();
        assert!(back.iter().zip(psf.iter()).all(|(a, b)| a.to_bits() == b.to_bits()), "kernel values changed on disk");

        let other = write_kernel(dir.path(), &kernel(129, 0.5)).unwrap();
        assert_ne!(other, first);
        assert_ne!(kernel_hash(&kernel(129, 1.0)), kernel_hash(&Array2::from_shape_vec((43, 387), psf.iter().copied().collect()).unwrap()));

        let err = read_kernel(dir.path(), &first, 128, 129).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains(&first), "{err}");
    }

    #[test]
    fn kernel_hashes_that_are_not_16_lowercase_hex_digits_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("0123456789abcdef");
        fs::create_dir_all(kernels_dir(&image)).unwrap();
        fs::write(image.join("escape.f32"), 1f32.to_le_bytes()).unwrap();
        for hash in ["../escape", "..\\escape", "0123456789ABCDEF", "0123456789abcde", "0123456789abcdef0", ""] {
            let err = read_kernel(&image, hash, 1, 1).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{hash:?}: {err}");
            assert_eq!(kernel_path(&image, hash).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        }
    }
}
