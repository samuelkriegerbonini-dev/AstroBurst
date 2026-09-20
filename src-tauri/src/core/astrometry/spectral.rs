use serde::Serialize;

use crate::core::astrometry::time::{
    gmst_deg, jd_from_mjd, julian_centuries_j2000, parse_fits_date_and_time, parse_fits_datetime,
    SECONDS_PER_DAY,
};
use crate::types::header::HduHeader;

pub const SPEED_OF_LIGHT_KMS: f64 = 299792.458;
pub const AIR_VACUUM_FORMULA: &str = "Greisen et al. 2006 (FITS Paper III eq. 65)";
pub const AIR_FORMULA_MIN_UM: f64 = 0.2;
pub const CORRECTION_ACCURACY_KMS: f64 = 0.02;

const AIR_VACUUM_TOLERANCE_UM: f64 = 1e-10;
const AIR_VACUUM_MAX_ITERATIONS: usize = 50;
const AU_KM: f64 = 149597870.7;
const EARTH_ROTATION_RAD_PER_S: f64 = 7.292115e-5;
const WGS84_A_M: f64 = 6378137.0;
const WGS84_F: f64 = 1.0 / 298.257223563;
const OBLIQUITY_J2000_DEG: f64 = 23.4392911;
const MOON_TO_EARTH_MASS_RATIO_INVERSE: f64 = 81.30056;
const VELOCITY_FINITE_DIFFERENCE_DAYS: f64 = 1.0 / 48.0;
const TT_MINUS_UTC_SECONDS: f64 = 69.2;
const GIANT_PLANETS: [(&str, f64, f64, f64, f64); 4] = [
    ("Jupiter", 1047.348644, 5.202603209, 34.351519, 3034.9056606),
    ("Saturn", 3497.9018, 9.554909192, 50.077444, 1222.1138488),
    ("Uranus", 22902.98, 19.218446062, 314.055005, 428.4669983),
    ("Neptune", 19412.26, 30.110386869, 304.348665, 218.4862002),
];
const SPACECRAFT_TELESCOPES: [&str; 3] = ["JWST", "HST", "ROMAN"];
const GEOCENTRIC_FRAME: &str = "GEOCENTR";
const EPHEMERIS_METHOD: &str = "Meeus low-precision ephemeris";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AxisKind {
    Wave,
    Awav,
    Freq,
    Vrad,
    Vopt,
    Velo,
    Zopt,
    Unknown,
}

impl AxisKind {
    fn from_code(code: &str) -> AxisKind {
        match code {
            "WAVE" | "LAMB" | "WAVELENG" | "WAVELENGTH" | "LAMBDA" => AxisKind::Wave,
            "AWAV" => AxisKind::Awav,
            "FREQ" => AxisKind::Freq,
            "VRAD" => AxisKind::Vrad,
            "VOPT" => AxisKind::Vopt,
            "VELO" => AxisKind::Velo,
            "ZOPT" => AxisKind::Zopt,
            _ => AxisKind::Unknown,
        }
    }

    pub fn canonical_unit(self) -> &'static str {
        match self {
            AxisKind::Wave | AxisKind::Awav => "um",
            AxisKind::Freq => "GHz",
            AxisKind::Vrad | AxisKind::Vopt | AxisKind::Velo => "km/s",
            AxisKind::Zopt | AxisKind::Unknown => "",
        }
    }

    pub fn fits_default_unit(self) -> &'static str {
        match self {
            AxisKind::Wave | AxisKind::Awav => "m",
            AxisKind::Freq => "Hz",
            AxisKind::Vrad | AxisKind::Vopt | AxisKind::Velo => "m/s",
            AxisKind::Zopt | AxisKind::Unknown => "",
        }
    }

    fn family(self) -> Option<UnitFamily> {
        match self {
            AxisKind::Wave | AxisKind::Awav => Some(UnitFamily::Wavelength),
            AxisKind::Freq => Some(UnitFamily::Frequency),
            AxisKind::Vrad | AxisKind::Vopt | AxisKind::Velo => Some(UnitFamily::Velocity),
            AxisKind::Zopt => Some(UnitFamily::Dimensionless),
            AxisKind::Unknown => None,
        }
    }

    pub fn is_velocity(self) -> bool {
        matches!(self, AxisKind::Vrad | AxisKind::Vopt | AxisKind::Velo)
    }

    pub fn is_spectral(self) -> bool {
        self != AxisKind::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnitFamily {
    Wavelength,
    Frequency,
    Velocity,
    Dimensionless,
}

impl UnitFamily {
    fn name(self) -> &'static str {
        match self {
            UnitFamily::Wavelength => "wavelength",
            UnitFamily::Frequency => "frequency",
            UnitFamily::Velocity => "velocity",
            UnitFamily::Dimensionless => "dimensionless",
        }
    }
}

struct ParsedUnit {
    family: UnitFamily,
    to_canonical: f64,
}

fn normalise_unit(raw: &str) -> String {
    raw.trim()
        .trim_matches('\'')
        .trim()
        .to_lowercase()
        .replace(|c: char| c == '^' || c == '.' || c.is_whitespace(), "")
        .replace("s-1", "/s")
}

fn parse_unit(raw: &str) -> Option<ParsedUnit> {
    let text = normalise_unit(raw);
    let (family, to_canonical) = match text.as_str() {
        "um" | "micron" | "microns" | "micrometer" | "micrometers" | "micrometre" | "micrometres" | "µm" | "μm" => {
            (UnitFamily::Wavelength, 1.0)
        }
        "nm" | "nanometer" | "nanometers" | "nanometre" | "nanometres" => (UnitFamily::Wavelength, 1e-3),
        "angstrom" | "angstroms" | "a" | "å" | "ang" => (UnitFamily::Wavelength, 1e-4),
        "m" | "meter" | "meters" | "metre" | "metres" => (UnitFamily::Wavelength, 1e6),
        "mm" => (UnitFamily::Wavelength, 1e3),
        "cm" => (UnitFamily::Wavelength, 1e4),
        "hz" => (UnitFamily::Frequency, 1e-9),
        "khz" => (UnitFamily::Frequency, 1e-6),
        "mhz" => (UnitFamily::Frequency, 1e-3),
        "ghz" => (UnitFamily::Frequency, 1.0),
        "thz" => (UnitFamily::Frequency, 1e3),
        "km/s" => (UnitFamily::Velocity, 1.0),
        "m/s" => (UnitFamily::Velocity, 1e-3),
        "" => (UnitFamily::Dimensionless, 1.0),
        _ => return None,
    };
    Some(ParsedUnit { family, to_canonical })
}

fn card_string(header: &HduHeader, key: &str) -> Option<String> {
    header
        .get(key)
        .map(|s| s.trim().trim_matches('\'').trim().to_string())
        .filter(|s| !s.is_empty())
}

fn card_string_upper(header: &HduHeader, key: &str) -> Option<String> {
    card_string(header, key).map(|s| s.to_uppercase())
}

fn first_f64(header: &HduHeader, keys: &[&'static str]) -> Option<(f64, &'static str)> {
    for key in keys {
        if let Some(v) = header.get_f64(key).filter(|v| v.is_finite()) {
            return Some((v, key));
        }
    }
    None
}

#[derive(Debug, Clone, Serialize)]
pub struct SpectralAxis {
    pub kind: AxisKind,
    pub ctype: String,
    pub unit: String,
    pub header_unit: String,
    pub header_scale: f64,
    pub values: Vec<f64>,
    pub crval: f64,
    pub cdelt: f64,
    pub crpix: f64,
    pub rest_wavelength_um: Option<f64>,
    pub rest_frequency_hz: Option<f64>,
    pub specsys: Option<String>,
    pub velosys: Option<f64>,
    pub notes: Vec<String>,
}

impl SpectralAxis {
    pub fn header_values(&self) -> Vec<f64> {
        self.values.iter().map(|v| v / self.header_scale).collect()
    }
}

struct ClassifiedCtype {
    kind: AxisKind,
    ctype: String,
    frame_from_suffix: Option<&'static str>,
    note: Option<String>,
}

const AIPS_FRAME_SUFFIXES: [(&str, &str); 7] = [
    ("LSR", "LSRK"),
    ("LSD", "LSRD"),
    ("HEL", "HELIOCEN"),
    ("BAR", "BARYCENT"),
    ("GEO", "GEOCENTR"),
    ("OBS", "TOPOCENT"),
    ("TOP", "TOPOCENT"),
];

fn classify_ctype(raw: Option<String>) -> Result<ClassifiedCtype, String> {
    let Some(ctype) = raw else {
        return Ok(ClassifiedCtype {
            kind: AxisKind::Unknown,
            ctype: String::new(),
            frame_from_suffix: None,
            note: Some("CTYPE3 missing: axis values are reported in header units without interpretation".to_string()),
        });
    };
    let upper = ctype.to_uppercase();
    let (code, suffix) = match upper.find('-') {
        Some(pos) => (&upper[..pos], upper[pos + 1..].trim_matches('-')),
        None => (upper.as_str(), ""),
    };
    let kind = AxisKind::from_code(code);
    if kind == AxisKind::Unknown {
        return Ok(ClassifiedCtype {
            kind,
            ctype: upper.clone(),
            frame_from_suffix: None,
            note: Some(format!("CTYPE3 '{}' is not a recognised spectral type", upper)),
        });
    }
    if suffix.is_empty() {
        let note = (code != "WAVE" && kind == AxisKind::Wave)
            .then(|| format!("CTYPE3 '{}' treated as vacuum wavelength (WAVE)", upper));
        return Ok(ClassifiedCtype { kind, ctype: upper.clone(), frame_from_suffix: None, note });
    }
    if let Some((_, frame)) = AIPS_FRAME_SUFFIXES.iter().find(|(s, _)| *s == suffix) {
        return Ok(ClassifiedCtype {
            kind,
            ctype: upper.clone(),
            frame_from_suffix: Some(frame),
            note: Some(format!("AIPS-style CTYPE3 '{}': frame suffix read as SPECSYS {}", upper, frame)),
        });
    }
    Err(format!("non-linear spectral axis ({}) is not supported", upper))
}

pub fn is_non_linear_spectral_ctype(ctype: &str) -> bool {
    let text = ctype.trim().trim_matches('\'').trim();
    classify_ctype(Some(text.to_string())).is_err()
}

fn spectral_step(header: &HduHeader, axis: usize, notes: &mut Vec<String>) -> Result<f64, String> {
    let cd_key = format!("CD{axis}_{axis}");
    if let Some(cd) = header.get_f64(&cd_key).filter(|v| v.is_finite()) {
        for other in 1..=4usize {
            if other == axis {
                continue;
            }
            let coupled = header.get_f64(&format!("CD{axis}_{other}")).unwrap_or(0.0);
            if coupled != 0.0 {
                notes.push(format!("CD{axis}_{other} is non-zero: the spectral axis is coupled to axis {other} and only the diagonal term was used"));
            }
        }
        return Ok(cd);
    }
    let cdelt_key = format!("CDELT{axis}");
    if let Some(cdelt) = header.get_f64(&cdelt_key).filter(|v| v.is_finite()) {
        let pc = header.get_f64(&format!("PC{axis}_{axis}")).filter(|v| v.is_finite()).unwrap_or(1.0);
        return Ok(cdelt * pc);
    }
    Err(format!("no spectral step: CDELT{axis} or CD{axis}_{axis} required"))
}

fn resolve_unit(
    header: &HduHeader,
    axis: usize,
    kind: AxisKind,
    ctype: &str,
    notes: &mut Vec<String>,
) -> Result<(String, f64, String), String> {
    let cunit_key = format!("CUNIT{axis}");
    let raw = card_string(header, &cunit_key);
    let Some(family) = kind.family() else {
        let unit = raw.clone().unwrap_or_default();
        return Ok((unit.clone(), 1.0, unit));
    };
    let header_unit = match raw {
        Some(u) => u,
        None => {
            let assumed = kind.fits_default_unit();
            notes.push(format!(
                "{} missing: assumed the FITS default '{}' for {}",
                cunit_key,
                if assumed.is_empty() { "dimensionless" } else { assumed },
                ctype
            ));
            assumed.to_string()
        }
    };
    let parsed = parse_unit(&header_unit)
        .ok_or_else(|| format!("{} '{}' is not a recognised spectral unit", cunit_key, header_unit))?;
    if parsed.family != family {
        return Err(format!(
            "{} '{}' is a {} unit but CTYPE{} {} needs a {} unit",
            cunit_key,
            header_unit,
            parsed.family.name(),
            axis,
            ctype,
            family.name()
        ));
    }
    Ok((kind.canonical_unit().to_string(), parsed.to_canonical, header_unit))
}

fn rest_values(header: &HduHeader, notes: &mut Vec<String>) -> (Option<f64>, Option<f64>) {
    let rest_wave = first_f64(header, &["RESTWAV", "RESTWAVE", "RESTWVL"])
        .filter(|(v, _)| *v > 0.0)
        .map(|(v, key)| {
            notes.push(format!("rest wavelength from {} ({} m)", key, v));
            v * 1e6
        });
    let rest_freq = first_f64(header, &["RESTFRQ", "RESTFREQ"])
        .filter(|(v, _)| *v > 0.0)
        .map(|(v, key)| {
            notes.push(format!("rest frequency from {} ({} Hz)", key, v));
            v
        });
    match (rest_wave, rest_freq) {
        (Some(w), None) => {
            let f = SPEED_OF_LIGHT_KMS * 1e9 / w;
            (Some(w), Some(f))
        }
        (None, Some(f)) => {
            let w = SPEED_OF_LIGHT_KMS * 1e9 / f;
            notes.push("rest wavelength derived from the rest frequency".to_string());
            (Some(w), Some(f))
        }
        other => other,
    }
}

pub fn spectral_axis(header: &HduHeader, naxis3: usize) -> Result<SpectralAxis, String> {
    spectral_axis_on(header, 3, naxis3)
}

pub fn spectral_axis_on(header: &HduHeader, axis: usize, len: usize) -> Result<SpectralAxis, String> {
    let mut notes = Vec::new();
    let classified = classify_ctype(card_string(header, &format!("CTYPE{axis}")))?;
    if let Some(note) = classified.note {
        notes.push(note);
    }
    let crval = header
        .get_f64(&format!("CRVAL{axis}"))
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("CRVAL{axis} missing"))?;
    let crpix = header.get_f64(&format!("CRPIX{axis}")).filter(|v| v.is_finite()).unwrap_or(1.0);
    let step = spectral_step(header, axis, &mut notes)?;
    let (unit, scale, header_unit) = resolve_unit(header, axis, classified.kind, &classified.ctype, &mut notes)?;
    let values: Vec<f64> = (0..len)
        .map(|i| (crval + (i as f64 + 1.0 - crpix) * step) * scale)
        .collect();
    let (rest_wavelength_um, rest_frequency_hz) = rest_values(header, &mut notes);
    let specsys = card_string_upper(header, "SPECSYS").or_else(|| classified.frame_from_suffix.map(str::to_string));
    let velosys = header.get_f64("VELOSYS").filter(|v| v.is_finite());
    Ok(SpectralAxis {
        kind: classified.kind,
        ctype: classified.ctype,
        unit,
        header_unit,
        header_scale: scale,
        values,
        crval: crval * scale,
        cdelt: step * scale,
        crpix,
        rest_wavelength_um,
        rest_frequency_hz,
        specsys,
        velosys,
        notes,
    })
}

pub fn air_refractive_index(lambda_um: f64) -> f64 {
    let l2 = lambda_um * lambda_um;
    1.0 + 1e-6 * (287.6155 + 1.62887 / l2 + 0.01360 / (l2 * l2))
}

pub fn air_formula_applies(lambda_um: f64) -> bool {
    lambda_um >= AIR_FORMULA_MIN_UM
}

pub fn vacuum_to_air_um(lambda_vacuum_um: f64) -> f64 {
    if !air_formula_applies(lambda_vacuum_um) {
        return lambda_vacuum_um;
    }
    lambda_vacuum_um / air_refractive_index(lambda_vacuum_um)
}

pub fn air_to_vacuum_um(lambda_air_um: f64) -> f64 {
    if !air_formula_applies(lambda_air_um) {
        return lambda_air_um;
    }
    let mut lambda_vacuum = lambda_air_um * air_refractive_index(lambda_air_um);
    for _ in 0..AIR_VACUUM_MAX_ITERATIONS {
        let next = lambda_air_um * air_refractive_index(lambda_vacuum);
        let converged = (next - lambda_vacuum).abs() < AIR_VACUUM_TOLERANCE_UM;
        lambda_vacuum = next;
        if converged {
            break;
        }
    }
    lambda_vacuum
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VelocityConvention {
    Optical,
    Radio,
    Relativistic,
}

impl VelocityConvention {
    pub fn parse(name: &str) -> Result<VelocityConvention, String> {
        match name.trim().to_lowercase().as_str() {
            "optical" | "vopt" => Ok(VelocityConvention::Optical),
            "radio" | "vrad" => Ok(VelocityConvention::Radio),
            "relativistic" | "velo" | "apparent" => Ok(VelocityConvention::Relativistic),
            other => Err(format!("unknown velocity convention '{}': use optical, radio or relativistic", other)),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            VelocityConvention::Optical => "optical",
            VelocityConvention::Radio => "radio",
            VelocityConvention::Relativistic => "relativistic",
        }
    }
}

pub fn velocity_kms(observed_um: f64, rest_um: f64, convention: VelocityConvention) -> f64 {
    match convention {
        VelocityConvention::Optical => SPEED_OF_LIGHT_KMS * (observed_um - rest_um) / rest_um,
        VelocityConvention::Radio => SPEED_OF_LIGHT_KMS * (observed_um - rest_um) / observed_um,
        VelocityConvention::Relativistic => {
            let o2 = observed_um * observed_um;
            let r2 = rest_um * rest_um;
            SPEED_OF_LIGHT_KMS * (o2 - r2) / (o2 + r2)
        }
    }
}

pub fn wavelength_from_velocity_um(velocity_kms: f64, rest_um: f64, convention: VelocityConvention) -> f64 {
    let beta = velocity_kms / SPEED_OF_LIGHT_KMS;
    match convention {
        VelocityConvention::Optical => rest_um * (1.0 + beta),
        VelocityConvention::Radio => rest_um / (1.0 - beta),
        VelocityConvention::Relativistic => rest_um * ((1.0 + beta) / (1.0 - beta)).sqrt(),
    }
}

pub fn frequency_ghz_from_wavelength_um(wavelength_um: f64) -> f64 {
    SPEED_OF_LIGHT_KMS / wavelength_um
}

pub fn wavelength_um_from_frequency_ghz(frequency_ghz: f64) -> f64 {
    SPEED_OF_LIGHT_KMS / frequency_ghz
}

#[derive(Debug, Clone, Serialize)]
pub struct VelocityAxis {
    pub values_kms: Vec<f64>,
    pub convention: VelocityConvention,
    pub rest_um: f64,
    pub notes: Vec<String>,
}

pub fn velocity_axis(axis: &SpectralAxis, rest_um: f64, convention: VelocityConvention) -> Result<VelocityAxis, String> {
    if !(rest_um.is_finite() && rest_um > 0.0) {
        return Err(format!("rest wavelength must be a positive number of micrometres, got {}", rest_um));
    }
    let mut notes = Vec::new();
    let values_kms: Vec<f64> = match axis.kind {
        AxisKind::Wave => axis.values.iter().map(|&w| velocity_kms(w, rest_um, convention)).collect(),
        AxisKind::Awav => {
            if axis.values.iter().any(|&w| w.is_finite() && !air_formula_applies(w)) {
                notes.push(format!(
                    "some air wavelengths are below {} um where {} does not apply: left unchanged",
                    AIR_FORMULA_MIN_UM, AIR_VACUUM_FORMULA
                ));
            }
            notes.push(format!("air wavelengths converted to vacuum with {}", AIR_VACUUM_FORMULA));
            axis.values
                .iter()
                .map(|&w| velocity_kms(air_to_vacuum_um(w), rest_um, convention))
                .collect()
        }
        AxisKind::Freq => axis
            .values
            .iter()
            .map(|&f| velocity_kms(wavelength_um_from_frequency_ghz(f), rest_um, convention))
            .collect(),
        AxisKind::Zopt => axis
            .values
            .iter()
            .map(|&z| velocity_kms(1.0 + z, 1.0, convention))
            .collect(),
        AxisKind::Vrad | AxisKind::Vopt | AxisKind::Velo => {
            notes.push(format!(
                "axis {} is already a velocity axis: values returned as stored (convention of the header, rest wavelength ignored)",
                axis.ctype
            ));
            axis.values.clone()
        }
        AxisKind::Unknown => {
            return Err(format!(
                "cannot build a velocity axis: CTYPE3 '{}' is not a spectral axis",
                axis.ctype
            ))
        }
    };
    Ok(VelocityAxis { values_kms, convention, rest_um, notes })
}

pub fn velocity_axis_kms(axis: &SpectralAxis, rest_um: f64, convention: VelocityConvention) -> Result<Vec<f64>, String> {
    velocity_axis(axis, rest_um, convention).map(|v| v.values_kms)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct SiteLocation {
    pub lon_deg: f64,
    pub lat_deg: f64,
    pub height_m: f64,
}

pub fn geocentric_to_geodetic(x_m: f64, y_m: f64, z_m: f64) -> SiteLocation {
    let e2 = 2.0 * WGS84_F - WGS84_F * WGS84_F;
    let lon = y_m.atan2(x_m);
    let p = (x_m * x_m + y_m * y_m).sqrt();
    let mut lat = z_m.atan2(p * (1.0 - e2));
    let mut height = 0.0;
    for _ in 0..8 {
        let sin_lat = lat.sin();
        let n = WGS84_A_M / (1.0 - e2 * sin_lat * sin_lat).sqrt();
        height = p / lat.cos() - n;
        lat = z_m.atan2(p * (1.0 - e2 * n / (n + height)));
    }
    SiteLocation { lon_deg: lon.to_degrees(), lat_deg: lat.to_degrees(), height_m: height }
}

pub fn parse_angle_card(raw: &str) -> Option<f64> {
    let text = raw.trim().trim_matches('\'').trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(v) = text.parse::<f64>() {
        return v.is_finite().then_some(v);
    }
    let (sign, body) = match text.chars().next()? {
        '-' => (-1.0, &text[1..]),
        '+' => (1.0, &text[1..]),
        _ => (1.0, text),
    };
    let parts: Vec<&str> = body
        .split(|c: char| c == ':' || c.is_whitespace() || c == 'h' || c == 'd' || c == 'm' || c == 's')
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let mut value = 0.0;
    let mut scale = 1.0;
    for part in parts {
        let v: f64 = part.parse().ok()?;
        value += v * scale;
        scale /= 60.0;
    }
    Some(sign * value)
}

fn first_angle(header: &HduHeader, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| header.get(k).and_then(parse_angle_card))
}

pub fn site_location(header: &HduHeader) -> Option<(SiteLocation, &'static str)> {
    if let (Some(x), Some(y), Some(z)) = (
        header.get_f64("OBSGEO-X"),
        header.get_f64("OBSGEO-Y"),
        header.get_f64("OBSGEO-Z"),
    ) {
        if x.is_finite() && y.is_finite() && z.is_finite() && (x != 0.0 || y != 0.0 || z != 0.0) {
            return Some((geocentric_to_geodetic(x, y, z), "OBSGEO-X/Y/Z"));
        }
    }
    if let (Some(lon), Some(lat)) = (header.get_f64("OBSGEO-L"), header.get_f64("OBSGEO-B")) {
        let height_m = header.get_f64("OBSGEO-H").unwrap_or(0.0);
        return Some((SiteLocation { lon_deg: lon, lat_deg: lat, height_m }, "OBSGEO-L/B/H"));
    }
    let triples: [([&str; 2], &str, &'static str); 4] = [
        (["SITELAT", "SITELONG"], "SITEELEV", "SITELAT/SITELONG"),
        (["LAT-OBS", "LONG-OBS"], "ALT-OBS", "LAT-OBS/LONG-OBS"),
        (["OBSLAT", "OBSLONG"], "OBSALT", "OBSLAT/OBSLONG"),
        (["LATITUDE", "LONGITUD"], "ELEVATIO", "LATITUDE/LONGITUD"),
    ];
    for (lat_lon, height_key, label) in triples {
        if let (Some(lat), Some(lon)) = (first_angle(header, &lat_lon[..1]), first_angle(header, &lat_lon[1..])) {
            let height_m = header.get_f64(height_key).filter(|v| v.is_finite()).unwrap_or(0.0);
            return Some((SiteLocation { lon_deg: lon, lat_deg: lat, height_m }, label));
        }
    }
    None
}

pub fn header_target_coordinates(header: &HduHeader) -> Option<(f64, f64, &'static str)> {
    let ctype1 = card_string_upper(header, "CTYPE1").unwrap_or_default();
    let ctype2 = card_string_upper(header, "CTYPE2").unwrap_or_default();
    if ctype1.starts_with("RA") && ctype2.starts_with("DEC") {
        if let (Some(ra), Some(dec)) = (header.get_f64("CRVAL1"), header.get_f64("CRVAL2")) {
            if ra.is_finite() && dec.is_finite() {
                return Some((ra, dec, "CRVAL1/CRVAL2"));
            }
        }
    }
    let degree_pairs: [(&str, &str, &'static str); 4] = [
        ("RA_TARG", "DEC_TARG", "RA_TARG/DEC_TARG"),
        ("TARG_RA", "TARG_DEC", "TARG_RA/TARG_DEC"),
        ("RA", "DEC", "RA/DEC"),
        ("OBJRA", "OBJDEC", "OBJRA/OBJDEC"),
    ];
    for (ra_key, dec_key, label) in degree_pairs {
        let (Some(ra_raw), Some(dec_raw)) = (header.get(ra_key), header.get(dec_key)) else {
            continue;
        };
        let ra_is_sexagesimal = ra_raw.trim().trim_matches('\'').trim().parse::<f64>().is_err();
        if let (Some(ra), Some(dec)) = (parse_angle_card(ra_raw), parse_angle_card(dec_raw)) {
            let ra_deg = if ra_is_sexagesimal { ra * 15.0 } else { ra };
            return Some((ra_deg, dec, label));
        }
    }
    if let (Some(ra_h), Some(dec)) = (
        header.get("OBJCTRA").and_then(parse_angle_card),
        header.get("OBJCTDEC").and_then(parse_angle_card),
    ) {
        return Some((ra_h * 15.0, dec, "OBJCTRA/OBJCTDEC"));
    }
    None
}

pub fn mid_exposure_jd(header: &HduHeader) -> Option<(f64, String)> {
    if let Some((mjd, key)) = first_f64(header, &["MJD-AVG", "EXPMID", "MJD-MID"]) {
        return Some((jd_from_mjd(mjd), format!("mid-exposure from {}", key)));
    }
    if let Some(date_avg) = card_string(header, "DATE-AVG") {
        if let Ok(jd) = parse_fits_datetime(&date_avg) {
            return Some((jd, "mid-exposure from DATE-AVG".to_string()));
        }
    }
    let exposure = first_f64(header, &["EXPTIME", "EXPOSURE"]);
    let half_exposure_days = exposure.map(|(s, _)| s / 2.0 / SECONDS_PER_DAY).unwrap_or(0.0);
    let exposure_note = match exposure {
        Some((s, key)) => format!("+ {}/2 ({} s)", key, s),
        None => "(EXPTIME missing: exposure start used)".to_string(),
    };
    if let Some(date_obs) = card_string(header, "DATE-OBS") {
        let time_obs = card_string(header, "TIME-OBS");
        if let Ok(jd) = parse_fits_date_and_time(&date_obs, time_obs.as_deref()) {
            let source = if time_obs.is_some() && !date_obs.contains('T') { "DATE-OBS+TIME-OBS" } else { "DATE-OBS" };
            return Some((jd + half_exposure_days, format!("{} {}", source, exposure_note)));
        }
    }
    if let Some((mjd, _)) = first_f64(header, &["MJD-OBS"]) {
        return Some((jd_from_mjd(mjd) + half_exposure_days, format!("MJD-OBS {}", exposure_note)));
    }
    None
}

fn rotate_z(v: [f64; 3], angle_deg: f64) -> [f64; 3] {
    let (s, c) = angle_deg.to_radians().sin_cos();
    [c * v[0] - s * v[1], s * v[0] + c * v[1], v[2]]
}

fn rotate_x(v: [f64; 3], angle_deg: f64) -> [f64; 3] {
    let (s, c) = angle_deg.to_radians().sin_cos();
    [v[0], c * v[1] - s * v[2], s * v[1] + c * v[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: [f64; 3], k: f64) -> [f64; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

pub fn sun_geometric_longitude_deg(jd_tt: f64) -> (f64, f64) {
    let t = julian_centuries_j2000(jd_tt);
    let l0 = 280.46646 + 36000.76983 * t + 0.0003032 * t * t;
    let m = (357.52911 + 35999.05029 * t - 0.0001537 * t * t).to_radians();
    let e = 0.016708634 - 0.000042037 * t - 0.0000001267 * t * t;
    let c = (1.914602 - 0.004817 * t - 0.000014 * t * t) * m.sin()
        + (0.019993 - 0.000101 * t) * (2.0 * m).sin()
        + 0.000289 * (3.0 * m).sin();
    let true_longitude = (l0 + c).rem_euclid(360.0);
    let true_anomaly = m + c.to_radians();
    let radius_au = 1.000001018 * (1.0 - e * e) / (1.0 + e * true_anomaly.cos());
    (true_longitude, radius_au)
}

fn earth_moon_barycentre_heliocentric_ecliptic_au(jd_tt: f64) -> [f64; 3] {
    let (sun_longitude, radius_au) = sun_geometric_longitude_deg(jd_tt);
    let earth_longitude = (sun_longitude + 180.0).to_radians();
    [radius_au * earth_longitude.cos(), radius_au * earth_longitude.sin(), 0.0]
}

fn moon_geocentric_ecliptic_au(jd_tt: f64) -> [f64; 3] {
    let t = julian_centuries_j2000(jd_tt);
    let lp = 218.3164477 + 481267.88123421 * t;
    let d = (297.8501921 + 445267.1114034 * t).to_radians();
    let m = (357.5291092 + 35999.0502909 * t).to_radians();
    let mp = (134.9633964 + 477198.8675055 * t).to_radians();
    let f = (93.2720950 + 483202.0175233 * t).to_radians();
    let sum_l = 6.288774 * mp.sin()
        + 1.274027 * (2.0 * d - mp).sin()
        + 0.658314 * (2.0 * d).sin()
        + 0.213618 * (2.0 * mp).sin()
        - 0.185116 * m.sin()
        - 0.114332 * (2.0 * f).sin()
        + 0.058793 * (2.0 * d - 2.0 * mp).sin()
        + 0.057066 * (2.0 * d - m - mp).sin()
        + 0.053322 * (2.0 * d + mp).sin()
        + 0.045758 * (2.0 * d - m).sin();
    let sum_b = 5.128122 * f.sin()
        + 0.280602 * (mp + f).sin()
        + 0.277693 * (mp - f).sin()
        + 0.173237 * (2.0 * d - f).sin()
        + 0.055413 * (2.0 * d - mp + f).sin()
        + 0.046271 * (2.0 * d - mp - f).sin();
    let sum_r_km = -20905.355 * mp.cos()
        - 3699.111 * (2.0 * d - mp).cos()
        - 2955.968 * (2.0 * d).cos()
        - 569.925 * (2.0 * mp).cos();
    let longitude = (lp + sum_l).to_radians();
    let latitude = sum_b.to_radians();
    let distance_au = (385000.56 + sum_r_km) / AU_KM;
    [
        distance_au * latitude.cos() * longitude.cos(),
        distance_au * latitude.cos() * longitude.sin(),
        distance_au * latitude.sin(),
    ]
}

fn earth_heliocentric_ecliptic_of_date_au(jd_tt: f64) -> [f64; 3] {
    let emb = earth_moon_barycentre_heliocentric_ecliptic_au(jd_tt);
    let moon = moon_geocentric_ecliptic_au(jd_tt);
    add(emb, scale(moon, -1.0 / (1.0 + MOON_TO_EARTH_MASS_RATIO_INVERSE)))
}

fn au_per_day_to_kms(v: [f64; 3]) -> [f64; 3] {
    scale(v, AU_KM / SECONDS_PER_DAY)
}

fn ecliptic_of_date_to_equatorial_j2000(v: [f64; 3], jd_tt: f64) -> [f64; 3] {
    let t = julian_centuries_j2000(jd_tt);
    let precession_in_longitude_deg = 1.3969713 * t + 0.0003086 * t * t;
    rotate_x(rotate_z(v, -precession_in_longitude_deg), OBLIQUITY_J2000_DEG)
}

pub fn earth_heliocentric_velocity_kms(jd_tt: f64) -> [f64; 3] {
    let dt = VELOCITY_FINITE_DIFFERENCE_DAYS;
    let after = earth_heliocentric_ecliptic_of_date_au(jd_tt + dt);
    let before = earth_heliocentric_ecliptic_of_date_au(jd_tt - dt);
    let velocity_au_per_day = scale(add(after, scale(before, -1.0)), 1.0 / (2.0 * dt));
    ecliptic_of_date_to_equatorial_j2000(au_per_day_to_kms(velocity_au_per_day), jd_tt)
}

pub fn sun_barycentric_velocity_kms(jd_tt: f64) -> [f64; 3] {
    let t = julian_centuries_j2000(jd_tt);
    let mut v = [0.0; 3];
    for (_, sun_to_planet_mass, semi_major_au, l0_deg, rate_deg_per_century) in GIANT_PLANETS {
        let longitude = (l0_deg + rate_deg_per_century * t).to_radians();
        let angular_rate_rad_per_day = rate_deg_per_century.to_radians() / 36525.0;
        let speed_au_per_day = semi_major_au * angular_rate_rad_per_day;
        let planet_velocity = [-longitude.sin() * speed_au_per_day, longitude.cos() * speed_au_per_day, 0.0];
        v = add(v, scale(planet_velocity, -1.0 / sun_to_planet_mass));
    }
    rotate_x(au_per_day_to_kms(v), OBLIQUITY_J2000_DEG)
}

pub fn diurnal_velocity_kms(site: &SiteLocation, jd_ut: f64) -> [f64; 3] {
    let lat = site.lat_deg.to_radians();
    let e2 = 2.0 * WGS84_F - WGS84_F * WGS84_F;
    let n = WGS84_A_M / (1.0 - e2 * lat.sin() * lat.sin()).sqrt();
    let axis_distance_km = (n + site.height_m) * lat.cos() / 1000.0;
    let speed = EARTH_ROTATION_RAD_PER_S * axis_distance_km;
    let lst = (gmst_deg(jd_ut) + site.lon_deg).to_radians();
    [-lst.sin() * speed, lst.cos() * speed, 0.0]
}

pub fn target_unit_vector(ra_deg: f64, dec_deg: f64) -> [f64; 3] {
    let (ra, dec) = (ra_deg.to_radians(), dec_deg.to_radians());
    [dec.cos() * ra.cos(), dec.cos() * ra.sin(), dec.sin()]
}

#[derive(Debug, Clone, Serialize)]
pub struct RadialVelocityCorrection {
    pub barycentric_kms: f64,
    pub heliocentric_kms: f64,
    pub jd_mid: f64,
    pub method: &'static str,
    pub accuracy_kms: f64,
    pub notes: Vec<String>,
}

fn is_spacecraft(telescop: &str) -> bool {
    SPACECRAFT_TELESCOPES.iter().any(|s| telescop.contains(s))
}

fn already_corrected_method(frame: &str) -> Option<&'static str> {
    match frame {
        "BARYCENT" => Some("already in BARYCENT"),
        "HELIOCEN" => Some("already in HELIOCEN"),
        "LSRK" | "LSR" => Some("already in LSRK"),
        "LSRD" => Some("already in LSRD"),
        "GALACTOC" => Some("already in GALACTOC"),
        "LOCALGRP" => Some("already in LOCALGRP"),
        "CMBDIPOL" => Some("already in CMBDIPOL"),
        "SOURCE" => Some("already in SOURCE"),
        _ => None,
    }
}

pub fn header_spectral_frame(header: &HduHeader) -> Option<(String, String)> {
    if let Some(specsys) = card_string_upper(header, "SPECSYS") {
        return Some((specsys, "SPECSYS".to_string()));
    }
    (1..=3usize).find_map(|axis| {
        let key = format!("CTYPE{axis}");
        classify_ctype(card_string(header, &key))
            .ok()
            .and_then(|classified| classified.frame_from_suffix)
            .map(|frame| (frame.to_string(), format!("{key} suffix")))
    })
}

fn longitude_sign_is_ambiguous(site_source: &str) -> bool {
    !site_source.starts_with("OBSGEO")
}

pub fn radial_velocity_correction(header: &HduHeader, ra_deg: f64, dec_deg: f64) -> Result<RadialVelocityCorrection, String> {
    if !(ra_deg.is_finite() && dec_deg.is_finite() && (-90.0..=90.0).contains(&dec_deg)) {
        return Err(format!("target coordinates out of range: RA {} Dec {}", ra_deg, dec_deg));
    }
    let timing = mid_exposure_jd(header);
    let jd_mid_or_nan = timing.as_ref().map(|(jd, _)| *jd).unwrap_or(f64::NAN);
    let (frame, frame_source) = header_spectral_frame(header).unwrap_or_default();
    if let Some(method) = already_corrected_method(&frame) {
        return Ok(RadialVelocityCorrection {
            barycentric_kms: 0.0,
            heliocentric_kms: 0.0,
            jd_mid: jd_mid_or_nan,
            method,
            accuracy_kms: 0.0,
            notes: vec![format!(
                "{} is {}: the spectral axis is already in that frame, no further shift applied",
                frame_source, frame
            )],
        });
    }
    let telescop = card_string_upper(header, "TELESCOP").unwrap_or_default();
    if is_spacecraft(&telescop) {
        return match header.get_f64("VELOSYS").filter(|v| v.is_finite()) {
            Some(velosys_m_s) => {
                let correction_kms = -velosys_m_s / 1000.0;
                let frame_note = if frame.is_empty() {
                    format!("TELESCOP {}: no spectral frame keyword in the header", telescop)
                } else {
                    format!("TELESCOP {}: spectral axis in {} ({})", telescop, frame, frame_source)
                };
                Ok(RadialVelocityCorrection {
                    barycentric_kms: correction_kms,
                    heliocentric_kms: correction_kms,
                    jd_mid: jd_mid_or_nan,
                    method: "header VELOSYS",
                    accuracy_kms: 0.0,
                    notes: vec![
                        format!(
                            "VELOSYS {} m/s from the header, JWST convention (positive = observer receding), applied as {} km/s",
                            velosys_m_s, correction_kms
                        ),
                        frame_note,
                    ],
                })
            }
            None => Err(format!(
                "TELESCOP {} is a spacecraft: the barycentric correction needs the spacecraft ephemeris and the header carries no VELOSYS",
                telescop
            )),
        };
    }
    let geocentric = frame == GEOCENTRIC_FRAME;
    let site = if geocentric {
        None
    } else {
        Some(site_location(header).ok_or_else(|| {
            "no observatory location in the header (OBSGEO-X/Y/Z, OBSGEO-L/B/H, SITELAT/SITELONG or LAT-OBS/LONG-OBS)".to_string()
        })?)
    };
    let (jd_mid, time_source) = timing.ok_or_else(|| {
        "no observation time in the header (MJD-AVG, EXPMID, DATE-AVG, DATE-OBS or MJD-OBS)".to_string()
    })?;
    let jd_tt = jd_mid + TT_MINUS_UTC_SECONDS / SECONDS_PER_DAY;
    let target = target_unit_vector(ra_deg, dec_deg);
    let orbital = dot(earth_heliocentric_velocity_kms(jd_tt), target);
    let site_rotation = site.as_ref().map(|(site, _)| diurnal_velocity_kms(site, jd_mid));
    let diurnal = site_rotation.map(|v| dot(v, target)).unwrap_or(0.0);
    let solar = dot(sun_barycentric_velocity_kms(jd_tt), target);
    let heliocentric_kms = orbital + diurnal;
    let barycentric_kms = heliocentric_kms + solar;
    let site_note = match &site {
        Some((site, source)) => {
            format!("site from {}: lon {:.5} lat {:.5} height {:.0} m", source, site.lon_deg, site.lat_deg, site.height_m)
        }
        None => format!("{} is {}: the diurnal term was already removed, no observatory location needed", frame_source, frame),
    };
    let diurnal_note = match &site {
        Some(_) => format!("diurnal term {:+.4} km/s: WGS84 rotation, positive when the site moves toward the target", diurnal),
        None => format!("diurnal term omitted (frame {})", frame),
    };
    let mut notes = vec![
        site_note,
        format!("time: {} -> JD {:.6} (UTC treated as UT1, TT = UTC + {} s)", time_source, jd_mid, TT_MINUS_UTC_SECONDS),
        format!("target RA {:.6} Dec {:.6} (ICRS/J2000 assumed)", ra_deg, dec_deg),
        format!("orbital term {:+.4} km/s: Meeus solar coordinates (Astronomical Algorithms ch. 25) with the Earth-Moon barycentre offset (ch. 47, truncated)", orbital),
        diurnal_note,
        format!("solar barycentric term {:+.4} km/s: Jupiter, Saturn, Uranus and Neptune circular-orbit reflex", solar),
        "sign convention: add the correction to a topocentric velocity; positive when the observer moves toward the target".to_string(),
    ];
    if let (Some((_, source)), Some(rotation)) = (&site, site_rotation) {
        if longitude_sign_is_ambiguous(source) {
            let flip_bound_kms = 2.0 * dot(rotation, rotation).sqrt() * dec_deg.to_radians().cos();
            notes.push(format!(
                "longitude from {} read as east-positive; a west-positive header flips the diurnal term, an error of up to {:.2} km/s that the header alone cannot rule out",
                source, flip_bound_kms
            ));
        }
    }
    if let Some(velosys) = header.get_f64("VELOSYS").filter(|v| v.is_finite()) {
        notes.push(format!("header VELOSYS {} m/s left unused (ground-based recomputation)", velosys));
    }
    if !frame.is_empty() {
        notes.push(format!("header {} {}", frame_source, frame));
    }
    Ok(RadialVelocityCorrection {
        barycentric_kms,
        heliocentric_kms,
        jd_mid,
        method: EPHEMERIS_METHOD,
        accuracy_kms: CORRECTION_ACCURACY_KMS,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::astrometry::time::jd_from_gregorian;
    use crate::core::imaging::region::test_support::make_header;

    fn axis_header(pairs: &[(&str, &str)]) -> HduHeader {
        make_header(pairs)
    }

    #[test]
    fn wave_axis_in_micrometres_with_cdelt3() {
        let h = axis_header(&[
            ("CTYPE3", "WAVE"),
            ("CUNIT3", "um"),
            ("CRVAL3", "1.0"),
            ("CDELT3", "0.01"),
            ("CRPIX3", "1.0"),
        ]);
        let axis = spectral_axis(&h, 5).unwrap();
        assert_eq!(axis.kind, AxisKind::Wave);
        assert_eq!(axis.unit, "um");
        assert_eq!(axis.header_unit, "um");
        assert_eq!(axis.values.len(), 5);
        assert!((axis.values[0] - 1.0).abs() < 1e-12);
        assert!((axis.values[4] - 1.04).abs() < 1e-12);
        assert!(axis.notes.is_empty(), "{:?}", axis.notes);
        assert_eq!(axis.header_values(), axis.values);
    }

    #[test]
    fn wave_axis_without_cunit3_assumes_metres_and_reports_micrometres() {
        let h = axis_header(&[
            ("CTYPE3", "'WAVE    '"),
            ("CRVAL3", "4.7e-7"),
            ("CDELT3", "1.25e-10"),
            ("CRPIX3", "1"),
            ("NAXIS3", "3"),
        ]);
        let axis = spectral_axis(&h, 3).unwrap();
        assert_eq!(axis.unit, "um");
        assert_eq!(axis.header_unit, "m");
        assert!((axis.values[0] - 0.47).abs() < 1e-9);
        assert!((axis.values[2] - 0.47025).abs() < 1e-9);
        assert!(axis.notes.iter().any(|n| n.contains("CUNIT3 missing")), "{:?}", axis.notes);
        let raw = axis.header_values();
        assert!((raw[0] - 4.7e-7).abs() < 1e-18);
        assert!((axis.crval - 0.47).abs() < 1e-9);
        assert!((axis.cdelt - 1.25e-4).abs() < 1e-12);
    }

    #[test]
    fn freq_axis_in_hz_with_cd3_3_becomes_ghz() {
        let h = axis_header(&[
            ("CTYPE3", "FREQ"),
            ("CUNIT3", "Hz"),
            ("CRVAL3", "2.3e11"),
            ("CD3_3", "1.0e6"),
            ("CRPIX3", "2.0"),
            ("RESTFRQ", "230538000000.0"),
            ("SPECSYS", "LSRK"),
        ]);
        let axis = spectral_axis(&h, 3).unwrap();
        assert_eq!(axis.kind, AxisKind::Freq);
        assert_eq!(axis.unit, "GHz");
        assert!((axis.values[0] - 229.999).abs() < 1e-9);
        assert!((axis.values[1] - 230.0).abs() < 1e-9);
        assert!((axis.values[2] - 230.001).abs() < 1e-9);
        assert_eq!(axis.rest_frequency_hz, Some(230538000000.0));
        let rest_um = axis.rest_wavelength_um.unwrap();
        assert!((rest_um - 1300.4036).abs() < 1e-3, "{rest_um}");
        assert_eq!(axis.specsys.as_deref(), Some("LSRK"));
    }

    #[test]
    fn awav_axis_in_angstrom_with_pc3_3_scales_cdelt() {
        let h = axis_header(&[
            ("CTYPE3", "AWAV"),
            ("CUNIT3", "Angstrom"),
            ("CRVAL3", "5000.0"),
            ("CDELT3", "1.0"),
            ("PC3_3", "2.0"),
            ("CRPIX3", "1.0"),
        ]);
        let axis = spectral_axis(&h, 4).unwrap();
        assert_eq!(axis.kind, AxisKind::Awav);
        assert_eq!(axis.unit, "um");
        assert!((axis.values[0] - 0.5).abs() < 1e-12);
        assert!((axis.values[3] - 0.5006).abs() < 1e-12);
        assert!((axis.cdelt - 2e-4).abs() < 1e-15);
    }

    #[test]
    fn cd3_3_takes_precedence_over_a_placeholder_cdelt3() {
        let h = axis_header(&[
            ("CTYPE3", "WAVE"),
            ("CUNIT3", "Angstrom"),
            ("CRVAL3", "4750.0"),
            ("CDELT3", "1.0"),
            ("CD3_3", "1.25"),
            ("CRPIX3", "1.0"),
        ]);
        let axis = spectral_axis(&h, 2).unwrap();
        assert!((axis.values[1] - 0.4751250).abs() < 1e-12);
    }

    #[test]
    fn log_wavelength_axis_is_rejected_with_the_ctype_named() {
        let h = axis_header(&[("CTYPE3", "WAVE-LOG"), ("CRVAL3", "1.0"), ("CDELT3", "0.1")]);
        let err = spectral_axis(&h, 3).unwrap_err();
        assert_eq!(err, "non-linear spectral axis (WAVE-LOG) is not supported");
        let h = axis_header(&[("CTYPE3", "FREQ-TAB"), ("CRVAL3", "1.0"), ("CDELT3", "0.1")]);
        assert!(spectral_axis(&h, 3).unwrap_err().contains("FREQ-TAB"));
        let h = axis_header(&[("CTYPE3", "WAVE-F2W"), ("CRVAL3", "1.0"), ("CDELT3", "0.1")]);
        assert!(spectral_axis(&h, 3).unwrap_err().contains("WAVE-F2W"));
    }

    #[test]
    fn non_linear_ctypes_are_recognised_without_building_the_axis() {
        assert!(is_non_linear_spectral_ctype("WAVE-LOG"));
        assert!(is_non_linear_spectral_ctype("freq-tab"));
        assert!(is_non_linear_spectral_ctype("'WAVE-F2W'"));
        assert!(!is_non_linear_spectral_ctype("WAVE"));
        assert!(!is_non_linear_spectral_ctype("VELO-LSR"));
        assert!(!is_non_linear_spectral_ctype("RA---TAN"));
        assert!(!is_non_linear_spectral_ctype(""));
    }

    #[test]
    fn aips_velocity_axis_with_frame_suffix_is_linear_and_in_km_s() {
        let h = axis_header(&[
            ("CTYPE3", "VELO-LSR"),
            ("CUNIT3", "m/s"),
            ("CRVAL3", "-100000.0"),
            ("CDELT3", "1000.0"),
            ("CRPIX3", "1.0"),
        ]);
        let axis = spectral_axis(&h, 3).unwrap();
        assert_eq!(axis.kind, AxisKind::Velo);
        assert_eq!(axis.unit, "km/s");
        assert!((axis.values[0] + 100.0).abs() < 1e-9);
        assert!((axis.values[2] + 98.0).abs() < 1e-9);
        assert_eq!(axis.specsys.as_deref(), Some("LSRK"));
    }

    #[test]
    fn legacy_header_without_ctype3_still_produces_the_raw_axis() {
        let h = axis_header(&[("CRVAL3", "10.0"), ("CDELT3", "2.0"), ("CRPIX3", "1.0"), ("CUNIT3", "furlong")]);
        let axis = spectral_axis(&h, 4).unwrap();
        assert_eq!(axis.kind, AxisKind::Unknown);
        assert_eq!(axis.values, vec![10.0, 12.0, 14.0, 16.0]);
        assert_eq!(axis.unit, "furlong");
        assert_eq!(axis.header_scale, 1.0);
        assert!(axis.notes.iter().any(|n| n.contains("CTYPE3 missing")));
        let no_step = axis_header(&[("CRVAL3", "10.0")]);
        assert!(spectral_axis(&no_step, 2).unwrap_err().contains("CDELT3"));
        let no_crval = axis_header(&[("CDELT3", "10.0")]);
        assert!(spectral_axis(&no_crval, 2).unwrap_err().contains("CRVAL3"));
    }

    #[test]
    fn unit_family_mismatch_is_an_error_and_unit_spellings_are_lenient() {
        let h = axis_header(&[("CTYPE3", "WAVE"), ("CUNIT3", "Hz"), ("CRVAL3", "1.0"), ("CDELT3", "1.0")]);
        let err = spectral_axis(&h, 2).unwrap_err();
        assert!(err.contains("frequency unit") && err.contains("wavelength unit"), "{err}");
        for spelling in ["MICRONS", "micron", "'um      '", "µm"] {
            let h = axis_header(&[("CTYPE3", "WAVE"), ("CUNIT3", spelling), ("CRVAL3", "2.0"), ("CDELT3", "1.0")]);
            assert_eq!(spectral_axis(&h, 1).unwrap().values, vec![2.0], "{spelling}");
        }
        for spelling in ["km/s", "km s-1", "KM/S", "km s^-1"] {
            let h = axis_header(&[("CTYPE3", "VRAD"), ("CUNIT3", spelling), ("CRVAL3", "5.0"), ("CDELT3", "1.0")]);
            assert_eq!(spectral_axis(&h, 1).unwrap().values, vec![5.0], "{spelling}");
        }
        let h = axis_header(&[("CTYPE3", "WAVE"), ("CUNIT3", "parsec"), ("CRVAL3", "1.0"), ("CDELT3", "1.0")]);
        assert!(spectral_axis(&h, 1).unwrap_err().contains("parsec"));
    }

    #[test]
    fn spectral_axis_on_reads_a_one_dimensional_spectrum_from_axis_1() {
        let h = axis_header(&[("CTYPE1", "WAVE"), ("CUNIT1", "nm"), ("CRVAL1", "500.0"), ("CDELT1", "0.5"), ("CRPIX1", "1")]);
        let axis = spectral_axis_on(&h, 1, 3).unwrap();
        assert!((axis.values[2] - 0.501).abs() < 1e-12);
    }

    #[test]
    fn air_5000_angstrom_becomes_vacuum_5001_472_by_the_greisen_formula() {
        let n: f64 = 1.0 + 1e-6 * (287.6155 + 1.62887 / 0.25 + 0.01360 / 0.0625);
        let expected_angstrom: f64 = 5000.0 * n;
        assert!((expected_angstrom - 5001.4717).abs() < 0.002, "{expected_angstrom}");
        let vacuum_um = air_to_vacuum_um(0.5);
        assert!((vacuum_um * 1e4 - expected_angstrom).abs() < 0.002, "{}", vacuum_um * 1e4);
        assert!((vacuum_um * 1e4 - 5001.472).abs() < 0.002);
        assert!(AIR_VACUUM_FORMULA.contains("Greisen"));
    }

    #[test]
    fn air_vacuum_round_trip_is_within_1e_6_and_short_wavelengths_pass_through() {
        for lambda in [0.2001, 0.3, 0.5, 0.6563, 1.0, 2.2, 5.0, 20.0] {
            let back = vacuum_to_air_um(air_to_vacuum_um(lambda));
            assert!((back - lambda).abs() < 1e-6 * lambda, "{lambda}: {back}");
            let back = air_to_vacuum_um(vacuum_to_air_um(lambda));
            assert!((back - lambda).abs() < 1e-6 * lambda, "{lambda}: {back}");
            assert!(air_to_vacuum_um(lambda) > lambda);
        }
        assert!((vacuum_to_air_um(air_to_vacuum_um(0.2)) - 0.2).abs() < 1e-7);
        let air_at_the_cutoff = vacuum_to_air_um(0.2);
        assert!(air_at_the_cutoff < AIR_FORMULA_MIN_UM);
        assert_eq!(air_to_vacuum_um(air_at_the_cutoff), air_at_the_cutoff);
        assert_eq!(air_to_vacuum_um(0.15), 0.15);
        assert_eq!(vacuum_to_air_um(0.1), 0.1);
        assert!(air_to_vacuum_um(f64::NAN).is_nan());
        assert!(!air_formula_applies(0.19));
        assert!(air_formula_applies(0.2));
    }

    #[test]
    fn h_alpha_shifted_to_0_6585_um_is_plus_1005_km_s_optical() {
        let rest = 0.6563;
        let observed = 0.6585;
        let expected_optical = SPEED_OF_LIGHT_KMS * (observed - rest) / rest;
        assert!((expected_optical - 1004.9).abs() < 0.2, "{expected_optical}");
        let optical = velocity_kms(observed, rest, VelocityConvention::Optical);
        let radio = velocity_kms(observed, rest, VelocityConvention::Radio);
        let relativistic = velocity_kms(observed, rest, VelocityConvention::Relativistic);
        assert!((optical - expected_optical).abs() < 1e-9);
        assert!((radio - SPEED_OF_LIGHT_KMS * (observed - rest) / observed).abs() < 1e-9);
        assert!(radio < relativistic && relativistic < optical, "{radio} {relativistic} {optical}");
        assert!((optical - radio - 3.36).abs() < 0.05, "{}", optical - radio);
        assert!((optical - relativistic - 1.68).abs() < 0.05, "{}", optical - relativistic);
        for conv in [VelocityConvention::Optical, VelocityConvention::Radio, VelocityConvention::Relativistic] {
            let back = wavelength_from_velocity_um(velocity_kms(observed, rest, conv), rest, conv);
            assert!((back - observed).abs() < 1e-12, "{:?}", conv);
            assert_eq!(velocity_kms(rest, rest, conv), 0.0);
        }
        assert!((frequency_ghz_from_wavelength_um(1300.4036) - 230.538).abs() < 1e-3);
        assert!((wavelength_um_from_frequency_ghz(frequency_ghz_from_wavelength_um(2.5)) - 2.5).abs() < 1e-12);
        assert_eq!(VelocityConvention::parse("Radio").unwrap(), VelocityConvention::Radio);
        assert!(VelocityConvention::parse("sideways").is_err());
    }

    #[test]
    fn golden_values_shared_with_the_typescript_mirror() {
        assert!((air_refractive_index(0.5) - 1.00029434858).abs() < 1e-10);
        assert!((air_to_vacuum_um(0.5) - 0.500147172245).abs() < 1e-10);
        assert!((vacuum_to_air_um(0.6563) - 0.656108763679).abs() < 1e-10);
        assert!((air_to_vacuum_um(0.6563) - 0.656491290559).abs() < 1e-10);
        assert!((velocity_kms(0.6585, 0.6563, VelocityConvention::Optical) - 1004.94195886).abs() < 1e-8);
        assert!((velocity_kms(0.6585, 0.6563, VelocityConvention::Radio) - 1001.584521792).abs() < 1e-8);
        assert!((velocity_kms(0.6585, 0.6563, VelocityConvention::Relativistic) - 1003.257622482).abs() < 1e-8);
        assert!((wavelength_um_from_frequency_ghz(230.538) - 1300.403655796).abs() < 1e-8);
    }

    #[test]
    fn velocity_axis_on_a_freq_axis_uses_the_rest_wavelength() {
        let h = axis_header(&[
            ("CTYPE3", "FREQ"),
            ("CUNIT3", "GHz"),
            ("CRVAL3", "230.538"),
            ("CDELT3", "-0.001"),
            ("CRPIX3", "1.0"),
        ]);
        let axis = spectral_axis(&h, 3).unwrap();
        let rest_um = wavelength_um_from_frequency_ghz(230.538);
        let v = velocity_axis(&axis, rest_um, VelocityConvention::Radio).unwrap();
        assert!(v.values_kms[0].abs() < 1e-6, "{}", v.values_kms[0]);
        let expected_channel_1 = SPEED_OF_LIGHT_KMS * 0.001 / 230.538;
        assert!((v.values_kms[1] - expected_channel_1).abs() < 1e-6, "{}", v.values_kms[1]);
        assert!(v.values_kms[2] > v.values_kms[1]);
        assert!(v.notes.is_empty());
        assert!(velocity_axis(&axis, 0.0, VelocityConvention::Radio).is_err());
        assert_eq!(velocity_axis_kms(&axis, rest_um, VelocityConvention::Radio).unwrap().len(), 3);
    }

    #[test]
    fn velocity_axis_converts_air_wavelengths_and_passes_velocity_axes_through() {
        let h = axis_header(&[("CTYPE3", "AWAV"), ("CUNIT3", "Angstrom"), ("CRVAL3", "6561.0"), ("CDELT3", "1.0")]);
        let axis = spectral_axis(&h, 1).unwrap();
        let v = velocity_axis(&axis, 0.6563, VelocityConvention::Optical).unwrap();
        let vacuum = air_to_vacuum_um(0.6561);
        assert!((v.values_kms[0] - velocity_kms(vacuum, 0.6563, VelocityConvention::Optical)).abs() < 1e-9);
        assert!(v.notes.iter().any(|n| n.contains("Greisen")));
        let h = axis_header(&[("CTYPE3", "VRAD"), ("CUNIT3", "m/s"), ("CRVAL3", "1500.0"), ("CDELT3", "500.0")]);
        let axis = spectral_axis(&h, 2).unwrap();
        let v = velocity_axis(&axis, 0.6563, VelocityConvention::Optical).unwrap();
        assert!((v.values_kms[0] - 1.5).abs() < 1e-12 && (v.values_kms[1] - 2.0).abs() < 1e-12, "{:?}", v.values_kms);
        assert!(v.notes.iter().any(|n| n.contains("already a velocity axis")));
        let h = axis_header(&[("CRVAL3", "1.0"), ("CDELT3", "1.0")]);
        let axis = spectral_axis(&h, 2).unwrap();
        assert!(velocity_axis(&axis, 0.6563, VelocityConvention::Optical).is_err());
    }

    #[test]
    fn site_location_falls_back_through_the_keyword_sets() {
        let h = axis_header(&[("SITELAT", "19.82"), ("SITELONG", "-155.47"), ("SITEELEV", "4200")]);
        let (site, source) = site_location(&h).unwrap();
        assert_eq!(source, "SITELAT/SITELONG");
        assert_eq!(site, SiteLocation { lon_deg: -155.47, lat_deg: 19.82, height_m: 4200.0 });
        let h = axis_header(&[("LAT-OBS", "'+19 49 34.0'"), ("LONG-OBS", "'-155 28 12'")]);
        let (site, source) = site_location(&h).unwrap();
        assert_eq!(source, "LAT-OBS/LONG-OBS");
        assert!((site.lat_deg - 19.826111).abs() < 1e-5);
        assert!((site.lon_deg + 155.47).abs() < 1e-5);
        let h = axis_header(&[("OBSGEO-X", "-5464487.8"), ("OBSGEO-Y", "-2492806.0"), ("OBSGEO-Z", "2151240.2")]);
        let (site, source) = site_location(&h).unwrap();
        assert_eq!(source, "OBSGEO-X/Y/Z");
        assert!((site.lat_deg - 19.826).abs() < 0.02, "{}", site.lat_deg);
        assert!((site.lon_deg + 155.47).abs() < 0.02, "{}", site.lon_deg);
        assert!(site.height_m > 3000.0 && site.height_m < 4500.0, "{}", site.height_m);
        assert!(site_location(&axis_header(&[("OBJECT", "M31")])).is_none());
    }

    #[test]
    fn geodetic_round_trip_through_geocentric_coordinates() {
        let lat: f64 = 19.826_f64.to_radians();
        let lon: f64 = (-155.47_f64).to_radians();
        let h = 4205.0;
        let e2 = 2.0 * WGS84_F - WGS84_F * WGS84_F;
        let n = WGS84_A_M / (1.0 - e2 * lat.sin() * lat.sin()).sqrt();
        let x = (n + h) * lat.cos() * lon.cos();
        let y = (n + h) * lat.cos() * lon.sin();
        let z = (n * (1.0 - e2) + h) * lat.sin();
        let site = geocentric_to_geodetic(x, y, z);
        assert!((site.lat_deg - 19.826).abs() < 1e-8);
        assert!((site.lon_deg + 155.47).abs() < 1e-8);
        assert!((site.height_m - h).abs() < 1e-3);
    }

    #[test]
    fn target_coordinates_prefer_celestial_crval_then_named_cards() {
        let h = axis_header(&[("CTYPE1", "RA---TAN"), ("CTYPE2", "DEC--TAN"), ("CRVAL1", "150.1"), ("CRVAL2", "2.2")]);
        assert_eq!(header_target_coordinates(&h), Some((150.1, 2.2, "CRVAL1/CRVAL2")));
        let h = axis_header(&[("CTYPE1", "WAVE"), ("CRVAL1", "5000"), ("RA", "180.5"), ("DEC", "-10.25")]);
        assert_eq!(header_target_coordinates(&h), Some((180.5, -10.25, "RA/DEC")));
        let h = axis_header(&[("OBJCTRA", "'12 00 00'"), ("OBJCTDEC", "'-30 30 00'")]);
        let (ra, dec, source) = header_target_coordinates(&h).unwrap();
        assert!((ra - 180.0).abs() < 1e-9 && (dec + 30.5).abs() < 1e-9);
        assert_eq!(source, "OBJCTRA/OBJCTDEC");
        let h = axis_header(&[("RA", "'06:30:00'"), ("DEC", "'+45:00:00'")]);
        let (ra, dec, _) = header_target_coordinates(&h).unwrap();
        assert!((ra - 97.5).abs() < 1e-9 && (dec - 45.0).abs() < 1e-9);
        assert!(header_target_coordinates(&axis_header(&[("OBJECT", "M31")])).is_none());
    }

    #[test]
    fn mid_exposure_time_prefers_mjd_avg_then_date_obs_plus_half_exposure() {
        let h = axis_header(&[("MJD-AVG", "61120.5"), ("DATE-OBS", "2026-03-21T10:00:00"), ("EXPTIME", "600")]);
        let (jd, source) = mid_exposure_jd(&h).unwrap();
        assert_eq!(jd, jd_from_mjd(61120.5));
        assert!(source.contains("MJD-AVG"));
        let h = axis_header(&[("DATE-OBS", "2026-03-21T10:00:00"), ("EXPTIME", "600")]);
        let (jd, source) = mid_exposure_jd(&h).unwrap();
        let start = jd_from_gregorian(2026, 3, 21.0 + 10.0 / 24.0);
        assert!((jd - (start + 300.0 / SECONDS_PER_DAY)).abs() < 1e-9);
        assert!(source.contains("EXPTIME"));
        let h = axis_header(&[("DATE-OBS", "2026-03-21"), ("TIME-OBS", "10:00:00")]);
        let (jd, source) = mid_exposure_jd(&h).unwrap();
        assert!((jd - start).abs() < 1e-9);
        assert!(source.contains("TIME-OBS") && source.contains("EXPTIME missing"));
        let h = axis_header(&[("MJD-OBS", "61120.0"), ("EXPOSURE", "86400")]);
        let (jd, _) = mid_exposure_jd(&h).unwrap();
        assert_eq!(jd, jd_from_mjd(61120.5));
        assert!(mid_exposure_jd(&axis_header(&[("OBJECT", "M31")])).is_none());
    }

    #[test]
    fn sun_longitude_is_near_zero_at_the_2026_march_equinox() {
        let equinox = jd_from_gregorian(2026, 3, 20.0 + (14.0 + 46.0 / 60.0) / 24.0);
        let (longitude, radius) = sun_geometric_longitude_deg(equinox);
        let wrapped = if longitude > 180.0 { longitude - 360.0 } else { longitude };
        assert!(wrapped.abs() < 0.03, "{wrapped}");
        assert!((radius - 0.996).abs() < 0.002, "{radius}");
    }

    #[test]
    fn earth_orbital_speed_and_direction_match_kepler_at_the_march_equinox() {
        let jd = jd_from_gregorian(2026, 3, 20.0 + (14.0 + 46.0 / 60.0) / 24.0);
        let v = earth_heliocentric_velocity_kms(jd);
        let speed = dot(v, v).sqrt();
        assert!((speed - 29.9).abs() < 0.25, "{speed}");
        let toward_winter_solstice_point = target_unit_vector(270.0, -OBLIQUITY_J2000_DEG);
        let along = dot(v, toward_winter_solstice_point);
        assert!(along > 29.5 && along <= speed, "{along}");
        let toward_summer_solstice_point = target_unit_vector(90.0, OBLIQUITY_J2000_DEG);
        assert!(dot(v, toward_summer_solstice_point) < -29.5);
        let toward_sun = target_unit_vector(0.0, 0.0);
        assert!(dot(v, toward_sun).abs() < 1.0, "{}", dot(v, toward_sun));
    }

    #[test]
    fn earth_moon_barycentre_offset_and_solar_reflex_are_at_the_metre_per_second_level() {
        let jd = jd_from_gregorian(2026, 9, 19.0);
        let with_moon = earth_heliocentric_ecliptic_of_date_au(jd);
        let without_moon = earth_moon_barycentre_heliocentric_ecliptic_au(jd);
        let offset_km = dot(add(with_moon, scale(without_moon, -1.0)), add(with_moon, scale(without_moon, -1.0))).sqrt() * AU_KM;
        assert!(offset_km > 4000.0 && offset_km < 5100.0, "{offset_km}");
        let moon_speed = {
            let dt = 0.01;
            let a = moon_geocentric_ecliptic_au(jd + dt);
            let b = moon_geocentric_ecliptic_au(jd - dt);
            let d = scale(add(a, scale(b, -1.0)), 1.0 / (2.0 * dt));
            dot(d, d).sqrt() * AU_KM / SECONDS_PER_DAY
        };
        assert!((moon_speed - 1.02).abs() < 0.08, "{moon_speed}");
        let solar = sun_barycentric_velocity_kms(jd);
        let solar_speed = dot(solar, solar).sqrt();
        assert!(solar_speed > 0.004 && solar_speed < 0.018, "{solar_speed}");
    }

    #[test]
    fn diurnal_velocity_is_positive_toward_a_rising_target_on_the_eastern_horizon() {
        let site = SiteLocation { lon_deg: 0.0, lat_deg: 0.0, height_m: 0.0 };
        let jd = jd_from_gregorian(2026, 9, 19.0);
        let lst = gmst_deg(jd) + site.lon_deg;
        let v = diurnal_velocity_kms(&site, jd);
        let speed = dot(v, v).sqrt();
        assert!((speed - 0.4651).abs() < 0.001, "{speed}");
        let rising = target_unit_vector(lst + 90.0, 0.0);
        assert!((dot(v, rising) - speed).abs() < 1e-9);
        let setting = target_unit_vector(lst - 90.0, 0.0);
        assert!((dot(v, setting) + speed).abs() < 1e-9);
        let on_meridian = target_unit_vector(lst, 40.0);
        assert!(dot(v, on_meridian).abs() < 1e-9);
        let pole = target_unit_vector(0.0, 90.0);
        assert!(dot(v, pole).abs() < 1e-12);
        let mauna_kea = SiteLocation { lon_deg: -155.47, lat_deg: 19.82, height_m: 4200.0 };
        let v = diurnal_velocity_kms(&mauna_kea, jd);
        assert!((dot(v, v).sqrt() - 0.4651 * 19.82_f64.to_radians().cos()).abs() < 0.002);
    }

    fn mauna_kea_header(extra: &[(&str, &str)]) -> HduHeader {
        let mut pairs = vec![
            ("SITELAT", "19.82"),
            ("SITELONG", "-155.47"),
            ("DATE-OBS", "2026-03-21T10:00:00"),
            ("EXPTIME", "600"),
        ];
        pairs.extend_from_slice(extra);
        axis_header(&pairs)
    }

    #[test]
    fn jwst_header_without_velosys_is_refused_with_the_reason() {
        let h = axis_header(&[("TELESCOP", "JWST"), ("DATE-OBS", "2026-03-21T10:00:00")]);
        let err = radial_velocity_correction(&h, 180.0, 0.0).unwrap_err();
        assert!(err.contains("JWST") && err.contains("ephemeris") && err.contains("VELOSYS"), "{err}");
        let h = axis_header(&[("TELESCOP", "'HST'"), ("SITELAT", "0"), ("SITELONG", "0")]);
        assert!(radial_velocity_correction(&h, 180.0, 0.0).unwrap_err().contains("HST"));
    }

    #[test]
    fn spacecraft_header_with_velosys_reports_it_as_the_correction() {
        let h = axis_header(&[("TELESCOP", "JWST"), ("VELOSYS", "-12345.0"), ("SPECSYS", "BARYCENT")]);
        let c = radial_velocity_correction(&h, 180.0, 0.0).unwrap();
        assert_eq!(c.method, "already in BARYCENT");
        assert_eq!(c.barycentric_kms, 0.0);
        let h = axis_header(&[("TELESCOP", "JWST"), ("VELOSYS", "-12345.0"), ("SPECSYS", "TOPOCENT"), ("MJD-AVG", "61120.5")]);
        let c = radial_velocity_correction(&h, 180.0, 0.0).unwrap();
        assert_eq!(c.method, "header VELOSYS");
        assert!((c.barycentric_kms - 12.345).abs() < 1e-12, "VELOSYS -12345 m/s (observer approaching) is a +12.345 km/s correction: {}", c.barycentric_kms);
        assert_eq!(c.heliocentric_kms, c.barycentric_kms);
        assert_eq!(c.jd_mid, jd_from_mjd(61120.5));
        assert!(c.notes.iter().any(|n| n.contains("TOPOCENT")));
        assert_eq!(
            c.notes[0],
            "VELOSYS -12345 m/s from the header, JWST convention (positive = observer receding), applied as 12.345 km/s"
        );

        let receding = axis_header(&[("TELESCOP", "JWST"), ("VELOSYS", "20000"), ("SPECSYS", "TOPOCENT")]);
        let c = radial_velocity_correction(&receding, 180.0, 0.0).unwrap();
        assert!((c.barycentric_kms + 20.0).abs() < 1e-12, "{}", c.barycentric_kms);
        assert!(c.notes[0].ends_with("applied as -20 km/s"), "{}", c.notes[0]);
    }

    #[test]
    fn specsys_barycent_gives_zero_with_the_method_named() {
        let h = mauna_kea_header(&[("SPECSYS", "BARYCENT")]);
        let c = radial_velocity_correction(&h, 180.0, 0.0).unwrap();
        assert_eq!(c.method, "already in BARYCENT");
        assert_eq!(c.barycentric_kms, 0.0);
        assert_eq!(c.heliocentric_kms, 0.0);
        assert_eq!(c.accuracy_kms, 0.0);
        assert!(c.jd_mid.is_finite());
        let h = mauna_kea_header(&[("SPECSYS", "HELIOCEN")]);
        assert_eq!(radial_velocity_correction(&h, 180.0, 0.0).unwrap().method, "already in HELIOCEN");
    }

    #[test]
    fn frames_beyond_the_barycentre_give_zero_with_the_frame_named() {
        let toward_earth_motion = (270.0, -OBLIQUITY_J2000_DEG);
        for (specsys, method) in [
            ("LSRK", "already in LSRK"),
            ("LSR", "already in LSRK"),
            ("LSRD", "already in LSRD"),
            ("GALACTOC", "already in GALACTOC"),
            ("LOCALGRP", "already in LOCALGRP"),
            ("CMBDIPOL", "already in CMBDIPOL"),
            ("SOURCE", "already in SOURCE"),
        ] {
            let h = mauna_kea_header(&[("SPECSYS", specsys), ("TELESCOP", "ALMA")]);
            let c = radial_velocity_correction(&h, toward_earth_motion.0, toward_earth_motion.1).unwrap();
            assert_eq!(c.method, method, "{specsys}");
            assert_eq!(c.barycentric_kms, 0.0, "{specsys}");
            assert_eq!(c.heliocentric_kms, 0.0, "{specsys}");
            assert_eq!(c.accuracy_kms, 0.0);
            assert!(c.jd_mid.is_finite());
            assert!(c.notes.iter().any(|n| n.contains("SPECSYS") && n.contains(specsys)), "{:?}", c.notes);
        }
        let aips = mauna_kea_header(&[("CTYPE3", "VELO-LSR"), ("CRVAL3", "0.0"), ("CDELT3", "1000.0")]);
        let c = radial_velocity_correction(&aips, toward_earth_motion.0, toward_earth_motion.1).unwrap();
        assert_eq!(c.method, "already in LSRK");
        assert_eq!(c.barycentric_kms, 0.0);
        assert!(c.notes.iter().any(|n| n.contains("CTYPE3 suffix")), "{:?}", c.notes);
        let topocentric = mauna_kea_header(&[("SPECSYS", "TOPOCENT")]);
        let c = radial_velocity_correction(&topocentric, toward_earth_motion.0, toward_earth_motion.1).unwrap();
        assert_eq!(c.method, EPHEMERIS_METHOD);
        assert!(c.barycentric_kms > 29.0, "{}", c.barycentric_kms);
    }

    #[test]
    fn geocentric_frame_skips_the_diurnal_term_and_needs_no_site() {
        let target = target_unit_vector(180.0, 0.0);
        let topocentric = radial_velocity_correction(&mauna_kea_header(&[]), 180.0, 0.0).unwrap();
        let geocentric = radial_velocity_correction(&mauna_kea_header(&[("SPECSYS", "GEOCENTR")]), 180.0, 0.0).unwrap();
        assert_eq!(geocentric.method, EPHEMERIS_METHOD);
        assert_eq!(geocentric.accuracy_kms, CORRECTION_ACCURACY_KMS);
        let site = SiteLocation { lon_deg: -155.47, lat_deg: 19.82, height_m: 0.0 };
        let diurnal = dot(diurnal_velocity_kms(&site, topocentric.jd_mid), target);
        assert!(diurnal > 0.03 && diurnal < 0.08, "{diurnal}");
        assert!((topocentric.heliocentric_kms - geocentric.heliocentric_kms - diurnal).abs() < 1e-9);
        assert!((topocentric.barycentric_kms - geocentric.barycentric_kms - diurnal).abs() < 1e-9);
        assert!(geocentric.notes.iter().any(|n| n.contains("diurnal term omitted") && n.contains("GEOCENTR")), "{:?}", geocentric.notes);
        assert!(!geocentric.notes.iter().any(|n| n.contains("east-positive")));
        let no_site = axis_header(&[("SPECSYS", "GEOCENTR"), ("DATE-OBS", "2026-03-21T10:00:00"), ("EXPTIME", "600")]);
        let c = radial_velocity_correction(&no_site, 180.0, 0.0).unwrap();
        assert!((c.barycentric_kms - geocentric.barycentric_kms).abs() < 1e-9);
        let aips_geo = axis_header(&[("CTYPE3", "FREQ-GEO"), ("DATE-OBS", "2026-03-21T10:00:00"), ("EXPTIME", "600")]);
        let c = radial_velocity_correction(&aips_geo, 180.0, 0.0).unwrap();
        assert!((c.barycentric_kms - geocentric.barycentric_kms).abs() < 1e-9);
        let no_time = axis_header(&[("SPECSYS", "GEOCENTR")]);
        assert!(radial_velocity_correction(&no_time, 180.0, 0.0).unwrap_err().contains("observation time"));
    }

    #[test]
    fn longitude_sign_convention_is_named_only_for_keyword_pair_sites() {
        let c = radial_velocity_correction(&mauna_kea_header(&[]), 180.0, 0.0).unwrap();
        let note = c.notes.iter().find(|n| n.contains("east-positive")).expect("east-positive note");
        assert!(note.contains("SITELAT/SITELONG"), "{note}");
        let bound: f64 = note.split("up to ").nth(1).and_then(|s| s.split(' ').next()).unwrap().parse().unwrap();
        assert!((bound - 2.0 * 0.4651 * 19.82_f64.to_radians().cos()).abs() < 0.01, "{bound}");
        let h = axis_header(&[
            ("OBSGEO-X", "-5464487.8"),
            ("OBSGEO-Y", "-2492806.0"),
            ("OBSGEO-Z", "2151240.2"),
            ("DATE-OBS", "2026-03-21T10:00:00"),
        ]);
        let c = radial_velocity_correction(&h, 180.0, 0.0).unwrap();
        assert!(!c.notes.iter().any(|n| n.contains("east-positive")), "{:?}", c.notes);
    }

    #[test]
    fn ground_based_mauna_kea_march_2026_toward_ra_180_is_small_and_positive() {
        let h = mauna_kea_header(&[]);
        let c = radial_velocity_correction(&h, 180.0, 0.0).unwrap();
        assert_eq!(c.method, "Meeus low-precision ephemeris");
        assert_eq!(c.accuracy_kms, CORRECTION_ACCURACY_KMS);
        assert!(c.barycentric_kms.abs() < 30.0);
        assert!(c.barycentric_kms.abs() < 1.0, "{}", c.barycentric_kms);
        let jd_tt = c.jd_mid + TT_MINUS_UTC_SECONDS / SECONDS_PER_DAY;
        let target = target_unit_vector(180.0, 0.0);
        let orbital = dot(earth_heliocentric_velocity_kms(jd_tt), target);
        let (sun_longitude_of_date, radius) = sun_geometric_longitude_deg(jd_tt);
        assert!(sun_longitude_of_date > 0.5 && sun_longitude_of_date < 1.2, "{sun_longitude_of_date}");
        let t = julian_centuries_j2000(jd_tt);
        let sun_longitude_j2000 = sun_longitude_of_date - (1.3969713 * t + 0.0003086 * t * t);
        assert!(sun_longitude_j2000 > 0.2 && sun_longitude_j2000 < 0.7, "{sun_longitude_j2000}");
        let eccentricity = 0.016708634;
        let mean_speed_kms = AU_KM / SECONDS_PER_DAY * 2.0 * std::f64::consts::PI / 365.25636;
        let earth_perihelion_longitude_deg = 102.94;
        let true_anomaly = (sun_longitude_of_date + 180.0 - earth_perihelion_longitude_deg).to_radians();
        let radial_outward_kms = mean_speed_kms * eccentricity * true_anomaly.sin() / (1.0 - eccentricity * eccentricity).sqrt();
        assert!(radial_outward_kms > 0.44 && radial_outward_kms < 0.52, "{radial_outward_kms}");
        let transverse_kms = mean_speed_kms * (1.0 - eccentricity * eccentricity).sqrt() / radius;
        let expected_orbital = radial_outward_kms * sun_longitude_j2000.to_radians().cos()
            - transverse_kms * sun_longitude_j2000.to_radians().sin();
        assert!(expected_orbital > 0.15 && expected_orbital < 0.4, "{expected_orbital}");
        assert!((orbital - expected_orbital).abs() < 0.03, "orbital {orbital} vs {expected_orbital}");
        let diurnal = dot(diurnal_velocity_kms(&SiteLocation { lon_deg: -155.47, lat_deg: 19.82, height_m: 0.0 }, c.jd_mid), target);
        assert!(diurnal > 0.03 && diurnal < 0.08, "{diurnal}");
        assert!((c.heliocentric_kms - (orbital + diurnal)).abs() < 1e-9);
        assert!(c.barycentric_kms > 0.0, "{}", c.barycentric_kms);
        assert!((c.barycentric_kms - c.heliocentric_kms).abs() < 0.02);
        assert!(c.notes.iter().any(|n| n.contains("SITELAT/SITELONG")));
        assert!(c.notes.iter().any(|n| n.contains("EXPTIME")));
    }

    #[test]
    fn ground_based_correction_is_near_plus_and_minus_thirty_along_the_earth_velocity() {
        let h = mauna_kea_header(&[]);
        let toward = radial_velocity_correction(&h, 270.0, -OBLIQUITY_J2000_DEG).unwrap();
        assert!(toward.barycentric_kms > 29.3 && toward.barycentric_kms < 30.4, "{}", toward.barycentric_kms);
        let away = radial_velocity_correction(&h, 90.0, OBLIQUITY_J2000_DEG).unwrap();
        assert!(away.barycentric_kms < -29.3 && away.barycentric_kms > -30.4, "{}", away.barycentric_kms);
        assert!((toward.barycentric_kms + away.barycentric_kms).abs() < 1.5);
    }

    #[test]
    fn ground_based_header_without_site_or_time_is_refused() {
        let h = axis_header(&[("DATE-OBS", "2026-03-21T10:00:00")]);
        assert!(radial_velocity_correction(&h, 180.0, 0.0).unwrap_err().contains("observatory location"));
        let h = axis_header(&[("SITELAT", "19.82"), ("SITELONG", "-155.47")]);
        assert!(radial_velocity_correction(&h, 180.0, 0.0).unwrap_err().contains("observation time"));
        let h = mauna_kea_header(&[]);
        assert!(radial_velocity_correction(&h, 180.0, 95.0).is_err());
    }
}
