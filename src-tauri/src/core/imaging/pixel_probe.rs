use ndarray::Array2;
use serde_json::{json, Value};

use crate::core::imaging::dq_flags::DqTable;
use crate::math::exact_median_f64;
use crate::types::constants::{
    HEADER_BUNIT, RES_BITS, RES_BOX, RES_DQ, RES_ERR, RES_MAX, RES_MEAN, RES_MEDIAN, RES_MIN,
    RES_NAMES, RES_NEIGHBORHOOD, RES_N_NAN, RES_N_PIXELS, RES_TABLE, RES_TEXT, RES_UNIT,
    RES_VALUE, RES_X, RES_Y,
};
use crate::types::header::HduHeader;
use crate::types::image::IntPlane;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DqProbe {
    pub bits: u32,
    pub value: i64,
    pub names: Vec<String>,
    pub table: DqTable,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CompanionProbe {
    pub dq: Option<DqProbe>,
    pub err: Option<f64>,
}

fn in_bounds(rows: usize, cols: usize, x: i64, y: i64) -> Option<(usize, usize)> {
    if x < 0 || y < 0 || x >= cols as i64 || y >= rows as i64 {
        return None;
    }
    Some((y as usize, x as usize))
}

pub fn probe_companions(
    dq: Option<(&IntPlane, DqTable)>,
    err: Option<&Array2<f32>>,
    x: i64,
    y: i64,
) -> CompanionProbe {
    let dq = dq.and_then(|(plane, table)| {
        let (rows, cols) = plane.bits.dim();
        let (yy, xx) = in_bounds(rows, cols, x, y)?;
        let bits = plane.bits[[yy, xx]];
        Some(DqProbe {
            bits,
            value: plane.value_at(yy, xx),
            names: table.decode(bits),
            table,
            text: table.format(bits),
        })
    });
    let err = err.and_then(|arr| {
        let (rows, cols) = arr.dim();
        let (yy, xx) = in_bounds(rows, cols, x, y)?;
        let v = arr[[yy, xx]] as f64;
        v.is_finite().then_some(v)
    });
    CompanionProbe { dq, err }
}

pub fn probe_json_with_companions(
    probe: &PixelProbe,
    unit: Option<&str>,
    err_unit: Option<&str>,
    comp: &CompanionProbe,
) -> Value {
    let mut out = probe_json(probe, unit);
    let dq = match &comp.dq {
        Some(d) => json!({
            RES_BITS: d.bits,
            RES_VALUE: d.value,
            RES_NAMES: d.names,
            RES_TABLE: d.table,
            RES_TEXT: d.text,
        }),
        None => Value::Null,
    };
    let err = match comp.err {
        Some(v) => json!({ RES_VALUE: v, RES_UNIT: err_unit }),
        None => Value::Null,
    };
    if let Some(obj) = out.as_object_mut() {
        obj.insert(RES_DQ.to_string(), dq);
        obj.insert(RES_ERR.to_string(), err);
    }
    out
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ProbeError {
    #[error("pixel ({x}, {y}) is outside the image extent {cols}×{rows}")]
    OutOfBounds {
        x: f64,
        y: f64,
        cols: usize,
        rows: usize,
    },
    #[error("box must be an odd positive integer, got {0}")]
    EvenBox(u32),
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct NeighborhoodStats {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub mean: Option<f64>,
    pub median: Option<f64>,
    pub n_pixels: u64,
    pub n_nan: u64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PixelProbe {
    pub x: i64,
    pub y: i64,
    pub value: Option<f64>,
    pub box_size: u32,
    pub neighborhood: NeighborhoodStats,
}

pub fn box_half(box_size: u32) -> Result<i64, ProbeError> {
    if box_size.is_multiple_of(2) {
        return Err(ProbeError::EvenBox(box_size));
    }
    Ok((box_size / 2) as i64)
}

pub fn probe_pixel(
    arr: &Array2<f32>,
    x: f64,
    y: f64,
    box_size: u32,
) -> Result<PixelProbe, ProbeError> {
    let (rows, cols) = arr.dim();
    let out_of_bounds = ProbeError::OutOfBounds { x, y, cols, rows };
    if !x.is_finite() || !y.is_finite() {
        return Err(out_of_bounds);
    }
    let cx = x.floor() as i64;
    let cy = y.floor() as i64;
    if cx < 0 || cy < 0 || cx >= cols as i64 || cy >= rows as i64 {
        return Err(out_of_bounds);
    }

    let half = box_half(box_size)?;
    let x0 = (cx - half).max(0);
    let x1 = (cx + half).min(cols as i64 - 1);
    let y0 = (cy - half).max(0);
    let y1 = (cy + half).min(rows as i64 - 1);

    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    let mut sum = 0.0f64;
    let mut n_nan: u64 = 0;
    let mut finite: Vec<f64> = Vec::with_capacity(((x1 - x0 + 1) * (y1 - y0 + 1)) as usize);
    for yy in y0..=y1 {
        for xx in x0..=x1 {
            let v = arr[[yy as usize, xx as usize]] as f64;
            if v.is_finite() {
                min = min.min(v);
                max = max.max(v);
                sum += v;
                finite.push(v);
            } else {
                n_nan += 1;
            }
        }
    }

    let n_pixels = finite.len() as u64;
    let neighborhood = if n_pixels > 0 {
        NeighborhoodStats {
            min: Some(min),
            max: Some(max),
            mean: Some(sum / n_pixels as f64),
            median: Some(exact_median_f64(&finite)),
            n_pixels,
            n_nan,
        }
    } else {
        NeighborhoodStats {
            min: None,
            max: None,
            mean: None,
            median: None,
            n_pixels,
            n_nan,
        }
    };

    let centre = arr[[cy as usize, cx as usize]] as f64;
    Ok(PixelProbe {
        x: cx,
        y: cy,
        value: centre.is_finite().then_some(centre),
        box_size,
        neighborhood,
    })
}

pub fn data_unit(header: &HduHeader) -> Option<String> {
    let raw = header.get(HEADER_BUNIT)?.trim();
    let stripped = raw
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .unwrap_or(raw)
        .trim();
    (!stripped.is_empty()).then(|| stripped.to_string())
}

pub fn probe_json(probe: &PixelProbe, unit: Option<&str>) -> Value {
    let nb = &probe.neighborhood;
    json!({
        RES_X: probe.x,
        RES_Y: probe.y,
        RES_VALUE: probe.value,
        RES_UNIT: unit,
        RES_BOX: probe.box_size,
        RES_NEIGHBORHOOD: {
            RES_MIN: nb.min,
            RES_MAX: nb.max,
            RES_MEAN: nb.mean,
            RES_MEDIAN: nb.median,
            RES_N_PIXELS: nb.n_pixels,
            RES_N_NAN: nb.n_nan,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp_7x7() -> Array2<f32> {
        Array2::from_shape_fn((7, 7), |(y, x)| (y * 7 + x) as f32)
    }

    #[test]
    fn even_box_sizes_are_rejected_instead_of_silently_widening() {
        for even in [0u32, 2, 4, 10] {
            assert_eq!(box_half(even), Err(ProbeError::EvenBox(even)), "box {even}");
        }
        assert_eq!(
            ProbeError::EvenBox(4).to_string(),
            "box must be an odd positive integer, got 4"
        );
    }

    #[test]
    fn odd_box_sizes_give_a_centred_window() {
        assert_eq!(box_half(1).unwrap(), 0);
        assert_eq!(box_half(3).unwrap(), 1);
        assert_eq!(box_half(5).unwrap(), 2);
        assert_eq!(box_half(21).unwrap(), 10);
    }

    #[test]
    fn five_by_five_window_on_ramp_reports_full_stats() {
        let arr = ramp_7x7();
        let p = probe_pixel(&arr, 3.7, 3.2, 5).unwrap();
        assert_eq!((p.x, p.y), (3, 3));
        assert_eq!(p.value, Some(24.0));
        assert_eq!(p.box_size, 5);
        let nb = &p.neighborhood;
        assert_eq!(nb.min, Some(8.0));
        assert_eq!(nb.max, Some(40.0));
        assert_eq!(nb.mean, Some(24.0));
        assert_eq!(nb.median, Some(24.0));
        assert_eq!(nb.n_pixels, 25);
        assert_eq!(nb.n_nan, 0);
    }

    #[test]
    fn window_clipped_at_the_corner_counts_only_on_image_pixels() {
        let arr = ramp_7x7();
        let p = probe_pixel(&arr, 0.0, 0.0, 5).unwrap();
        assert_eq!(p.value, Some(0.0));
        assert_eq!(p.neighborhood.n_pixels, 9);
        assert_eq!(p.neighborhood.max, Some(16.0));
        assert_eq!(p.neighborhood.median, Some(8.0));

        let p = probe_pixel(&arr, 6.0, 6.0, 3).unwrap();
        assert_eq!(p.neighborhood.n_pixels, 4);
        assert_eq!(p.neighborhood.min, Some(40.0));
        assert_eq!(p.neighborhood.max, Some(48.0));
    }

    #[test]
    fn box_of_one_reports_only_the_centre() {
        let arr = ramp_7x7();
        let p = probe_pixel(&arr, 2.0, 5.0, 1).unwrap();
        assert_eq!(p.value, Some(37.0));
        assert_eq!(p.neighborhood.n_pixels, 1);
        assert_eq!(p.neighborhood.min, Some(37.0));
        assert_eq!(p.neighborhood.median, Some(37.0));
    }

    #[test]
    fn nan_centre_gives_none_value_and_is_counted() {
        let mut arr = ramp_7x7();
        arr[[3, 3]] = f32::NAN;
        arr[[2, 2]] = f32::INFINITY;
        let p = probe_pixel(&arr, 3.0, 3.0, 3).unwrap();
        assert_eq!(p.value, None);
        assert_eq!(p.neighborhood.n_nan, 2);
        assert_eq!(p.neighborhood.n_pixels, 7);
        let finite = [17.0, 18.0, 23.0, 25.0, 30.0, 31.0, 32.0];
        assert_eq!(p.neighborhood.min, Some(17.0));
        assert_eq!(p.neighborhood.max, Some(32.0));
        assert_eq!(p.neighborhood.mean, Some(finite.iter().sum::<f64>() / 7.0));
        assert_eq!(p.neighborhood.median, Some(25.0));

        let all_nan = Array2::from_elem((3, 3), f32::NAN);
        let p = probe_pixel(&all_nan, 1.0, 1.0, 3).unwrap();
        assert_eq!(p.value, None);
        assert_eq!(p.neighborhood.n_pixels, 0);
        assert_eq!(p.neighborhood.n_nan, 9);
        assert_eq!(p.neighborhood.median, None);
    }

    #[test]
    fn out_of_bounds_and_non_finite_coordinates_error() {
        let arr = ramp_7x7();
        for (x, y) in [(-0.5, 3.0), (3.0, -1.0), (7.0, 3.0), (3.0, 7.0), (100.0, 100.0)] {
            assert_eq!(
                probe_pixel(&arr, x, y, 3),
                Err(ProbeError::OutOfBounds { x, y, cols: 7, rows: 7 }),
                "({x}, {y})"
            );
        }
        assert!(matches!(
            probe_pixel(&arr, f64::NAN, 1.0, 3),
            Err(ProbeError::OutOfBounds { .. })
        ));
        assert_eq!(
            ProbeError::OutOfBounds { x: 100.0, y: 100.0, cols: 7, rows: 7 }.to_string(),
            "pixel (100, 100) is outside the image extent 7×7"
        );
    }

    fn header_with(key: &str, value: &str) -> HduHeader {
        let mut h = HduHeader::empty();
        h.set(key, value.to_string());
        h
    }

    #[test]
    fn data_unit_strips_quotes_and_whitespace() {
        assert_eq!(data_unit(&header_with("BUNIT", "'MJy/sr  '")), Some("MJy/sr".into()));
        assert_eq!(data_unit(&header_with("BUNIT", "  DN / s ")), Some("DN / s".into()));
        assert_eq!(data_unit(&header_with("BUNIT", "electron")), Some("electron".into()));
        assert_eq!(data_unit(&header_with("BUNIT", "''")), None);
        assert_eq!(data_unit(&header_with("BUNIT", "   ")), None);
        assert_eq!(data_unit(&header_with("BUNIT", "")), None);
        assert_eq!(data_unit(&HduHeader::empty()), None);
        assert_eq!(data_unit(&header_with("BSCALE", "1.0")), None);
    }

    fn dq_plane(signed: bool) -> IntPlane {
        let mut bits = Array2::<u32>::zeros((3, 3));
        bits[[1, 1]] = 3;
        bits[[2, 2]] = 0xFFFF_FFFF;
        IntPlane { bits, signed }
    }

    #[test]
    fn probe_companions_in_and_out_of_bounds() {
        let plane = dq_plane(false);
        let mut err = Array2::from_elem((3, 3), 0.5f32);
        err[[0, 0]] = f32::NAN;
        let c = probe_companions(Some((&plane, DqTable::Jwst)), Some(&err), 1, 1);
        let dq = c.dq.unwrap();
        assert_eq!(dq.bits, 3);
        assert_eq!(dq.value, 3);
        assert_eq!(dq.names, vec!["DO_NOT_USE", "SATURATED"]);
        assert_eq!(dq.table, DqTable::Jwst);
        assert_eq!(dq.text, "3: DO_NOT_USE | SATURATED");
        assert_eq!(c.err, Some(0.5));

        let c = probe_companions(Some((&plane, DqTable::Jwst)), Some(&err), 0, 0);
        assert_eq!(c.dq.unwrap().text, "0: GOOD");
        assert_eq!(c.err, None);

        for (x, y) in [(-1, 0), (0, -1), (3, 0), (0, 3)] {
            let c = probe_companions(Some((&plane, DqTable::Jwst)), Some(&err), x, y);
            assert!(c.dq.is_none() && c.err.is_none(), "({x},{y})");
        }
        let c = probe_companions(None, None, 1, 1);
        assert_eq!(c, CompanionProbe { dq: None, err: None });
    }

    #[test]
    fn probe_companions_dq_value_signed_vs_unsigned() {
        let unsigned = probe_companions(Some((&dq_plane(false), DqTable::Unknown)), None, 2, 2).dq.unwrap();
        assert_eq!(unsigned.bits, 0xFFFF_FFFF);
        assert_eq!(unsigned.value, 4294967295);
        assert_eq!(unsigned.names.len(), 32);
        let signed = probe_companions(Some((&dq_plane(true), DqTable::Unknown)), None, 2, 2).dq.unwrap();
        assert_eq!(signed.bits, 0xFFFF_FFFF);
        assert_eq!(signed.value, -1);
    }

    #[test]
    fn probe_json_with_companions_adds_dq_and_err_objects() {
        let arr = ramp_7x7();
        let p = probe_pixel(&arr, 1.0, 1.0, 3).unwrap();
        let plane = IntPlane { bits: Array2::from_elem((7, 7), 3u32), signed: false };
        let err = Array2::from_elem((7, 7), 0.25f32);
        let comp = probe_companions(Some((&plane, DqTable::Jwst)), Some(&err), 1, 1);
        let j = probe_json_with_companions(&p, Some("MJy/sr"), Some("MJy/sr"), &comp);
        assert_eq!(j[RES_VALUE], 8.0);
        assert_eq!(j[RES_DQ][RES_BITS], 3);
        assert_eq!(j[RES_DQ][RES_VALUE], 3);
        assert_eq!(j[RES_DQ][RES_NAMES], json!(["DO_NOT_USE", "SATURATED"]));
        assert_eq!(j[RES_DQ][RES_TABLE], "jwst");
        assert_eq!(j[RES_DQ][RES_TEXT], "3: DO_NOT_USE | SATURATED");
        assert_eq!(j[RES_ERR][RES_VALUE], 0.25);
        assert_eq!(j[RES_ERR][RES_UNIT], "MJy/sr");

        let empty = probe_json_with_companions(&p, None, None, &CompanionProbe { dq: None, err: None });
        assert!(empty[RES_DQ].is_null());
        assert!(empty[RES_ERR].is_null());
        assert_eq!(empty[RES_NEIGHBORHOOD][RES_N_PIXELS], 9);

        let err_only = probe_json_with_companions(&p, None, None, &CompanionProbe { dq: None, err: Some(1.5) });
        assert!(err_only[RES_DQ].is_null());
        assert_eq!(err_only[RES_ERR][RES_VALUE], 1.5);
        assert!(err_only[RES_ERR][RES_UNIT].is_null());
    }

    #[test]
    fn probe_json_uses_the_shared_result_keys() {
        let arr = ramp_7x7();
        let p = probe_pixel(&arr, 3.0, 3.0, 5).unwrap();
        let j = probe_json(&p, Some("MJy/sr"));
        assert_eq!(j[RES_X], 3);
        assert_eq!(j[RES_Y], 3);
        assert_eq!(j[RES_VALUE], 24.0);
        assert_eq!(j[RES_UNIT], "MJy/sr");
        assert_eq!(j[RES_BOX], 5);
        assert_eq!(j[RES_NEIGHBORHOOD][RES_MIN], 8.0);
        assert_eq!(j[RES_NEIGHBORHOOD][RES_MAX], 40.0);
        assert_eq!(j[RES_NEIGHBORHOOD][RES_MEAN], 24.0);
        assert_eq!(j[RES_NEIGHBORHOOD][RES_MEDIAN], 24.0);
        assert_eq!(j[RES_NEIGHBORHOOD][RES_N_PIXELS], 25);
        assert_eq!(j[RES_NEIGHBORHOOD][RES_N_NAN], 0);

        let mut arr = ramp_7x7();
        arr[[0, 0]] = f32::NAN;
        let p = probe_pixel(&arr, 0.0, 0.0, 1).unwrap();
        let j = probe_json(&p, None);
        assert!(j[RES_VALUE].is_null());
        assert!(j[RES_UNIT].is_null());
        assert!(j[RES_NEIGHBORHOOD][RES_MEDIAN].is_null());
        assert_eq!(j[RES_NEIGHBORHOOD][RES_N_NAN], 1);
    }
}
