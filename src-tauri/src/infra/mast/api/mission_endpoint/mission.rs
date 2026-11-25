use serde::{Deserialize, Serialize};

pub const API_VERSION: &str = "v0.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mission {
    MultiMission,
    Classy,
    Hst,
    Iue,
    Jwst,
    Roman,
    RomanCgi,
    RomanSpectra,
    Tess,
    Ullyses,
}

impl Mission {
    pub const ALL: [Mission; 10] = [
        Mission::MultiMission,
        Mission::Classy,
        Mission::Hst,
        Mission::Iue,
        Mission::Jwst,
        Mission::Roman,
        Mission::RomanCgi,
        Mission::RomanSpectra,
        Mission::Tess,
        Mission::Ullyses,
    ];

    pub const fn slug(self) -> &'static str {
        match self {
            Mission::MultiMission => "multi_mission",
            Mission::Classy => "classy",
            Mission::Hst => "hst",
            Mission::Iue => "iue",
            Mission::Jwst => "jwst",
            Mission::Roman => "roman",
            Mission::RomanCgi => "roman_cgi",
            Mission::RomanSpectra => "roman_spectra",
            Mission::Tess => "tess",
            Mission::Ullyses => "ullyses",
        }
    }

    pub fn base_path(self) -> String {
        format!("search/{}/api/{API_VERSION}", self.slug())
    }

    pub fn operation_path(self, operation: &str) -> String {
        format!("{}/{}", self.base_path(), operation.trim_start_matches('/'))
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mission| mission.slug() == slug)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_follow_openapi_layout() {
        assert_eq!(
            Mission::Jwst.operation_path("search"),
            "search/jwst/api/v0.1/search"
        );
        assert_eq!(
            Mission::MultiMission.operation_path("/get_image"),
            "search/multi_mission/api/v0.1/get_image"
        );
        assert_eq!(
            Mission::RomanSpectra.base_path(),
            "search/roman_spectra/api/v0.1"
        );
    }

    #[test]
    fn slug_round_trips_for_every_mission() {
        for mission in Mission::ALL {
            assert_eq!(Mission::from_slug(mission.slug()), Some(mission));
            assert_eq!(
                serde_json::to_value(mission).unwrap(),
                serde_json::Value::String(mission.slug().to_owned())
            );
        }
        assert_eq!(Mission::from_slug("galex"), None);
    }
}
