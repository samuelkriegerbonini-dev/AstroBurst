
use std::collections::HashMap;
use std::io::{BufWriter, Write};

use anyhow::{Context, Result};
use ndarray::Array2;

use crate::model::HduHeader;
use crate::utils::constants::BLOCK_SIZE;






#[derive(Debug, Clone, Default)]
pub struct FitsWriteConfig {
    
    pub extra_headers: HashMap<String, String>,
    
    pub copy_wcs: bool,
    
    pub copy_obs_metadata: bool,
    
    pub software: Option<String>,
}


pub fn write_fits_image(
    image: &Array2<f32>,
    output_path: &str,
    source_header: Option<&HduHeader>,
    config: &FitsWriteConfig,
) -> Result<String> {
    let (rows, cols) = image.dim();

    
    let mut cards = Vec::new();

    
    cards.push(("SIMPLE".into(), "T".into()));
    cards.push(("BITPIX".into(), "-32".into()));
    cards.push(("NAXIS".into(), "2".into()));
    cards.push(("NAXIS1".into(), format!("{}", cols)));
    cards.push(("NAXIS2".into(), format!("{}", rows)));
    cards.push(("BSCALE".into(), "1.0".into()));
    cards.push(("BZERO".into(), "0.0".into()));

    
    if let Some(src) = source_header {
        if config.copy_wcs {
            for key in WCS_KEYS {
                if let Some(val) = src.get(key) {
                    cards.push((key.to_string(), val.to_string()));
                }
            }
        }
        if config.copy_obs_metadata {
            for key in OBS_KEYS {
                if let Some(val) = src.get(key) {
                    cards.push((key.to_string(), val.to_string()));
                }
            }
        }
    }

    
    for (k, v) in &config.extra_headers {
        
        cards.retain(|(ck, _)| ck != k);
        cards.push((k.clone(), v.clone()));
    }

    
    if let Some(sw) = &config.software {
        cards.push(("HISTORY".into(), format!("Processed by {}", sw)));
    }

    
    let file = std::fs::File::create(output_path)
        .with_context(|| format!("Cannot create {}", output_path))?;
    let mut writer = BufWriter::new(file);

    write_header_block(&mut writer, &cards)?;
    write_f32_data(&mut writer, image)?;

    writer.flush()?;
    Ok(output_path.to_string())
}


pub fn write_fits_rgb(
    r: &Array2<f32>,
    g: &Array2<f32>,
    b: &Array2<f32>,
    output_path: &str,
    source_header: Option<&&HduHeader>,
    config: &FitsWriteConfig,
) -> Result<String> {
    let (rows, cols) = r.dim();

    let mut cards = Vec::new();

    cards.push(("SIMPLE".into(), "T".into()));
    cards.push(("BITPIX".into(), "-32".into()));
    cards.push(("NAXIS".into(), "3".into()));
    cards.push(("NAXIS1".into(), format!("{}", cols)));
    cards.push(("NAXIS2".into(), format!("{}", rows)));
    cards.push(("NAXIS3".into(), "3".into()));
    cards.push(("BSCALE".into(), "1.0".into()));
    cards.push(("BZERO".into(), "0.0".into()));

    
    if let Some(src) = source_header {
        if config.copy_wcs {
            for key in WCS_KEYS {
                if let Some(val) = src.get(key) {
                    cards.push((key.to_string(), val.to_string()));
                }
            }
        }
        if config.copy_obs_metadata {
            for key in OBS_KEYS {
                if let Some(val) = src.get(key) {
                    cards.push((key.to_string(), val.to_string()));
                }
            }
        }
    }

    for (k, v) in &config.extra_headers {
        cards.retain(|(ck, _)| ck != k);
        cards.push((k.clone(), v.clone()));
    }

    if let Some(sw) = &config.software {
        cards.push(("HISTORY".into(), format!("Processed by {}", sw)));
    }

    let file = std::fs::File::create(output_path)
        .with_context(|| format!("Cannot create {}", output_path))?;
    let mut writer = BufWriter::new(file);

    write_header_block(&mut writer, &cards)?;

    
    write_f32_data_no_pad(&mut writer, r)?;
    write_f32_data_no_pad(&mut writer, g)?;
    write_f32_data_no_pad(&mut writer, b)?;

    
    let total_bytes = 3 * rows * cols * 4;
    let remainder = total_bytes % BLOCK_SIZE;
    if remainder != 0 {
        let padding = BLOCK_SIZE - remainder;
        writer.write_all(&vec![0u8; padding])?;
    }

    writer.flush()?;
    Ok(output_path.to_string())
}





const WCS_KEYS: &[&str] = &[
    "CTYPE1", "CTYPE2", "CRPIX1", "CRPIX2", "CRVAL1", "CRVAL2",
    "CD1_1", "CD1_2", "CD2_1", "CD2_2",
    "CDELT1", "CDELT2", "CROTA2",
    "RADESYS", "EQUINOX", "LONPOLE", "LATPOLE",
    "A_ORDER", "B_ORDER", "AP_ORDER", "BP_ORDER",
];

const OBS_KEYS: &[&str] = &[
    "TELESCOP", "INSTRUME", "DETECTOR", "FILTER", "CHANNEL",
    "TARGNAME", "OBJECT", "DATE-OBS", "EXPTIME", "EFFEXPTM",
    "RA_TARG", "DEC_TARG", "BUNIT", "ORIGIN", "OBSERVER",
];





fn write_header_block(writer: &mut impl Write, cards: &[(String, String)]) -> Result<()> {
    let mut block_bytes = Vec::new();

    for (key, value) in cards {
        let card = format_card(key, value);
        block_bytes.extend_from_slice(card.as_bytes());
    }

    
    let end_card = format!("{:<80}", "END");
    block_bytes.extend_from_slice(end_card.as_bytes());

    
    let remainder = block_bytes.len() % BLOCK_SIZE;
    if remainder != 0 {
        let padding = BLOCK_SIZE - remainder;
        block_bytes.extend_from_slice(&vec![b' '; padding]);
    }

    writer.write_all(&block_bytes)?;
    Ok(())
}

fn format_card(key: &str, value: &str) -> String {
    
    if key == "HISTORY" || key == "COMMENT" {
        return format!("{:<8}{:<72}", key, value);
    }

    let keyword = format!("{:<8}", &key[..key.len().min(8)]);

    
    let trimmed = value.trim();
    let is_bool = trimmed == "T" || trimmed == "F";
    let is_numeric = trimmed.parse::<f64>().is_ok() || trimmed.parse::<i64>().is_ok();

    let formatted_value = if is_bool {
        format!("{:>20}", trimmed)
    } else if is_numeric {
        format!("{:>20}", trimmed)
    } else {
        
        let s = if trimmed.len() < 8 {
            format!("{:<8}", trimmed)
        } else {
            trimmed.to_string()
        };
        format!("'{}'", s)
    };

    let card = format!("{}= {}", keyword, formatted_value);

    
    format!("{:<80}", &card[..card.len().min(80)])
}





fn write_f32_data(writer: &mut impl Write, image: &Array2<f32>) -> Result<()> {
    write_f32_data_no_pad(writer, image)?;

    let (rows, cols) = image.dim();
    let data_bytes = rows * cols * 4;
    let remainder = data_bytes % BLOCK_SIZE;
    if remainder != 0 {
        let padding = BLOCK_SIZE - remainder;
        writer.write_all(&vec![0u8; padding])?;
    }

    Ok(())
}

fn write_f32_data_no_pad(writer: &mut impl Write, image: &Array2<f32>) -> Result<()> {
    let (rows, cols) = image.dim();

    
    let mut buf = Vec::with_capacity(cols * 4);
    for y in 0..rows {
        buf.clear();
        for x in 0..cols {
            buf.extend_from_slice(&image[[y, x]].to_be_bytes());
        }
        writer.write_all(&buf)?;
    }

    Ok(())
}





#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array2;

    #[test]
    fn test_format_card_numeric() {
        let card = format_card("BITPIX", "-32");
        assert_eq!(card.len(), 80);
        assert!(card.starts_with("BITPIX  = "));
        assert!(card.contains("-32"));
    }

    #[test]
    fn test_format_card_string() {
        let card = format_card("TELESCOP", "JWST");
        assert_eq!(card.len(), 80);
        assert!(card.contains("'JWST"));
    }

    #[test]
    fn test_format_card_bool() {
        let card = format_card("SIMPLE", "T");
        assert_eq!(card.len(), 80);
        assert!(card.contains("T"));
    }

    #[test]
    fn test_write_fits_roundtrip() {
        let image = Array2::from_shape_fn((64, 64), |(r, c)| (r as f32 * 64.0 + c as f32));
        let path = "/tmp/test_fits_writer.fits";

        let config = FitsWriteConfig {
            software: Some("AstroKit Test".into()),
            ..Default::default()
        };

        write_fits_image(&image, path, None, &config).unwrap();

        
        let meta = std::fs::metadata(path).unwrap();
        let file_size = meta.len() as usize;

        
        let data_size = 64 * 64 * 4;
        let padded_data = ((data_size + BLOCK_SIZE - 1) / BLOCK_SIZE) * BLOCK_SIZE;
        assert!(file_size >= BLOCK_SIZE + padded_data);

        
        let file = std::fs::File::open(path).unwrap();
        let result = crate::utils::mmap::extract_image_mmap(&file).unwrap();
        assert_eq!(result.image.dim(), (64, 64));

        
        let diff = (result.image[[0, 1]] - 1.0).abs();
        assert!(diff < 1e-4, "Pixel mismatch: expected 1.0, got {}", result.image[[0, 1]]);

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_write_fits_rgb() {
        let r = Array2::from_elem((32, 32), 100.0f32);
        let g = Array2::from_elem((32, 32), 200.0f32);
        let b = Array2::from_elem((32, 32), 300.0f32);
        let path = "/tmp/test_fits_rgb.fits";

        let config = FitsWriteConfig::default();
        write_fits_rgb(&r, &g, &b, path, None, &config).unwrap();

        let meta = std::fs::metadata(path).unwrap();
        assert!(meta.len() > 0);

        std::fs::remove_file(path).ok();
    }
}
