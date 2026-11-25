use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use astroburst_lib::core::analysis::deconvolution::{generate_gaussian_psf, richardson_lucy};
use astroburst_lib::core::imaging::wavelet::{wavelet_denoise, WaveletConfig};
use astroburst_lib::types::stacking::RLConfig;
use ndarray::Array2;

struct PeakCounting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

unsafe impl GlobalAlloc for PeakCounting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = System.alloc(layout);
        if !pointer.is_null() {
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        System.dealloc(pointer, layout);
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }
}

#[global_allocator]
static ALLOCATOR: PeakCounting = PeakCounting;

const SIDE: usize = 1024;
const PLANE_BYTES: usize = SIDE * SIDE * std::mem::size_of::<f32>();

fn peak_planes_above_start<T>(work: impl FnOnce() -> T) -> (T, f64) {
    let start = LIVE.load(Ordering::Relaxed);
    PEAK.store(start, Ordering::Relaxed);
    let out = work();
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(start);
    (out, peak as f64 / PLANE_BYTES as f64)
}

fn star_on_a_textured_sky(side: usize) -> Array2<f32> {
    let centre = side as f32 / 2.0;
    Array2::from_shape_fn((side, side), |(y, x)| {
        let d2 = (x as f32 - centre).powi(2) + (y as f32 - centre).powi(2);
        100.0 + ((y * 31 + x * 17) % 23) as f32 + 400.0 * (-d2 / 18.0).exp()
    })
}

#[test]
fn wavelet_denoise_peaks_below_six_planes() {
    let _guard = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let config = WaveletConfig::default();
    wavelet_denoise(&star_on_a_textured_sky(64), &config, None).unwrap();
    let image = star_on_a_textured_sky(SIDE);

    let (result, planes) = peak_planes_above_start(|| wavelet_denoise(&image, &config, None).unwrap());

    assert_eq!(result.denoised.dim(), (SIDE, SIDE));
    assert!(planes < 6.0, "wavelet denoise held {planes:.1} image planes at its peak");
}

#[test]
fn richardson_lucy_peaks_below_ten_planes() {
    let _guard = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let psf = generate_gaussian_psf(15, 2.0);
    let config = RLConfig { iterations: 2, ..RLConfig::default() };
    richardson_lucy(&star_on_a_textured_sky(64), &psf, &config, None).unwrap();
    let image = star_on_a_textured_sky(SIDE);

    let (result, planes) = peak_planes_above_start(|| richardson_lucy(&image, &psf, &config, None).unwrap());

    assert_eq!(result.image.dim(), (SIDE, SIDE));
    assert!(planes < 10.0, "Richardson-Lucy held {planes:.1} image planes at its peak");
}
