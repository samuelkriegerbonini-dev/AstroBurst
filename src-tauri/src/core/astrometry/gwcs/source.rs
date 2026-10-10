use super::pipeline::GwcsPipeline;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GwcsOrigin {
    AsdfTree { key: String },
    FitsAsdfHdu { hdu_index: usize },
}

impl GwcsOrigin {
    pub fn describe(&self) -> String {
        match self {
            GwcsOrigin::AsdfTree { key } => key.clone(),
            GwcsOrigin::FitsAsdfHdu { hdu_index } => format!("ASDF HDU {hdu_index}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GwcsSource {
    pub pipeline: GwcsPipeline,
    pub origin: GwcsOrigin,
    pub wcsinfo_sip_max_err_px: Option<f64>,
    pub wcsinfo_sip_inv_err_px: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::GwcsOrigin;

    #[test]
    fn origin_describes_itself_as_the_info_panel_source() {
        assert_eq!(GwcsOrigin::FitsAsdfHdu { hdu_index: 8 }.describe(), "ASDF HDU 8");
        assert_eq!(GwcsOrigin::AsdfTree { key: "roman.meta.wcs".into() }.describe(), "roman.meta.wcs");
        assert_eq!(GwcsOrigin::AsdfTree { key: "wcs".into() }.describe(), "wcs");
    }
}
