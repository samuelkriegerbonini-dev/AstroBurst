pub(in crate::service::sagui) fn memory_estimate(pixels: u64, bands: u64, mode: &str, knn: u64) -> u64 {
    let cube = pixels.saturating_mul(bands).saturating_mul(64);
    let clustering = if mode == "ward" { pixels.saturating_mul(pixels.saturating_sub(1)).saturating_mul(12) }
        else { pixels.saturating_mul(knn).saturating_mul(128) };
    cube.saturating_add(clustering).saturating_add(128 * 1024 * 1024)
}

pub(in crate::service::sagui) fn footprint_memory(pixels: u64, bands: u64, mode: &str, knn: u64) -> u64 {
    let cube = pixels.saturating_mul(bands).saturating_mul(64);
    let graph = if mode == "sparse" { pixels.saturating_mul(knn).saturating_mul(128) } else { 0 };
    cube.saturating_add(graph).saturating_add(128 * 1024 * 1024)
}
