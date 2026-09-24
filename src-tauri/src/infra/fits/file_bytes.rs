// file_bytes — contributed by Jae-Joon Lee <https://github.com/leejjoon>

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::ops::Deref;
use std::str::FromStr;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use memmap2::{Mmap, MmapOptions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoMode {
    Auto,
    Mmap,
    Read,
}

impl FromStr for IoMode {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(IoMode::Auto),
            "mmap" => Ok(IoMode::Mmap),
            "read" => Ok(IoMode::Read),
            other => Err(format!("invalid io mode '{other}' (expected auto|mmap|read)")),
        }
    }
}

pub fn io_mode() -> IoMode {
    static MODE: OnceLock<IoMode> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("ASTROBURST_IO_MODE") {
        Err(_) => IoMode::Auto,
        Ok(raw) => raw.parse().unwrap_or_else(|e| {
            log::warn!("ASTROBURST_IO_MODE: {e}; using auto");
            IoMode::Auto
        }),
    })
}

pub enum FileBytes {
    Mapped(Mmap),
    Owned(Vec<u8>),
}

impl Deref for FileBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            FileBytes::Mapped(m) => m,
            FileBytes::Owned(v) => v,
        }
    }
}

impl AsRef<[u8]> for FileBytes {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

pub(crate) fn prefer_mmap(file: &File, mode: IoMode) -> bool {
    match mode {
        IoMode::Mmap => true,
        IoMode::Read => false,
        IoMode::Auto => !is_network_fs(file),
    }
}

pub fn read_file_bytes(file: &File) -> Result<FileBytes> {
    read_file_bytes_with_mode(file, io_mode())
}

pub fn read_file_bytes_with_mode(file: &File, mode: IoMode) -> Result<FileBytes> {
    let use_mmap = prefer_mmap(file, mode);

    if use_mmap {
        let mmap = unsafe { MmapOptions::new().map(file).context("mmap failed")? };
        #[cfg(unix)]
        {
            let _ = mmap.advise(memmap2::Advice::Sequential);
        }
        Ok(FileBytes::Mapped(mmap))
    } else {
        let len = file.metadata().map(|m| m.len() as usize).unwrap_or(0);
        let mut buf = Vec::with_capacity(len);
        let mut f = file;
        f.seek(SeekFrom::Start(0)).context("seek failed")?;
        f.read_to_end(&mut buf).context("file read failed")?;
        Ok(FileBytes::Owned(buf))
    }
}

#[cfg(target_os = "linux")]
const NETWORK_FS_MAGICS: &[u64] = &[
    0x6969,
    0x517B,
    0xFE534D42,
    0xFF534D42,
    0x0BD00BD0,
    0x00C36400,
    0x65735546,
    0x01021997,
    0x47504653,
    0x01161970,
    0x7461636F,
    0x5346414F,
    0x73757245,
];

#[cfg(target_os = "linux")]
fn is_network_fs(file: &File) -> bool {
    use std::os::unix::io::AsRawFd;
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::fstatfs(file.as_raw_fd(), &mut stat) };
    if rc != 0 {
        return false;
    }
    let ftype = stat.f_type as u64;
    NETWORK_FS_MAGICS.contains(&ftype)
}

#[cfg(windows)]
fn is_network_fs(file: &File) -> bool {
    use std::os::windows::io::AsRawHandle;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetFinalPathNameByHandleW(
            handle: *mut core::ffi::c_void,
            buf: *mut u16,
            len: u32,
            flags: u32,
        ) -> u32;
        fn GetDriveTypeW(root: *const u16) -> u32;
    }

    const DRIVE_REMOTE: u32 = 4;

    let mut buf = [0u16; 1024];
    let n = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle() as *mut core::ffi::c_void,
            buf.as_mut_ptr(),
            buf.len() as u32,
            0,
        )
    };
    if n == 0 || n as usize >= buf.len() {
        return false;
    }
    let path = String::from_utf16_lossy(&buf[..n as usize]);
    let p = path.strip_prefix(r"\\?\").unwrap_or(&path);
    if p.starts_with(r"UNC\") || p.starts_with(r"\\") {
        return true;
    }
    let bytes = p.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        let root: Vec<u16> = format!("{}:\\", bytes[0] as char)
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let drive_type = unsafe { GetDriveTypeW(root.as_ptr()) };
        return drive_type == DRIVE_REMOTE;
    }
    false
}

#[cfg(any(target_os = "macos", test))]
const MACOS_NETWORK_FS_TYPES: [&[u8]; 4] = [b"smbfs", b"nfs", b"afpfs", b"webdav"];

#[cfg(any(target_os = "macos", test))]
const STATFS64_BYTES: usize = 2168;
#[cfg(any(target_os = "macos", test))]
const STATFS64_WORDS: usize = STATFS64_BYTES.div_ceil(8);
#[cfg(any(target_os = "macos", test))]
const F_FSTYPENAME_OFFSET: usize = 72;
#[cfg(any(target_os = "macos", test))]
const F_FSTYPENAME_LEN: usize = 16;

#[cfg(any(target_os = "macos", test))]
fn is_network_fs_type_name(field: &[u8]) -> bool {
    let name = field.split(|&b| b == 0).next().unwrap_or(field);
    MACOS_NETWORK_FS_TYPES.contains(&name)
}

#[cfg(any(target_os = "macos", test))]
fn statfs64_fstypename(stat: &[u64; STATFS64_WORDS]) -> Vec<u8> {
    stat.iter()
        .flat_map(|word| word.to_ne_bytes())
        .skip(F_FSTYPENAME_OFFSET)
        .take(F_FSTYPENAME_LEN)
        .collect()
}

#[cfg(target_os = "macos")]
fn is_network_fs(file: &File) -> bool {
    use std::os::raw::c_int;
    use std::os::unix::io::AsRawFd;

    extern "C" {
        #[cfg_attr(not(target_arch = "aarch64"), link_name = "fstatfs$INODE64")]
        fn fstatfs(fd: c_int, buf: *mut u64) -> c_int;
    }

    let mut stat = [0u64; STATFS64_WORDS];
    if unsafe { fstatfs(file.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return false;
    }
    is_network_fs_type_name(&statfs64_fstypename(&stat))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn is_network_fs(_file: &File) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn io_mode_parses_expected_values() {
        assert_eq!("auto".parse::<IoMode>().unwrap(), IoMode::Auto);
        assert_eq!("MMAP".parse::<IoMode>().unwrap(), IoMode::Mmap);
        assert_eq!(" read ".parse::<IoMode>().unwrap(), IoMode::Read);
        assert!("fast".parse::<IoMode>().is_err());
    }

    #[test]
    fn mapped_and_owned_yield_identical_bytes() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        let content: Vec<u8> = (0..10_000u32).flat_map(|v| v.to_be_bytes()).collect();
        tmp.write_all(&content).unwrap();
        tmp.flush().unwrap();

        let file = File::open(tmp.path()).unwrap();
        let mapped = read_file_bytes_with_mode(&file, IoMode::Mmap).unwrap();
        let owned = read_file_bytes_with_mode(&file, IoMode::Read).unwrap();
        assert!(matches!(mapped, FileBytes::Mapped(_)));
        assert!(matches!(owned, FileBytes::Owned(_)));
        assert_eq!(&*mapped, &content[..]);
        assert_eq!(&*owned, &content[..]);
    }

    #[test]
    fn owned_read_is_independent_of_prior_cursor_position() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(b"0123456789").unwrap();
        tmp.flush().unwrap();

        let mut file = File::open(tmp.path()).unwrap();
        let mut probe = [0u8; 4];
        std::io::Read::read_exact(&mut file, &mut probe).unwrap();

        let owned = read_file_bytes_with_mode(&file, IoMode::Read).unwrap();
        assert_eq!(&*owned, b"0123456789");
    }

    fn fstypename_field(name: &str) -> [u8; 16] {
        let mut field = [0u8; 16];
        field[..name.len()].copy_from_slice(name.as_bytes());
        field
    }

    #[test]
    fn macos_network_filesystem_type_names_are_recognised() {
        for name in ["smbfs", "nfs", "afpfs", "webdav"] {
            assert!(is_network_fs_type_name(&fstypename_field(name)), "{name}");
        }
        for name in ["apfs", "hfs", "msdos", "exfat", "devfs", "nfsx", "smb", ""] {
            assert!(!is_network_fs_type_name(&fstypename_field(name)), "{name:?}");
        }
        assert!(is_network_fs_type_name(b"webdav"), "a field without a terminator is read to its end");
    }

    #[test]
    fn fstypename_is_read_from_its_statfs64_offset() {
        let mut bytes = [b'x'; STATFS64_WORDS * 8];
        bytes[F_FSTYPENAME_OFFSET..F_FSTYPENAME_OFFSET + F_FSTYPENAME_LEN].copy_from_slice(&fstypename_field("smbfs"));
        let mut stat = [0u64; STATFS64_WORDS];
        for (word, chunk) in stat.iter_mut().zip(bytes.chunks_exact(8)) {
            *word = u64::from_ne_bytes(chunk.try_into().unwrap());
        }
        assert_eq!(STATFS64_WORDS * 8, STATFS64_BYTES);
        assert_eq!(statfs64_fstypename(&stat), fstypename_field("smbfs"));
        assert!(is_network_fs_type_name(&statfs64_fstypename(&stat)));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_local_temporary_file_is_mapped_in_auto_mode_on_macos() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let file = File::open(tmp.path()).unwrap();
        assert!(!is_network_fs(&file));
        assert!(prefer_mmap(&file, IoMode::Auto));
    }
}
