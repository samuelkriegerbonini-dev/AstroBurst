use std::collections::BTreeMap;
use ndarray::Array2;
use serde_json::json;
use crate::core::cubeprep::cube::{dq_table_of, Cube, CubePlane};
use crate::core::imaging::dq_flags::{bits_present, DqTable};
use crate::service::sagui::types::PlaneDqFlags;

fn all_finite(cube: &Cube) -> Vec<bool> {
    let (height, width) = cube.dims();
    (0..height * width).map(|i| cube.planes.iter().all(|p| p[(i / width, i % width)].is_finite())).collect()
}

pub(in crate::service::sagui) fn plane_dq_table(cube: &Cube, plane: &CubePlane) -> DqTable {
    dq_table_of(plane).or_else(|| DqTable::select(&cube.header))
}

fn flag_name(table: DqTable, bit: u32) -> String {
    table.decode(bit).into_iter().next().unwrap_or_default()
}

fn flagged_finite_counts(plane: &Array2<u32>, finite: &[bool], bits: u32, table: DqTable) -> BTreeMap<String, usize> {
    let present = bits_present(plane) & bits;
    (0..32).map(|n| 1u32 << n).filter(|bit| present & bit != 0).filter_map(|bit| {
        let count = plane.iter().enumerate().filter(|(i, &value)| finite[*i] && value & bit != 0).count();
        (count > 0).then(|| (flag_name(table, bit), count))
    }).collect()
}

pub(in crate::service::sagui) fn dq_flag_counts(cube: &Cube) -> Vec<PlaneDqFlags> {
    let Some(dq) = &cube.dq else { return Vec::new() };
    let finite = all_finite(cube);
    dq.planes.iter().zip(&cube.filters).map(|(plane, filter)| {
        let table = plane_dq_table(cube, filter);
        PlaneDqFlags { filter: filter.filter.clone(), table: table.label(), flagged_finite: flagged_finite_counts(plane, &finite, u32::MAX, table) }
    }).collect()
}

pub(in crate::service::sagui) fn dq_exclusion_report(cube: &Cube, bits: u32, eligible: &[bool]) -> (serde_json::Value, Vec<String>) {
    let finite = all_finite(cube);
    let finite_pixels = finite.iter().filter(|&&f| f).count();
    let available = eligible.iter().filter(|&&valid| valid).count();
    let mut warnings = Vec::new();
    let planes: Vec<serde_json::Value> = cube.dq.as_ref().map(|dq| dq.planes.iter().zip(&cube.filters).map(|(plane, filter)| {
        let table = plane_dq_table(cube, filter);
        let removed = plane.iter().enumerate().filter(|(i, &value)| finite[*i] && value & bits != 0).count();
        let flags = flagged_finite_counts(plane, &finite, bits, table);
        if removed > 0 {
            let detail = flags.iter().map(|(name, count)| format!("{name} {count}")).collect::<Vec<_>>().join(", ");
            warnings.push(format!("{}: DQ policy 0x{bits:08x} removes {removed} finite pixel{} ({detail})", filter.filter, if removed == 1 { "" } else { "s" }));
        }
        json!({ "filter": filter.filter, "table": table.label(), "removed_pixels": removed, "flags": flags })
    }).collect()).unwrap_or_default();
    (json!({ "bits": bits, "finite_pixels": finite_pixels, "excluded_pixels": finite_pixels - available, "planes": planes }), warnings)
}

pub(in crate::service::sagui) fn eligible(cube: &Cube, bits: u32) -> Vec<bool> {
    let (height, width) = cube.dims();
    (0..height * width).map(|i| cube.planes.iter().all(|p| p[(i / width, i % width)].is_finite()) && cube.dq.as_ref().is_none_or(|dq| dq.planes.iter().all(|p| p[(i / width, i % width)] & bits == 0))).collect()
}
