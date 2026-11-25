use ndarray::{Array2, Zip};

#[derive(Clone, Copy)]
struct GridBracket {
    low: usize,
    high: usize,
    high_weight: f64,
}

fn grid_brackets(coordinates: &[f32], grid_len: usize) -> Vec<Option<GridBracket>> {
    let last = grid_len - 1;
    coordinates
        .iter()
        .map(|&coordinate| {
            let coordinate = f64::from(coordinate);
            coordinate.is_finite().then(|| {
                let clamped = coordinate.clamp(0.0, last as f64);
                let floor = clamped.floor();
                let low = floor as usize;
                GridBracket { low, high: (low + 1).min(last), high_weight: clamped - floor }
            })
        })
        .collect()
}

pub fn sdss_sky_image(allsky: &Array2<f32>, xinterp: &[f32], yinterp: &[f32]) -> Result<Array2<f32>, String> {
    let (grid_rows, grid_cols) = allsky.dim();
    if grid_rows == 0 || grid_cols == 0 {
        return Err("the SDSS sky grid (ALLSKY) is empty".to_string());
    }
    if xinterp.is_empty() || yinterp.is_empty() {
        return Err("XINTERP and YINTERP must not be empty".to_string());
    }
    let row_brackets = grid_brackets(yinterp, grid_rows);
    let column_brackets = grid_brackets(xinterp, grid_cols);
    let mut sky = Array2::from_elem((yinterp.len(), xinterp.len()), f32::NAN);
    Zip::indexed(&mut sky).par_for_each(|(row, column), value| {
        if let (Some(y), Some(x)) = (row_brackets[row], column_brackets[column]) {
            let along_row = |grid_row: usize| {
                f64::from(allsky[[grid_row, x.low]]) * (1.0 - x.high_weight)
                    + f64::from(allsky[[grid_row, x.high]]) * x.high_weight
            };
            *value = (along_row(y.low) * (1.0 - y.high_weight) + along_row(y.high) * y.high_weight) as f32;
        }
    });
    Ok(sky)
}

fn pixel_sigma(flux_nmgy: f32, nmgy_per_count: f32, sky_counts: f32, gain: f64, dark_variance: f64) -> f32 {
    if !(flux_nmgy.is_finite() && sky_counts.is_finite() && nmgy_per_count.is_finite() && nmgy_per_count > 0.0) {
        return f32::NAN;
    }
    let nmgy_per_count = f64::from(nmgy_per_count);
    let counts = f64::from(flux_nmgy) / nmgy_per_count + f64::from(sky_counts);
    ((counts.max(0.0) / gain + dark_variance).sqrt() * nmgy_per_count) as f32
}

pub fn sdss_frame_sigma(
    image_nmgy: &Array2<f32>,
    calib: &[f32],
    sky_counts: &Array2<f32>,
    gain: f64,
    dark_variance: f64,
) -> Result<Array2<f32>, String> {
    let (rows, cols) = image_nmgy.dim();
    if calib.len() != cols {
        return Err(format!("the calibration vector has {} values but the frame has {cols} columns", calib.len()));
    }
    let (sky_rows, sky_cols) = sky_counts.dim();
    if (sky_rows, sky_cols) != (rows, cols) {
        return Err(format!("the sky image is {sky_cols} x {sky_rows} but the frame is {cols} x {rows}"));
    }
    if !(gain.is_finite() && gain > 0.0) {
        return Err(format!("gain = {gain} e-/DN must be a positive number"));
    }
    if !(dark_variance.is_finite() && dark_variance >= 0.0) {
        return Err(format!("dark variance = {dark_variance} DN^2 must be zero or positive"));
    }
    let mut sigma = Array2::from_elem((rows, cols), f32::NAN);
    Zip::indexed(&mut sigma).and(image_nmgy).and(sky_counts).par_for_each(|(_, column), value, &flux, &sky| {
        *value = pixel_sigma(flux, calib[column], sky, gain, dark_variance);
    });
    Ok(sigma)
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Read;

    use bzip2::read::BzDecoder;

    use super::*;
    use crate::core::astrometry::gwcs::test_support::env_dir;
    use crate::infra::fits::reader::{decode_pixels, parse_header_at};
    use crate::infra::fits::table::{read_columns, Column, ColumnValues};

    const RECIPE_TOLERANCE: f64 = 1e-6;
    const FRAME: &str = "frame-r-001462-4-0165.fits.bz2";
    const FRAME_BYTES: usize = 12_447_360;
    const FRAME_ROWS: usize = 1489;
    const FRAME_COLS: usize = 2048;
    const SKY_GRID_ROWS: usize = 192;
    const SKY_GRID_COLS: usize = 256;
    const TEST_GAIN_E_PER_DN: f64 = 4.76;
    const TEST_DARK_VARIANCE_DN2: f64 = 1.3225;
    const ORACLE_SKY_MIN: f64 = 173.486712038517;
    const ORACLE_SKY_MAX: f64 = 174.58360332250595;
    const ORACLE_COUNTS_MEDIAN: f64 = 174.00007;
    const ORACLE_SIGMA_MEDIAN: f64 = 0.03272367192425486;
    const ORACLE_PIXELS: [([usize; 2], f64, f64); 5] = [
        ([0, 0], 174.5472116470337, 0.03336109225098343),
        ([744, 1024], 174.0297565460205, 0.03207692514935458),
        ([1488, 2047], 173.53621768951416, 0.03345319614361214),
        ([100, 2000], 174.4731149673462, 0.03255361640767915),
        ([7, 3], 174.5419578552246, 0.03288596792515526),
    ];

    fn plane(rows: usize, cols: usize, values: &[f32]) -> Array2<f32> {
        Array2::from_shape_vec((rows, cols), values.to_vec()).unwrap()
    }

    fn relative_difference(actual: f64, expected: f64) -> f64 {
        ((actual - expected) / expected).abs()
    }

    fn float_column(column: &Column) -> Vec<f32> {
        let ColumnValues::F64(values) = &column.values else {
            panic!("column {} holds integers, floats were expected", column.name);
        };
        values.iter().map(|&value| value as f32).collect()
    }

    fn median(values: &mut [f64]) -> f64 {
        values.sort_by(f64::total_cmp);
        let middle = values.len() / 2;
        if values.len() % 2 == 0 {
            (values[middle - 1] + values[middle]) / 2.0
        } else {
            values[middle]
        }
    }

    #[test]
    fn the_sky_is_the_bilinear_interpolation_with_edge_values() {
        let allsky = plane(2, 3, &[0.0, 1.0, 2.0, 10.0, 11.0, 12.0]);
        let xinterp = [-0.5f32, 0.0, 0.5, 1.25, 2.0, 2.7];
        let yinterp = [0.0f32, 0.25, 1.0, 1.5];
        let sky = sdss_sky_image(&allsky, &xinterp, &yinterp).unwrap();
        let expected = plane(
            4,
            6,
            &[
                0.0, 0.0, 0.5, 1.25, 2.0, 2.0, 2.5, 2.5, 3.0, 3.75, 4.5, 4.5, 10.0, 10.0, 10.5, 11.25, 12.0, 12.0,
                10.0, 10.0, 10.5, 11.25, 12.0, 12.0,
            ],
        );
        assert_eq!(sky, expected);
    }

    #[test]
    fn a_one_cell_grid_and_a_non_finite_coordinate() {
        let allsky = plane(1, 1, &[7.5]);
        let sky = sdss_sky_image(&allsky, &[-3.0, 0.0, 9.5], &[0.0, 4.0]).unwrap();
        assert_eq!(sky, Array2::from_elem((2, 3), 7.5f32));

        let sky = sdss_sky_image(&allsky, &[-3.0, f32::NAN, 9.5], &[0.0, 4.0]).unwrap();
        for row in 0..2 {
            assert_eq!(sky[[row, 0]], 7.5);
            assert!(sky[[row, 1]].is_nan());
            assert_eq!(sky[[row, 2]], 7.5);
        }

        let sky = sdss_sky_image(&allsky, &[-3.0, 0.0], &[f32::INFINITY, 4.0]).unwrap();
        assert!(sky[[0, 0]].is_nan() && sky[[0, 1]].is_nan());
        assert_eq!((sky[[1, 0]], sky[[1, 1]]), (7.5, 7.5));
    }

    #[test]
    fn sigma_follows_the_frame_recipe() {
        let image = plane(1, 3, &[0.0, 0.02, -1.0]);
        let calib = [0.005f32, 0.004, 0.005];
        let sky = plane(1, 3, &[100.0, 80.0, 100.0]);
        let sigma = sdss_frame_sigma(&image, &calib, &sky, 4.0, 1.0).unwrap();
        let expected = [0.025495097567963924f64, 0.018867962264113206, 0.005];
        for (column, nanomaggy) in expected.into_iter().enumerate() {
            let found = f64::from(sigma[[0, column]]);
            assert!(
                relative_difference(found, nanomaggy) < RECIPE_TOLERANCE,
                "column {column}: {found} against {nanomaggy} nanomaggy"
            );
        }
    }

    #[test]
    fn pixels_without_a_usable_input_are_nan() {
        let image = plane(2, 5, &[f32::NAN, 0.02, 0.02, 0.02, 0.02, 0.02, 0.02, 0.02, 0.02, 0.02]);
        let calib = [0.005f32, 0.005, 0.0, -0.005, 0.005];
        let sky = plane(2, 5, &[100.0, f32::NAN, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0]);
        let sigma = sdss_frame_sigma(&image, &calib, &sky, 4.0, 1.0).unwrap();
        let unusable = [[0, 0], [0, 1], [0, 2], [0, 3], [1, 2], [1, 3]];
        for (pixel, value) in sigma.indexed_iter() {
            let pixel = [pixel.0, pixel.1];
            assert_eq!(value.is_nan(), unusable.contains(&pixel), "pixel {pixel:?}: {value}");
            assert!(value.is_nan() || (value.is_finite() && *value > 0.0), "pixel {pixel:?}: {value}");
        }
    }

    #[test]
    fn mismatched_shapes_and_bad_noise_inputs_are_refused() {
        let grid = plane(1, 1, &[7.5]);
        for empty in [Array2::<f32>::zeros((0, 0)), Array2::zeros((0, 3)), Array2::zeros((2, 0))] {
            assert_eq!(
                sdss_sky_image(&empty, &[0.0], &[0.0]).unwrap_err(),
                "the SDSS sky grid (ALLSKY) is empty"
            );
        }
        assert_eq!(sdss_sky_image(&grid, &[], &[0.0]).unwrap_err(), "XINTERP and YINTERP must not be empty");
        assert_eq!(sdss_sky_image(&grid, &[0.0], &[]).unwrap_err(), "XINTERP and YINTERP must not be empty");

        let image = Array2::<f32>::zeros((3, 2));
        let sky = Array2::from_elem((3, 2), 100.0f32);
        let calib = [0.005f32, 0.005];
        assert_eq!(
            sdss_frame_sigma(&image, &[0.005, 0.005, 0.005], &sky, 4.0, 1.0).unwrap_err(),
            "the calibration vector has 3 values but the frame has 2 columns"
        );

        let wide_image = Array2::<f32>::zeros((2, 3));
        let square_sky = Array2::from_elem((2, 2), 100.0f32);
        assert_eq!(
            sdss_frame_sigma(&wide_image, &[0.005, 0.005, 0.005], &square_sky, 4.0, 1.0).unwrap_err(),
            "the sky image is 2 x 2 but the frame is 3 x 2"
        );

        let bad_gains = [(0.0, "0"), (-2.0, "-2"), (f64::NAN, "NaN"), (f64::INFINITY, "inf")];
        for (gain, shown) in bad_gains {
            assert_eq!(
                sdss_frame_sigma(&image, &calib, &sky, gain, 1.0).unwrap_err(),
                format!("gain = {shown} e-/DN must be a positive number")
            );
        }

        let bad_dark_variances = [(-1.0, "-1"), (f64::NAN, "NaN"), (f64::INFINITY, "inf")];
        for (dark_variance, shown) in bad_dark_variances {
            assert_eq!(
                sdss_frame_sigma(&image, &calib, &sky, 4.0, dark_variance).unwrap_err(),
                format!("dark variance = {shown} DN^2 must be zero or positive")
            );
        }

        assert!(sdss_frame_sigma(&image, &calib, &sky, 4.0, 0.0).is_ok());
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/S1/frame-r-001462-4-0165.fits.bz2"]
    fn real_sdss_r_frame_dn_is_integer_and_sigma_matches_the_oracle() {
        let path = env_dir("ASTROBURST_SCIENCE_DATA_DIR").join("survey").join("S1").join(FRAME);
        let mut bytes = Vec::new();
        BzDecoder::new(File::open(&path).unwrap()).read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes.len(), FRAME_BYTES);

        let image_hdu = parse_header_at(&bytes, 0).unwrap();
        let calib_hdu = parse_header_at(&bytes, image_hdu.next_hdu_offset).unwrap();
        let sky_hdu = parse_header_at(&bytes, calib_hdu.next_hdu_offset).unwrap();
        assert_eq!(image_hdu.header.get_i64("BITPIX"), Some(-32));
        assert_eq!(image_hdu.header.get_i64("NAXIS1"), Some(FRAME_COLS as i64));
        assert_eq!(image_hdu.header.get_i64("NAXIS2"), Some(FRAME_ROWS as i64));
        assert_eq!(calib_hdu.header.get_i64("BITPIX"), Some(-32));
        assert_eq!(calib_hdu.header.get_i64("NAXIS1"), Some(FRAME_COLS as i64));

        let image_bytes = &bytes[image_hdu.data_start..image_hdu.data_start + FRAME_ROWS * FRAME_COLS * 4];
        let image = plane(FRAME_ROWS, FRAME_COLS, &decode_pixels(image_bytes, -32, 1.0, 0.0));
        let calib = decode_pixels(&bytes[calib_hdu.data_start..calib_hdu.data_start + FRAME_COLS * 4], -32, 1.0, 0.0);
        let columns =
            read_columns(&bytes[sky_hdu.data_start..], &sky_hdu.header, &["ALLSKY", "XINTERP", "YINTERP"]).unwrap();
        assert_eq!(columns[0].tdim, Some(vec![SKY_GRID_COLS, SKY_GRID_ROWS]));
        let allsky = plane(SKY_GRID_ROWS, SKY_GRID_COLS, &float_column(&columns[0]));
        let xinterp = float_column(&columns[1]);
        let yinterp = float_column(&columns[2]);
        assert_eq!((xinterp.len(), yinterp.len()), (FRAME_COLS, FRAME_ROWS));

        let sky = sdss_sky_image(&allsky, &xinterp, &yinterp).unwrap();
        assert_eq!(sky.dim(), (FRAME_ROWS, FRAME_COLS));
        let sky_min = f64::from(sky.iter().copied().fold(f32::INFINITY, f32::min));
        let sky_max = f64::from(sky.iter().copied().fold(f32::NEG_INFINITY, f32::max));
        println!("sky minimum {sky_min} DN, maximum {sky_max} DN");
        assert!((sky_min - ORACLE_SKY_MIN).abs() < 5e-5, "sky minimum {sky_min} DN");
        assert!((sky_max - ORACLE_SKY_MAX).abs() < 5e-5, "sky maximum {sky_max} DN");
        for (pixel, oracle_sky, _) in ORACLE_PIXELS {
            let found = f64::from(sky[pixel]);
            println!("sky {pixel:?} = {found} DN (oracle {oracle_sky})");
            assert!((found - oracle_sky).abs() < 3e-5, "sky {pixel:?} = {found} DN against {oracle_sky}");
        }

        let mut counts: Vec<f64> = image
            .indexed_iter()
            .map(|((row, column), &flux)| f64::from(flux) / f64::from(calib[column]) + f64::from(sky[[row, column]]))
            .collect();
        let near_integer =
            counts.iter().filter(|dn| (*dn - dn.round()).abs() < 0.02).count() as f64 / counts.len() as f64;
        let below_zero = counts.iter().filter(|dn| **dn < 0.0).count();
        let counts_median = median(&mut counts);
        println!("dn within 0.02 of an integer: fraction {near_integer}");
        println!("dn median {counts_median} DN, pixels with dn below 0: {below_zero}");
        assert!(near_integer >= 0.99, "fraction of dn within 0.02 of an integer: {near_integer}");
        assert!((counts_median - ORACLE_COUNTS_MEDIAN).abs() < 0.01, "dn median {counts_median} DN");
        assert_eq!(below_zero, 0);

        let sigma = sdss_frame_sigma(&image, &calib, &sky, TEST_GAIN_E_PER_DN, TEST_DARK_VARIANCE_DN2).unwrap();
        assert_eq!(sigma.dim(), (FRAME_ROWS, FRAME_COLS));
        for (pixel, _, oracle_sigma) in ORACLE_PIXELS {
            let found = f64::from(sigma[pixel]);
            println!("sigma {pixel:?} = {found} nanomaggy (oracle {oracle_sigma})");
            assert!(
                relative_difference(found, oracle_sigma) < 5e-6,
                "sigma {pixel:?} = {found} nanomaggy against {oracle_sigma}"
            );
        }
        let mut sigmas: Vec<f64> = sigma.iter().map(|&value| f64::from(value)).collect();
        let sigma_median = median(&mut sigmas);
        println!("sigma median {sigma_median} nanomaggy (oracle {ORACLE_SIGMA_MEDIAN})");
        assert!(
            relative_difference(sigma_median, ORACLE_SIGMA_MEDIAN) < 1e-5,
            "sigma median {sigma_median} nanomaggy"
        );
    }
}
