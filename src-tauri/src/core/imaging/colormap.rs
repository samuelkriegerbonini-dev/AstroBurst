// Gray/viridis colormaps and RGB8 PNG encoding — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use image::{codecs::png::PngEncoder, ColorType, ImageEncoder};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colormap {
    Gray,
    Viridis,
}

impl Colormap {
    pub fn from_name(name: &str) -> Result<Self, String> {
        match name {
            "gray" | "grey" => Ok(Colormap::Gray),
            "viridis" => Ok(Colormap::Viridis),
            other => Err(format!(
                "unknown colormap '{other}' (supported: gray, viridis)"
            )),
        }
    }

    #[inline]
    fn lookup(self, v: f32) -> [u8; 3] {
        let clamped = if v > 1.0 {
            1.0
        } else if v > 0.0 {
            v
        } else {
            0.0
        };
        let idx = (clamped * 255.0).round() as usize;
        match self {
            Colormap::Gray => {
                let g = idx as u8;
                [g, g, g]
            }
            Colormap::Viridis => VIRIDIS_LUT[idx],
        }
    }
}

pub fn apply_colormap(values: &[f32], colormap: Colormap) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 3);
    for &v in values {
        out.extend_from_slice(&colormap.lookup(v));
    }
    out
}

pub fn encode_png_rgb8(rgb: &[u8], width: usize, height: usize) -> anyhow::Result<Vec<u8>> {
    debug_assert_eq!(rgb.len(), width * height * 3, "rgb buffer size mismatch");
    let mut buf = Vec::with_capacity(width * height * 3);
    let encoder = PngEncoder::new_with_quality(
        std::io::Cursor::new(&mut buf),
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Sub,
    );
    encoder.write_image(rgb, width as u32, height as u32, ColorType::Rgb8.into())?;
    Ok(buf)
}

#[rustfmt::skip]
static VIRIDIS_LUT: [[u8; 3]; 256] = [
    [71, 1, 85], [71, 3, 87], [71, 4, 88], [71, 6, 89], [71, 7, 91], [71, 8, 92],
    [71, 10, 93], [71, 11, 95], [72, 13, 96], [72, 14, 97], [72, 15, 99], [72, 17, 100],
    [72, 18, 101], [72, 20, 103], [72, 21, 104], [72, 22, 105], [72, 24, 106], [72, 25, 108],
    [72, 26, 109], [72, 28, 110], [72, 29, 111], [72, 31, 112], [72, 32, 113], [72, 33, 114],
    [72, 35, 116], [72, 36, 117], [72, 37, 118], [72, 39, 119], [71, 40, 120], [71, 41, 121],
    [71, 42, 121], [71, 44, 122], [71, 45, 123], [71, 46, 124], [71, 48, 125], [70, 49, 126],
    [70, 50, 127], [70, 51, 127], [70, 53, 128], [70, 54, 129], [69, 55, 129], [69, 56, 130],
    [69, 58, 131], [69, 59, 131], [68, 60, 132], [68, 61, 133], [68, 62, 133], [68, 63, 134],
    [67, 65, 134], [67, 66, 135], [67, 67, 135], [66, 68, 136], [66, 69, 136], [65, 70, 136],
    [65, 72, 137], [65, 73, 137], [64, 74, 138], [64, 75, 138], [63, 76, 138], [63, 77, 139],
    [63, 78, 139], [62, 79, 139], [62, 80, 139], [61, 81, 140], [61, 82, 140], [60, 84, 140],
    [60, 85, 140], [59, 86, 140], [59, 87, 141], [58, 88, 141], [58, 89, 141], [57, 90, 141],
    [57, 91, 141], [56, 92, 141], [56, 93, 141], [55, 94, 142], [54, 95, 142], [54, 96, 142],
    [53, 97, 142], [53, 98, 142], [52, 99, 142], [52, 100, 142], [51, 101, 142], [50, 102, 142],
    [50, 103, 142], [49, 104, 142], [49, 105, 142], [48, 106, 142], [48, 107, 142], [47, 108, 142],
    [46, 109, 142], [46, 110, 142], [45, 111, 142], [45, 112, 142], [44, 113, 142], [44, 114, 142],
    [43, 115, 142], [43, 116, 142], [42, 116, 142], [41, 117, 142], [41, 118, 142], [40, 119, 142],
    [40, 120, 142], [39, 121, 142], [39, 122, 142], [38, 123, 142], [38, 124, 141], [37, 125, 141],
    [37, 126, 141], [37, 127, 141], [36, 128, 141], [36, 129, 141], [35, 130, 141], [35, 131, 141],
    [34, 132, 141], [34, 133, 141], [34, 134, 141], [33, 134, 141], [33, 135, 140], [33, 136, 140],
    [33, 137, 140], [32, 138, 140], [32, 139, 140], [32, 140, 140], [32, 141, 140], [31, 142, 140],
    [31, 143, 139], [31, 144, 139], [31, 145, 139], [31, 146, 139], [31, 147, 139], [31, 148, 139],
    [31, 148, 138], [31, 149, 138], [31, 150, 138], [31, 151, 138], [31, 152, 137], [31, 153, 137],
    [31, 154, 137], [31, 155, 137], [32, 156, 136], [32, 157, 136], [32, 158, 136], [32, 159, 136],
    [33, 160, 135], [33, 161, 135], [33, 162, 135], [34, 162, 134], [34, 163, 134], [35, 164, 133],
    [35, 165, 133], [36, 166, 133], [37, 167, 132], [37, 168, 132], [38, 169, 131], [39, 170, 131],
    [39, 171, 130], [40, 172, 130], [41, 172, 129], [42, 173, 128], [43, 174, 128], [43, 175, 127],
    [44, 176, 127], [45, 177, 126], [46, 178, 125], [48, 179, 125], [49, 180, 124], [50, 180, 123],
    [51, 181, 122], [52, 182, 122], [53, 183, 121], [55, 184, 120], [56, 185, 119], [58, 186, 118],
    [59, 186, 117], [60, 187, 116], [62, 188, 115], [63, 189, 114], [65, 190, 113], [67, 191, 112],
    [68, 191, 111], [70, 192, 110], [72, 193, 109], [74, 194, 108], [75, 195, 107], [77, 195, 105],
    [79, 196, 104], [81, 197, 103], [83, 198, 102], [85, 198, 100], [87, 199, 99], [89, 200, 98],
    [91, 201, 96], [94, 201, 95], [96, 202, 94], [98, 203, 92], [100, 204, 91], [103, 204, 89],
    [105, 205, 88], [107, 206, 86], [110, 206, 85], [112, 207, 83], [115, 208, 82], [117, 208, 80],
    [120, 209, 78], [122, 210, 77], [125, 210, 75], [127, 211, 74], [130, 211, 72], [132, 212, 70],
    [135, 213, 69], [138, 213, 67], [141, 214, 65], [143, 214, 64], [146, 215, 62], [149, 215, 61],
    [152, 216, 59], [154, 217, 57], [157, 217, 56], [160, 218, 54], [163, 218, 52], [166, 219, 51],
    [168, 219, 49], [171, 220, 48], [174, 220, 46], [177, 220, 45], [180, 221, 43], [183, 221, 42],
    [186, 222, 41], [188, 222, 39], [191, 223, 38], [194, 223, 37], [197, 223, 36], [200, 224, 35],
    [202, 224, 33], [205, 225, 32], [208, 225, 32], [210, 225, 31], [213, 226, 30], [216, 226, 29],
    [218, 226, 29], [221, 227, 28], [224, 227, 28], [226, 227, 27], [228, 228, 27], [231, 228, 27],
    [233, 228, 27], [236, 229, 27], [238, 229, 27], [240, 229, 28], [242, 230, 28], [244, 230, 29],
    [246, 230, 30], [248, 231, 31], [250, 231, 32], [252, 231, 33],
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gray_is_exact_round_of_v_times_255() {
        let vals: Vec<f32> = (0..=255).map(|i| i as f32 / 255.0).collect();
        let rgb = apply_colormap(&vals, Colormap::Gray);
        for (i, chunk) in rgb.chunks_exact(3).enumerate() {
            let expected = (vals[i] * 255.0).round() as u8;
            assert_eq!(chunk, [expected, expected, expected], "mismatch at index {i}");
        }
        assert_eq!(&apply_colormap(&[0.0], Colormap::Gray)[..], [0, 0, 0]);
        assert_eq!(&apply_colormap(&[1.0], Colormap::Gray)[..], [255, 255, 255]);
        assert_eq!(&apply_colormap(&[0.5], Colormap::Gray)[..], [128, 128, 128]);
    }

    #[test]
    fn values_are_clamped_and_nan_maps_low() {
        let rgb = apply_colormap(&[-1.0, 2.0, f32::NAN], Colormap::Gray);
        assert_eq!(&rgb[0..3], [0, 0, 0]);
        assert_eq!(&rgb[3..6], [255, 255, 255]);
        assert_eq!(&rgb[6..9], [0, 0, 0]);
        let v = apply_colormap(&[-5.0, 5.0], Colormap::Viridis);
        assert_eq!(&v[0..3], &VIRIDIS_LUT[0][..]);
        assert_eq!(&v[3..6], &VIRIDIS_LUT[255][..]);
    }

    #[test]
    fn viridis_matches_matplotlib_at_reference_points() {
        let refs: [(f32, [u8; 3]); 5] = [
            (0.0, [68, 1, 84]),
            (0.25, [59, 82, 139]),
            (0.5, [33, 145, 140]),
            (0.75, [94, 201, 98]),
            (1.0, [253, 231, 37]),
        ];
        for (v, expected) in refs {
            let got = apply_colormap(&[v], Colormap::Viridis);
            for c in 0..3 {
                let diff = (got[c] as i32 - expected[c] as i32).abs();
                assert!(
                    diff <= 5,
                    "viridis({v}) channel {c}: got {}, expected {} (diff {diff})",
                    got[c],
                    expected[c],
                );
            }
        }
    }

    #[test]
    fn from_name_parses_known_and_rejects_unknown() {
        assert_eq!(Colormap::from_name("gray").unwrap(), Colormap::Gray);
        assert_eq!(Colormap::from_name("grey").unwrap(), Colormap::Gray);
        assert_eq!(Colormap::from_name("viridis").unwrap(), Colormap::Viridis);
        let err = Colormap::from_name("jet").unwrap_err();
        assert!(err.contains("jet"), "message should name the bad input: {err}");
        assert!(err.contains("gray") && err.contains("viridis"), "message lists supported: {err}");
    }

    #[test]
    fn encode_png_rgb8_round_trips_through_the_decoder() {
        let width = 2;
        let height = 2;
        #[rustfmt::skip]
        let rgb: Vec<u8> = vec![
            255, 0, 0,    0, 255, 0,
            0, 0, 255,    255, 255, 0,
        ];
        let png = encode_png_rgb8(&rgb, width, height).unwrap();
        assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        let decoded = image::load_from_memory(&png).unwrap().into_rgb8();
        assert_eq!(decoded.dimensions(), (width as u32, height as u32));
        assert_eq!(decoded.into_raw(), rgb);
    }
}
