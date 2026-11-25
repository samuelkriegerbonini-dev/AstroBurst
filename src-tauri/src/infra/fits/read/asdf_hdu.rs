use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use anyhow::{bail, Context, Result};

use super::reader::{extract_header_by_index, list_extensions};
use crate::types::header::HduHeader;

pub const ASDF_HDU_EXTNAME: &str = "ASDF";
const ASDF_CELL_COLUMN: &str = "ASDF_METADATA";
const ASDF_MAGIC: &[u8] = b"#ASDF";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsdfHdu {
    pub index: usize,
    pub data_start: usize,
    pub byte_len: usize,
}

pub fn find_asdf_hdu(file: &File) -> Result<Option<AsdfHdu>> {
    for info in list_extensions(file)? {
        let named_asdf = info.extname.as_deref().is_some_and(|n| n.trim().eq_ignore_ascii_case(ASDF_HDU_EXTNAME));
        if !named_asdf {
            continue;
        }
        let header = extract_header_by_index(file, info.index)?;
        match asdf_cell_byte_len(&header) {
            Some(byte_len) => {
                return Ok(Some(AsdfHdu { index: info.index, data_start: info.data_start, byte_len }));
            }
            None => log::debug!(
                "HDU {} 'ASDF' is not a one-column byte table (TTYPE1 '{}', TFORM1 '{}')",
                info.index,
                header.get("TTYPE1").unwrap_or("").trim(),
                header.get("TFORM1").unwrap_or("").trim()
            ),
        }
    }
    Ok(None)
}

fn asdf_cell_byte_len(header: &HduHeader) -> Option<usize> {
    let bintable = header.get("XTENSION").is_some_and(|x| x.to_ascii_uppercase().contains("BINTABLE"));
    let one_column = header.get_i64("TFIELDS") == Some(1);
    let cell_column = header.get("TTYPE1").is_some_and(|t| t.trim().eq_ignore_ascii_case(ASDF_CELL_COLUMN));
    let one_row = header.get_i64("NAXIS2") == Some(1);
    let no_heap = header.get_i64("PCOUNT").unwrap_or(0) == 0;
    let naxis1 = usize::try_from(header.get_i64("NAXIS1")?).ok().filter(|n| *n > 0)?;
    let tform_bytes = header.get("TFORM1")?.trim().strip_suffix('B')?.trim();
    let byte_count = tform_bytes.bytes().all(|b| b.is_ascii_digit()).then(|| tform_bytes.parse::<usize>().ok()).flatten()?;
    (bintable && one_column && cell_column && one_row && no_heap && byte_count == naxis1).then_some(naxis1)
}

pub fn read_asdf_hdu_bytes(file: &File, hdu: &AsdfHdu) -> Result<Vec<u8>> {
    let file_len = file.metadata().with_context(|| format!("HDU {} 'ASDF': cannot stat the file", hdu.index))?.len();
    let cell_end = u64::try_from(hdu.data_start).ok().zip(u64::try_from(hdu.byte_len).ok()).and_then(|(start, len)| start.checked_add(len));
    match cell_end {
        Some(end) if end <= file_len => {}
        _ => bail!(
            "HDU {} 'ASDF': the {}-byte cell at byte {} runs past the end of the file ({} bytes)",
            hdu.index,
            hdu.byte_len,
            hdu.data_start,
            file_len
        ),
    }
    let mut reader = file;
    reader
        .seek(SeekFrom::Start(hdu.data_start as u64))
        .with_context(|| format!("HDU {} 'ASDF': cannot seek to byte {}", hdu.index, hdu.data_start))?;
    let mut bytes = vec![0u8; hdu.byte_len];
    reader
        .read_exact(&mut bytes)
        .with_context(|| format!("HDU {} 'ASDF': cannot read the {}-byte cell", hdu.index, hdu.byte_len))?;
    if !bytes.starts_with(ASDF_MAGIC) {
        bail!("HDU {} 'ASDF' cell does not start with #ASDF", hdu.index);
    }
    Ok(bytes)
}

#[cfg(test)]
pub(crate) mod test_fixtures {
    use std::path::Path;

    use crate::infra::fits::reader::test_fixtures::{empty_primary_cards, plane_hdu, write_raw_hdus};

    pub(crate) type RawHdu = (Vec<(&'static str, String)>, Vec<u8>);

    pub(crate) fn asdf_bintable_hdu(cell: &[u8]) -> RawHdu {
        let n = cell.len();
        let cards: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'BINTABLE'".into()),
            ("BITPIX", "8".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", n.to_string()),
            ("NAXIS2", "1".into()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("TFIELDS", "1".into()),
            ("TTYPE1", "'ASDF_METADATA'".into()),
            ("TFORM1", format!("'{n}B'")),
            ("EXTNAME", "'ASDF    '".into()),
        ];
        (cards, cell.to_vec())
    }

    pub(crate) fn set_card(cards: &mut [(&'static str, String)], key: &str, value: &str) {
        let slot = cards.iter_mut().find(|(k, _)| *k == key).expect("card present");
        slot.1 = value.to_string();
    }

    pub(crate) fn sci_hdu(extra_cards: &[(&'static str, String)]) -> RawHdu {
        let (mut cards, data) = plane_hdu("SCI", 4, 4);
        cards.extend(extra_cards.iter().cloned());
        (cards, data)
    }

    pub(crate) fn write_fits_with_hdus(path: &Path, sci_cards: &[(&'static str, String)], trailing: Vec<RawHdu>) {
        let mut hdus = vec![(empty_primary_cards(), Vec::new()), sci_hdu(sci_cards)];
        hdus.extend(trailing);
        write_raw_hdus(path, &hdus);
    }

    pub(crate) fn write_fits_with_asdf_cell(path: &Path, sci_cards: &[(&'static str, String)], cell: &[u8]) {
        write_fits_with_hdus(path, sci_cards, vec![asdf_bintable_hdu(cell)]);
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use super::test_fixtures::{asdf_bintable_hdu, set_card, write_fits_with_asdf_cell, write_fits_with_hdus};
    use super::*;
    use crate::core::astrometry::gwcs::find_wcs_node;
    use crate::core::astrometry::gwcs::test_support::fixtures_dir;
    use crate::infra::asdf::AsdfFile;
    use crate::types::constants::BLOCK_SIZE;

    fn nircam_cell() -> Vec<u8> {
        std::fs::read(fixtures_dir().join("wcs_jwst_nircam_cal300.asdf")).unwrap()
    }

    #[test]
    fn asdf_hdu_cell_is_read_back_byte_for_byte() {
        let cell = nircam_cell();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nircam_cal300.fits");
        write_fits_with_asdf_cell(&path, &[], &cell);
        let file = File::open(&path).unwrap();
        let hdu = find_asdf_hdu(&file).unwrap().expect("the ASDF HDU is found");
        assert_eq!(hdu.index, 2);
        assert_eq!(hdu.byte_len, cell.len());
        assert_eq!(hdu.data_start % BLOCK_SIZE, 0);
        assert_eq!(hdu.data_start, 3 * BLOCK_SIZE + BLOCK_SIZE);
        let bytes = read_asdf_hdu_bytes(&file, &hdu).unwrap();
        assert_eq!(bytes, cell);
        let asdf = AsdfFile::from_bytes(bytes).unwrap();
        let (key, _) = find_wcs_node(&asdf.tree).expect("the cell holds a wcs node");
        assert_eq!(key.0, "wcs");
    }

    #[test]
    fn each_table_card_is_load_bearing() {
        let dir = tempfile::tempdir().unwrap();
        let cell = nircam_cell();
        let cases = [
            ("XTENSION", "'IMAGE   '"),
            ("TFIELDS", "2"),
            ("TTYPE1", "'PAYLOAD'"),
            ("TFORM1", "'100B'"),
            ("NAXIS2", "2"),
            ("PCOUNT", "8"),
        ];
        for (card, value) in cases {
            let path = dir.path().join(format!("{}.fits", card.to_ascii_lowercase()));
            let (mut cards, data) = asdf_bintable_hdu(&cell);
            set_card(&mut cards, card, value);
            write_fits_with_hdus(&path, &[], vec![(cards, data)]);
            assert_eq!(find_asdf_hdu(&File::open(&path).unwrap()).unwrap(), None, "{card} = {value}");
        }
    }

    #[test]
    fn a_foreign_asdf_extension_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let cell = nircam_cell();

        let image_named_asdf = dir.path().join("image_named_asdf.fits");
        let (mut cards, data) = crate::infra::fits::reader::test_fixtures::plane_hdu("ASDF", 4, 4);
        cards.push(("TFIELDS", "1".into()));
        write_fits_with_hdus(&image_named_asdf, &[], vec![(cards, data)]);
        assert_eq!(find_asdf_hdu(&File::open(&image_named_asdf).unwrap()).unwrap(), None);

        let foreign_then_real = dir.path().join("foreign_then_real.fits");
        let (mut cards, data) = asdf_bintable_hdu(&cell);
        set_card(&mut cards, "TFIELDS", "2");
        write_fits_with_hdus(&foreign_then_real, &[], vec![(cards, data), asdf_bintable_hdu(&cell)]);
        let found = find_asdf_hdu(&File::open(&foreign_then_real).unwrap()).unwrap().expect("the real cell after a foreign one");
        assert_eq!((found.index, found.byte_len), (3, cell.len()));
    }

    #[test]
    fn a_cell_running_past_the_file_end_is_refused_before_allocation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge_claim.fits");
        let claimed: usize = 1 << 40;
        let (mut cards, data) = asdf_bintable_hdu(&nircam_cell());
        set_card(&mut cards, "NAXIS1", &claimed.to_string());
        set_card(&mut cards, "TFORM1", &format!("'{claimed}B'"));
        write_fits_with_hdus(&path, &[], vec![(cards, data)]);
        let file = File::open(&path).unwrap();
        let hdu = find_asdf_hdu(&file).unwrap().expect("the header alone qualifies");
        assert_eq!(hdu.byte_len, claimed);
        let err = read_asdf_hdu_bytes(&file, &hdu).err().expect("the claim exceeds the file").to_string();
        assert!(err.contains("runs past the end of the file"), "{err}");
        assert!(err.contains(&format!("{claimed}-byte cell")), "{err}");
    }

    #[test]
    fn a_cell_without_the_magic_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("junk_cell.fits");
        write_fits_with_asdf_cell(&path, &[], b"junk");
        let file = File::open(&path).unwrap();
        let hdu = find_asdf_hdu(&file).unwrap().expect("the table layout is still a valid ASDF HDU");
        assert_eq!(hdu.byte_len, 4);
        let err = read_asdf_hdu_bytes(&file, &hdu).err().expect("junk is refused").to_string();
        assert!(err.contains("HDU 2 'ASDF' cell does not start with #ASDF"), "{err}");
    }
}
