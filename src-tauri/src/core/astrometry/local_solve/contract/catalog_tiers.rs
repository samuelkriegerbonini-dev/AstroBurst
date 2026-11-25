#[cfg(test)]
use std::path::Path;

use crate::core::astrometry::catalog::CatalogRow;
#[cfg(test)]
use crate::core::astrometry::healpix_cat::{HealpixCatalog, MmapSource};

use super::constants::{TIER0_MEAN_STARS_PER_DEG2, TIER0_MIN_EXPECTED_STARS};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RankingKey {
    pub colour_slope: f64,
    pub colour_range: (f64, f64),
}

impl RankingKey {
    pub const MAGNITUDE: RankingKey = RankingKey {
        colour_slope: 0.0,
        colour_range: (0.0, 0.0),
    };
    pub const TWO_MASS_J_KS: RankingKey = RankingKey {
        colour_slope: 2.0,
        colour_range: (-0.2, 1.5),
    };

    pub fn of(&self, row: &CatalogRow) -> Option<f64> {
        let mag = row.g.filter(|g| g.is_finite())?;
        let colour = row
            .bp_rp
            .filter(|c| c.is_finite())
            .map_or(0.0, |c| c.clamp(self.colour_range.0, self.colour_range.1));
        Some(mag + self.colour_slope * colour)
    }
}

pub trait CatalogProvider: Send + Sync {
    fn label(&self) -> &'static str;

    fn cone(&self, ra_deg: f64, dec_deg: f64, radius_deg: f64) -> Result<Vec<CatalogRow>, String>;

    fn ranking_key(&self) -> RankingKey {
        RankingKey::MAGNITUDE
    }

    fn fixed_epoch_year(&self) -> Option<f64> {
        None
    }

    fn answering_host(&self) -> Option<String> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogTier {
    Tier0,
    Tier1,
    Tier2,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TierSpec {
    pub tier: CatalogTier,
    pub label: &'static str,
    pub ranking: RankingKey,
}

pub const TYCHO2_TIER: TierSpec = TierSpec {
    tier: CatalogTier::Tier0,
    label: "tier0-tycho2",
    ranking: RankingKey::MAGNITUDE,
};
pub const SIRIL_GAIA_TIER: TierSpec = TierSpec {
    tier: CatalogTier::Tier1,
    label: "tier1-gaia-siril",
    ranking: RankingKey::MAGNITUDE,
};
pub const TWOMASS_TIER: TierSpec = TierSpec {
    tier: CatalogTier::Tier1,
    label: "tier1-2mass",
    ranking: RankingKey::TWO_MASS_J_KS,
};
pub const GAIA_ONLINE_TIER: TierSpec = TierSpec {
    tier: CatalogTier::Tier2,
    label: "tier2-gaia-vizier",
    ranking: RankingKey::MAGNITUDE,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldSize {
    pub width_deg: f64,
    pub height_deg: f64,
}

#[derive(Default)]
pub struct TierChain {
    pub tiers: Vec<(CatalogTier, Box<dyn CatalogProvider>)>,
}

pub fn expected_tier0_stars(field: FieldSize) -> f64 {
    TIER0_MEAN_STARS_PER_DEG2 * field.width_deg * field.height_deg
}

impl TierChain {
    pub fn choose(&self, field: FieldSize) -> Option<(CatalogTier, &dyn CatalogProvider)> {
        let tier0 = self.tier(CatalogTier::Tier0);
        if expected_tier0_stars(field) >= TIER0_MIN_EXPECTED_STARS {
            if let Some(provider) = tier0 {
                return Some((CatalogTier::Tier0, provider));
            }
        }
        self.tiers
            .iter()
            .find(|(tier, _)| *tier != CatalogTier::Tier0)
            .map(|(tier, provider)| (*tier, provider.as_ref()))
            .or_else(|| tier0.map(|provider| (CatalogTier::Tier0, provider)))
    }

    fn tier(&self, wanted: CatalogTier) -> Option<&dyn CatalogProvider> {
        self.tiers
            .iter()
            .find(|(tier, _)| *tier == wanted)
            .map(|(_, provider)| provider.as_ref())
    }
}

#[cfg(test)]
struct HealpixProvider {
    catalog: HealpixCatalog<MmapSource>,
    spec: TierSpec,
}

#[cfg(test)]
impl CatalogProvider for HealpixProvider {
    fn label(&self) -> &'static str {
        self.spec.label
    }

    fn cone(&self, ra_deg: f64, dec_deg: f64, radius_deg: f64) -> Result<Vec<CatalogRow>, String> {
        self.catalog
            .cone(ra_deg, dec_deg, radius_deg, None)
            .map_err(|error| error.to_string())
    }

    fn ranking_key(&self) -> RankingKey {
        self.spec.ranking
    }
}

#[cfg(test)]
pub fn healpix_provider(path: &Path, spec: TierSpec) -> Result<Box<dyn CatalogProvider>, String> {
    let catalog = HealpixCatalog::open_file(path)
        .map_err(|error| format!("{} at {}: {error}", spec.label, path.display()))?;
    Ok(Box::new(HealpixProvider { catalog, spec }))
}
