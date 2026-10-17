use std::fs::File;

use anyhow::{anyhow, bail, Context, Result};

use crate::infra::fits::gzip::resolve_gzip;
use crate::infra::fits::reader::{extract_header_by_index_merged, extract_header_mmap};
use crate::types::header::HduHeader;
use crate::types::image_ref::{ImageRef, PlaneSelector};
use crate::types::stacking::{DrizzleConfig, NormalizationMethod, StackConfig};

pub const MEMORY_BUDGET_ENV: &str = "ASTROBURST_MEMORY_BUDGET_MB";

const F32_BYTES: u64 = 4;
const WARN_FRACTION: f64 = 0.8;
const KIB: u64 = 1024;
const MIB: f64 = 1024.0 * 1024.0;
const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
const DRIZZLE_OUTPUT_PLANES: u64 = 5;
const STACK_WORK_PLANES: u64 = 4;
const PIPELINE_STATS_PLANES: u64 = 4;
const DRIZZLE_MIN_SCALE: f64 = 1.0;
const DRIZZLE_MAX_SCALE: f64 = 4.0;

const MEMORY_ADVICE: &str = "Stack fewer frames at a time and combine the partial stacks, turn alignment off if the frames are already registered, or close other applications. If the system can page this much, set ASTROBURST_MEMORY_BUDGET_MB to the memory to allow (in MB) and restart.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryInfo {
    pub physical_available: u64,
    pub commit_available: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSize {
    pub width: u64,
    pub height: u64,
}

impl FrameSize {
    pub fn pixels(self) -> u64 {
        self.width.saturating_mul(self.height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryJob<'a> {
    Stack { frames: usize, largest: FrameSize },
    Drizzle { frames: usize, largest: FrameSize },
    PipelineChannel(&'a str),
    PipelineMasters,
}

impl MemoryJob<'_> {
    fn subject(&self) -> String {
        match self {
            MemoryJob::Stack { frames, largest } => {
                format!("Stacking these {} frames of {}x{} needs", frames, largest.width, largest.height)
            }
            MemoryJob::Drizzle { frames, largest } => {
                format!("Drizzling these {} frames of {}x{} needs", frames, largest.width, largest.height)
            }
            MemoryJob::PipelineChannel(label) => format!("Channel '{}' of the Pipeline needs", label),
            MemoryJob::PipelineMasters => "Building the calibration masters of the Pipeline needs".to_string(),
        }
    }

    fn contents(&self) -> &'static str {
        match self {
            MemoryJob::PipelineMasters => "the calibration frames and the masters",
            _ => "the frames, their aligned copies and the result",
        }
    }
}

fn meminfo_kib(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.trim() != key {
            return None;
        }
        value.split_whitespace().next()?.parse().ok()
    })
}

pub fn parse_meminfo(text: &str) -> Option<MemoryInfo> {
    let available = meminfo_kib(text, "MemAvailable")?;
    let swap_free = meminfo_kib(text, "SwapFree").unwrap_or(0);
    Some(MemoryInfo {
        physical_available: available.saturating_mul(KIB),
        commit_available: available.saturating_add(swap_free).saturating_mul(KIB),
    })
}

pub fn macos_info(memsize: u64, vm_free: u64) -> MemoryInfo {
    MemoryInfo { physical_available: memsize, commit_available: memsize.saturating_add(vm_free) }
}

#[cfg(target_os = "linux")]
fn system_memory() -> Option<MemoryInfo> {
    std::fs::read_to_string("/proc/meminfo").ok().as_deref().and_then(parse_meminfo)
}

#[cfg(target_os = "macos")]
fn volume_free_bytes(path: &std::ffi::CStr) -> Option<u64> {
    let mut stats: libc::statfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statfs(path.as_ptr(), &mut stats) };
    (rc == 0).then(|| stats.f_bavail.saturating_mul(stats.f_bsize as u64))
}

#[cfg(target_os = "macos")]
fn system_memory() -> Option<MemoryInfo> {
    let mut memsize: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    let rc = unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&mut memsize as *mut u64).cast::<libc::c_void>(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || memsize == 0 {
        return None;
    }
    let vm_free = volume_free_bytes(c"/System/Volumes/VM").or_else(|| volume_free_bytes(c"/")).unwrap_or(0);
    Some(macos_info(memsize, vm_free))
}

#[cfg(windows)]
fn system_memory() -> Option<MemoryInfo> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    (ok != 0).then_some(MemoryInfo { physical_available: status.ullAvailPhys, commit_available: status.ullAvailPageFile })
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn system_memory() -> Option<MemoryInfo> {
    None
}

pub fn budget_from_env_value(value: &str) -> Option<f64> {
    let mb: f64 = value.trim().parse().ok()?;
    (mb.is_finite() && mb > 0.0).then_some(mb)
}

pub fn apply_budget(info: Option<MemoryInfo>, budget_mb: Option<f64>) -> Option<MemoryInfo> {
    match budget_mb {
        Some(mb) => {
            let bytes = (mb * MIB) as u64;
            Some(MemoryInfo { physical_available: bytes, commit_available: bytes })
        }
        None => info,
    }
}

pub fn effective_memory() -> Option<MemoryInfo> {
    let budget = std::env::var(MEMORY_BUDGET_ENV).ok().as_deref().and_then(budget_from_env_value);
    apply_budget(system_memory(), budget)
}

pub fn header_size(header: &HduHeader) -> FrameSize {
    let axis = |key: &str| header.get_i64(key).map_or(0, |v| v.max(0) as u64);
    let compressed = header.get("ZIMAGE").is_some_and(|v| v.trim() == "T") && header.get("ZNAXIS1").is_some();
    if compressed {
        FrameSize { width: axis("ZNAXIS1"), height: axis("ZNAXIS2") }
    } else {
        FrameSize { width: axis("NAXIS1"), height: axis("NAXIS2") }
    }
}

fn frame_size(path: &str) -> Result<FrameSize> {
    let reference = ImageRef::parse(path);
    let (readable, _decompressed) = resolve_gzip(std::path::Path::new(&reference.path))?;
    let file = File::open(&readable).with_context(|| format!("Failed to open {}", reference.path))?;
    let header = match &reference.plane {
        PlaneSelector::Auto => extract_header_mmap(&file),
        PlaneSelector::Hdu(n) => extract_header_by_index_merged(&file, *n),
        PlaneSelector::Array(_) => bail!("FITS files have no ASDF arrays; use #hdu=<n>: {}", path),
    }
    .with_context(|| format!("Failed to read the header of {}", path))?;
    Ok(header_size(&header))
}

pub fn frame_sizes(paths: &[String]) -> Result<Vec<FrameSize>> {
    paths.iter().map(|p| frame_size(p)).collect()
}

pub fn pixel_counts(sizes: &[FrameSize]) -> Vec<u64> {
    sizes.iter().map(|s| s.pixels()).collect()
}

pub fn largest_frame(sizes: &[FrameSize]) -> FrameSize {
    sizes.iter().copied().max_by_key(|s| s.pixels()).unwrap_or(FrameSize { width: 0, height: 0 })
}

fn sum_and_max(pixels: &[u64]) -> (u64, u64) {
    let sum = pixels.iter().fold(0u64, |acc, &p| acc.saturating_add(p));
    (sum, pixels.iter().copied().max().unwrap_or(0))
}

pub fn stack_bytes(pixels: &[u64], copies: bool) -> u64 {
    let (sum, max) = sum_and_max(pixels);
    let frames = sum.saturating_mul(1 + copies as u64);
    F32_BYTES.saturating_mul(frames.saturating_add(STACK_WORK_PLANES.saturating_mul(max)))
}

pub fn drizzle_bytes(pixels: &[u64], align: bool, scale: f64) -> u64 {
    let (sum, max) = sum_and_max(pixels);
    let scale = scale.clamp(DRIZZLE_MIN_SCALE, DRIZZLE_MAX_SCALE);
    let area = (scale * scale).ceil() as u64;
    let frames = sum.saturating_mul(1 + align as u64);
    let output = max.saturating_mul(area).saturating_mul(DRIZZLE_OUTPUT_PLANES);
    F32_BYTES.saturating_mul(frames.saturating_add(output))
}

#[derive(Debug, Clone, Copy)]
pub struct PipelineShape<'a> {
    pub channels: &'a [Vec<u64>],
    pub calibration_pixels: &'a [u64],
    pub largest_calibration_set: usize,
    pub masters: usize,
    pub align: bool,
    pub normalize: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelinePeak {
    Masters,
    Channel(usize),
}

pub fn pipeline_bytes(shape: &PipelineShape) -> (u64, PipelinePeak) {
    let max = shape.channels.iter().flatten().chain(shape.calibration_pixels).copied().max().unwrap_or(0);
    let plane = F32_BYTES.saturating_mul(max);
    let masters = plane.saturating_mul(shape.masters as u64);
    let build = plane.saturating_mul((shape.largest_calibration_set + shape.masters) as u64);
    let mut peak = (build, PipelinePeak::Masters);
    let copies = 2 + shape.align as u64 + shape.normalize as u64;
    for (index, channel) in shape.channels.iter().enumerate() {
        let (sum, _) = sum_and_max(channel);
        let frames = F32_BYTES.saturating_mul(sum.saturating_mul(copies));
        let work = plane.saturating_mul(PIPELINE_STATS_PLANES);
        let earlier = plane.saturating_mul(index as u64);
        let total = masters.saturating_add(frames).saturating_add(work).saturating_add(earlier);
        if total > peak.0 {
            peak = (total, PipelinePeak::Channel(index));
        }
    }
    peak
}

pub fn format_bytes(bytes: u64) -> String {
    let value = bytes as f64;
    if value >= GIB {
        format!("{:.1} GB", value / GIB)
    } else {
        format!("{:.1} MB", value / MIB)
    }
}

pub fn check_memory(required: u64, info: Option<MemoryInfo>, what: &MemoryJob) -> Result<Option<String>> {
    let Some(info) = info else {
        return Ok(None);
    };
    if required > info.commit_available {
        return Err(anyhow!(
            "{} about {} of memory ({}) but only {} is available. {}",
            what.subject(),
            format_bytes(required),
            what.contents(),
            format_bytes(info.commit_available),
            MEMORY_ADVICE
        ));
    }
    if required as f64 > WARN_FRACTION * info.physical_available as f64 {
        return Ok(Some(format!(
            "This run needs about {} of memory and {} is free; the system may page to disk and slow down.",
            format_bytes(required),
            format_bytes(info.physical_available)
        )));
    }
    Ok(None)
}

pub fn stack_copies(config: &StackConfig) -> bool {
    config.align || config.normalization != NormalizationMethod::None
}

pub fn stack_memory_check(paths: &[String], config: &StackConfig, info: Option<MemoryInfo>) -> Result<Option<String>> {
    if info.is_none() {
        return Ok(None);
    }
    let sizes = frame_sizes(paths)?;
    let required = stack_bytes(&pixel_counts(&sizes), stack_copies(config));
    check_memory(required, info, &MemoryJob::Stack { frames: sizes.len(), largest: largest_frame(&sizes) })
}

pub fn drizzle_memory_check(paths: &[String], config: &DrizzleConfig, info: Option<MemoryInfo>) -> Result<Option<String>> {
    drizzle_channels_memory_check(&[paths], config, info)
}

pub fn drizzle_channels_memory_check(
    channels: &[&[String]],
    config: &DrizzleConfig,
    info: Option<MemoryInfo>,
) -> Result<Option<String>> {
    if info.is_none() {
        return Ok(None);
    }
    let mut required = 0u64;
    let mut all: Vec<FrameSize> = Vec::new();
    for paths in channels {
        let sizes = frame_sizes(paths)?;
        required = required.saturating_add(drizzle_bytes(&pixel_counts(&sizes), config.align, config.scale));
        all.extend(sizes);
    }
    check_memory(required, info, &MemoryJob::Drizzle { frames: all.len(), largest: largest_frame(&all) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array2;

    const GIB_BYTES: u64 = 1 << 30;

    fn info(physical_gib: u64, commit_gib: u64) -> Option<MemoryInfo> {
        Some(MemoryInfo { physical_available: physical_gib * GIB_BYTES, commit_available: commit_gib * GIB_BYTES })
    }

    fn stack_job() -> MemoryJob<'static> {
        MemoryJob::Stack { frames: 150, largest: FrameSize { width: 6000, height: 4000 } }
    }

    #[test]
    fn refuses_when_the_estimate_exceeds_the_commit_limit() {
        let err = check_memory(30 * GIB_BYTES, info(16, 20), &stack_job()).unwrap_err().to_string();
        for needle in [
            "Stacking these 150 frames of 6000x4000",
            "needs about 30.0 GB",
            "20.0 GB is available",
            "Stack fewer frames",
            "ASTROBURST_MEMORY_BUDGET_MB",
        ] {
            assert!(err.contains(needle), "{needle:?} missing from {err}");
        }
        let drizzle = MemoryJob::Drizzle { frames: 6, largest: FrameSize { width: 1024, height: 1024 } };
        let err = check_memory(3 * GIB_BYTES, info(1, 1), &drizzle).unwrap_err().to_string();
        assert!(err.starts_with("Drizzling these 6 frames of 1024x1024 needs about 3.0 GB"), "{err}");
        let channel = check_memory(3 * GIB_BYTES, info(1, 1), &MemoryJob::PipelineChannel("L")).unwrap_err().to_string();
        assert!(channel.starts_with("Channel 'L' of the Pipeline needs about 3.0 GB"), "{channel}");
    }

    #[test]
    fn warns_above_eighty_percent_of_free_physical_memory() {
        let warning = check_memory(14 * GIB_BYTES, info(16, 40), &stack_job()).unwrap();
        let text = warning.expect("14 GB of 16 GB free must warn");
        assert!(text.contains("may page to disk"), "{text}");
        assert!(text.contains("needs about 14.0 GB") && text.contains("16.0 GB is free"), "{text}");
    }

    #[test]
    fn passes_below_the_threshold_and_without_memory_info() {
        assert_eq!(check_memory(12 * GIB_BYTES, info(16, 40), &stack_job()).unwrap(), None);
        assert_eq!(check_memory(u64::MAX, None, &stack_job()).unwrap(), None);
        assert!(check_memory(30 * GIB_BYTES, info(16, 20), &stack_job()).is_err());
    }

    #[test]
    fn sizes_are_shown_in_gb_from_one_gib_and_in_mb_below() {
        assert_eq!(format_bytes(GIB_BYTES), "1.0 GB");
        assert_eq!(format_bytes(GIB_BYTES - 1), "1024.0 MB");
        assert_eq!(format_bytes(3 * 1024 * 1024 / 2), "1.5 MB");
    }

    #[test]
    fn stack_estimate_counts_each_loaded_frame_and_its_aligned_copy() {
        let frames = vec![24_000_000u64; 150];
        assert_eq!(stack_bytes(&frames, true), 4 * 24_000_000 * 304);
        assert_eq!(stack_bytes(&frames, true), 29_184_000_000);
        assert_eq!(stack_bytes(&frames, false), 4 * 24_000_000 * 154);
    }

    #[test]
    fn stack_estimate_uses_every_frame_size_not_the_first_one() {
        assert_eq!(stack_bytes(&[100, 10_000], true), 4 * (10_100 * 2 + 4 * 10_000));
        assert_eq!(stack_bytes(&[10_000, 100], true), 4 * (10_100 * 2 + 4 * 10_000));
    }

    #[test]
    fn stack_copies_follow_alignment_and_normalization_clones() {
        let base = StackConfig::default();
        assert!(stack_copies(&StackConfig { align: true, normalization: NormalizationMethod::None, ..base.clone() }));
        assert!(stack_copies(&StackConfig { align: false, normalization: NormalizationMethod::Additive, ..base.clone() }));
        assert!(!stack_copies(&StackConfig { align: false, normalization: NormalizationMethod::None, ..base }));
    }

    #[test]
    fn drizzle_estimate_counts_frames_copies_and_five_planes_per_scaled_output_pixel() {
        let frames = [1_000u64, 1_000, 2_000];
        let output = 4 * 2_000 * 4 * 5;
        assert_eq!(drizzle_bytes(&frames, true, 2.0), 4 * 4_000 * 2 + output);
        assert_eq!(drizzle_bytes(&frames, false, 2.0), 4 * 4_000 + output);
        assert_eq!(drizzle_bytes(&frames, false, 1.5), 4 * 4_000 + 4 * 2_000 * 3 * 5);
        assert_eq!(drizzle_bytes(&frames, false, 9.0), 4 * 4_000 + 4 * 2_000 * 16 * 5);
    }

    #[test]
    fn pipeline_estimate_takes_the_master_build_peak_or_the_largest_channel() {
        let lights = vec![vec![100u64; 3]];
        let masters_heavy = PipelineShape {
            channels: &lights,
            calibration_pixels: &[100; 50],
            largest_calibration_set: 50,
            masters: 3,
            align: true,
            normalize: true,
        };
        assert_eq!(pipeline_bytes(&masters_heavy), (4 * 100 * 53, PipelinePeak::Masters));

        let channels = vec![vec![100u64; 2], vec![100u64; 10]];
        let late_channel = PipelineShape {
            channels: &channels,
            calibration_pixels: &[100; 4],
            largest_calibration_set: 4,
            masters: 2,
            align: true,
            normalize: false,
        };
        let channel_two = 4 * 100 * 2 + 4 * (1_000 * 3 + 4 * 100) + 4 * 100;
        assert_eq!(pipeline_bytes(&late_channel), (channel_two, PipelinePeak::Channel(1)));

        let mixed = vec![vec![100u64; 2], vec![400u64; 1]];
        let shape = PipelineShape {
            channels: &mixed,
            calibration_pixels: &[],
            largest_calibration_set: 0,
            masters: 0,
            align: false,
            normalize: false,
        };
        let first = 4 * (200 * 2 + 4 * 400);
        let second = 4 * (400 * 2 + 4 * 400) + 4 * 400;
        assert!(second > first);
        assert_eq!(pipeline_bytes(&shape), (second, PipelinePeak::Channel(1)));
    }

    #[test]
    fn budget_value_replaces_both_figures() {
        let measured = info(16, 20);
        let raised = apply_budget(measured, Some(40.0 * 1024.0)).unwrap();
        assert_eq!(raised, MemoryInfo { physical_available: 40 * GIB_BYTES, commit_available: 40 * GIB_BYTES });
        let lowered = apply_budget(measured, Some(1.0)).unwrap();
        assert_eq!(lowered, MemoryInfo { physical_available: 1 << 20, commit_available: 1 << 20 });
        assert_eq!(apply_budget(measured, None), measured);
        assert_eq!(apply_budget(None, None), None);
        assert_eq!(apply_budget(None, Some(512.0)), Some(MemoryInfo { physical_available: 512 << 20, commit_available: 512 << 20 }));
    }

    #[test]
    fn budget_env_value_accepts_only_positive_finite_megabytes() {
        assert_eq!(budget_from_env_value("2048"), Some(2048.0));
        assert_eq!(budget_from_env_value(" 1.5 "), Some(1.5));
        for bad in ["", "0", "-5", "abc", "NaN", "inf", "2,5"] {
            assert_eq!(budget_from_env_value(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn parse_meminfo_reads_available_memory_and_swap() {
        let sample = "MemTotal:       16318412 kB\nMemFree:          612340 kB\nMemAvailable:    8000000 kB\nBuffers:          123456 kB\nSwapTotal:       2097148 kB\nSwapFree:        2000000 kB\n";
        assert_eq!(
            parse_meminfo(sample),
            Some(MemoryInfo { physical_available: 8_000_000 * KIB, commit_available: 10_000_000 * KIB })
        );
        let no_swap = "MemTotal: 100 kB\nMemAvailable: 50 kB\n";
        assert_eq!(parse_meminfo(no_swap), Some(MemoryInfo { physical_available: 50 * KIB, commit_available: 50 * KIB }));
        assert_eq!(parse_meminfo("MemTotal: 100 kB\nMemFree: 50 kB\nSwapFree: 10 kB\n"), None);
        assert_eq!(parse_meminfo(""), None);
    }

    #[test]
    fn macos_figures_use_total_ram_and_the_vm_volume() {
        assert_eq!(
            macos_info(16 * GIB_BYTES, 50 * GIB_BYTES),
            MemoryInfo { physical_available: 16 * GIB_BYTES, commit_available: 66 * GIB_BYTES }
        );
        assert_eq!(macos_info(8 * GIB_BYTES, 0), MemoryInfo { physical_available: 8 * GIB_BYTES, commit_available: 8 * GIB_BYTES });
    }

    fn truncated_frame(dir: &tempfile::TempDir, name: &str, rows: usize, cols: usize) -> String {
        let path = dir.path().join(name).to_str().unwrap().to_string();
        crate::infra::fits::writer::write_fits_mono(&path, &Array2::from_elem((rows, cols), 1.0f32), None).unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        let len = file.metadata().unwrap().len();
        file.set_len(len - 2880).unwrap();
        path
    }

    #[test]
    fn frame_pixels_reads_headers_only() {
        use crate::infra::fits::reader::test_fixtures::{write_test_mef, HduData, TestHdu};
        let dir = tempfile::tempdir().unwrap();
        let mono = dir.path().join("mono.fits").to_str().unwrap().to_string();
        crate::infra::fits::writer::write_fits_mono(&mono, &Array2::from_elem((5, 7), 1.0f32), None).unwrap();
        let mef = dir.path().join("mef.fits");
        write_test_mef(
            &mef,
            &[],
            &[
                TestHdu { extname: Some("SCI"), extver: Some(1), cols: 4, rows: 3, data: HduData::F32(vec![0.0; 12]), extra_cards: vec![] },
                TestHdu { extname: Some("ERR"), extver: Some(1), cols: 6, rows: 2, data: HduData::F32(vec![0.0; 12]), extra_cards: vec![] },
            ],
        );
        let plane = crate::types::image_ref::ImageRef::hdu(mef.to_str().unwrap(), 2).cache_key();
        let truncated = truncated_frame(&dir, "truncated.fits", 64, 80);
        assert!(crate::infra::fits::reader::load_fits_image(&truncated).is_err(), "the fixture must not load as an image");

        let sizes = frame_sizes(&[mono, plane, truncated]).unwrap();
        assert_eq!(pixel_counts(&sizes), vec![35, 12, 5_120]);
        assert_eq!(sizes[0], FrameSize { width: 7, height: 5 });
        assert_eq!(sizes[1], FrameSize { width: 6, height: 2 });
        assert_eq!(largest_frame(&sizes), FrameSize { width: 80, height: 64 });
        assert!(frame_sizes(&[dir.path().join("missing.fits").to_str().unwrap().to_string()]).is_err());
    }

    #[test]
    fn compressed_headers_report_the_decompressed_image_size() {
        let mut header = HduHeader::empty();
        for (key, value) in [("NAXIS1", "8"), ("NAXIS2", "40"), ("ZIMAGE", "T"), ("ZNAXIS1", "300"), ("ZNAXIS2", "200")] {
            header.set(key, value.to_string());
        }
        assert_eq!(header_size(&header), FrameSize { width: 300, height: 200 });
        header.set("ZIMAGE", "F".to_string());
        assert_eq!(header_size(&header), FrameSize { width: 8, height: 40 });
    }

    #[test]
    fn stack_check_reads_headers_and_refuses_without_loading() {
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<String> = (0..2).map(|i| truncated_frame(&dir, &format!("t{i}.fits"), 64, 80)).collect();
        let tiny = Some(MemoryInfo { physical_available: KIB, commit_available: KIB });
        let config = StackConfig { align: true, ..StackConfig::default() };
        let err = stack_memory_check(&paths, &config, tiny).unwrap_err().to_string();
        assert!(err.starts_with("Stacking these 2 frames of 80x64 needs about"), "{err}");
        assert_eq!(stack_memory_check(&paths, &config, None).unwrap(), None);
        let drizzle = DrizzleConfig { align: false, scale: 2.0, ..DrizzleConfig::default() };
        let err = drizzle_memory_check(&paths, &drizzle, tiny).unwrap_err().to_string();
        assert!(err.starts_with("Drizzling these 2 frames of 80x64 needs about"), "{err}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_reports_free_physical_memory_and_commit_headroom() {
        let measured = system_memory().expect("GlobalMemoryStatusEx must report memory on Windows");
        assert!(measured.physical_available > 0, "{measured:?}");
        assert!(measured.commit_available > 0, "{measured:?}");
    }
}
