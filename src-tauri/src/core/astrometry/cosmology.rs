use serde::Serialize;

use crate::core::metadata::photcal::ARCSEC_PER_RADIAN;

pub const SPEED_OF_LIGHT_KM_S: f64 = 299_792.458;
pub const PLANCK18_H0_KM_S_MPC: f64 = 67.66;
pub const PLANCK18_OMEGA_M: f64 = 0.30966;
pub const PLANCK18_OMEGA_NU_MASSIVE: f64 = 0.0014147613306646316;
pub const PLANCK18_OMEGA_RAD: f64 = 7.893312479128647e-5;

const MIN_H0_KM_S_MPC: f64 = 20.0;
const MAX_H0_KM_S_MPC: f64 = 200.0;
const SIMPSON_HALF_INTERVALS_PER_UNIT_REDSHIFT: f64 = 256.0;
const SIMPSON_MAX_INTERVALS: f64 = 2_000_000.0;
const KPC_PER_MPC: f64 = 1000.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct FlatLcdm {
    pub h0_km_s_mpc: f64,
    pub omega_m: f64,
    pub omega_nu_massive: f64,
    pub omega_rad: f64,
}

impl FlatLcdm {
    pub fn planck18() -> FlatLcdm {
        FlatLcdm {
            h0_km_s_mpc: PLANCK18_H0_KM_S_MPC,
            omega_m: PLANCK18_OMEGA_M,
            omega_nu_massive: PLANCK18_OMEGA_NU_MASSIVE,
            omega_rad: PLANCK18_OMEGA_RAD,
        }
    }

    pub fn with(h0_km_s_mpc: f64, omega_m: f64) -> Result<FlatLcdm, String> {
        if !(MIN_H0_KM_S_MPC..=MAX_H0_KM_S_MPC).contains(&h0_km_s_mpc) {
            return Err("H0 must be between 20 and 200 km/s/Mpc".into());
        }
        if !(omega_m > 0.0 && omega_m < 1.0) {
            return Err("Omega_m must be above 0 and below 1".into());
        }
        let inverse_h_squared = (PLANCK18_H0_KM_S_MPC / h0_km_s_mpc).powi(2);
        let omega_nu_massive = PLANCK18_OMEGA_NU_MASSIVE * inverse_h_squared;
        let omega_rad = PLANCK18_OMEGA_RAD * inverse_h_squared;
        if omega_m + omega_nu_massive + omega_rad >= 1.0 {
            return Err(format!(
                "H0 {h0_km_s_mpc} and Omega_m {omega_m} leave no dark energy in a flat model"
            ));
        }
        Ok(FlatLcdm { h0_km_s_mpc, omega_m, omega_nu_massive, omega_rad })
    }

    pub fn efunc(&self, z: f64) -> f64 {
        let matter = self.omega_m + self.omega_nu_massive;
        let radiation = self.omega_rad;
        let stretch = 1.0 + z;
        (matter * stretch.powi(3) + radiation * stretch.powi(4) + (1.0 - matter - radiation)).sqrt()
    }

    pub fn comoving_distance_mpc(&self, z: f64) -> Option<f64> {
        if !z.is_finite() || z <= 0.0 {
            return None;
        }
        let intervals = (2.0 * (SIMPSON_HALF_INTERVALS_PER_UNIT_REDSHIFT * z.max(1.0)).ceil())
            .min(SIMPSON_MAX_INTERVALS) as usize;
        let step = z / intervals as f64;
        let mut weighted_sum = 1.0 / self.efunc(0.0) + 1.0 / self.efunc(z);
        for node in 1..intervals {
            let weight = if node % 2 == 1 { 4.0 } else { 2.0 };
            weighted_sum += weight / self.efunc(node as f64 * step);
        }
        Some(SPEED_OF_LIGHT_KM_S / self.h0_km_s_mpc * weighted_sum * step / 3.0)
    }

    pub fn angular_diameter_distance_mpc(&self, z: f64) -> Option<f64> {
        self.comoving_distance_mpc(z).map(|comoving| comoving / (1.0 + z))
    }

    pub fn kpc_per_arcsec(&self, z: f64) -> Option<f64> {
        self.angular_diameter_distance_mpc(z)
            .map(|distance| distance * KPC_PER_MPC / ARCSEC_PER_RADIAN)
    }

    pub fn description(&self) -> String {
        format!(
            "flat LambdaCDM, H0 {}, Omega_m {}, plus Planck 2018 neutrinos and radiation",
            self.h0_km_s_mpc, self.omega_m
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REDSHIFTS: [f64; 8] = [0.1, 0.5, 1.0, 1.25, 1.592, 2.0, 3.0, 6.0];
    const PLANCK18_DESCRIPTION: &str =
        "flat LambdaCDM, H0 67.66, Omega_m 0.30966, plus Planck 2018 neutrinos and radiation";

    fn assert_relative(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance * expected.abs(),
            "{actual} differs from {expected} by more than {tolerance} relative"
        );
    }

    fn planck18_scale(z: f64) -> f64 {
        FlatLcdm::planck18().kpc_per_arcsec(z).unwrap()
    }

    #[test]
    fn planck18_scale_matches_the_closed_form_of_the_same_four_numbers() {
        let closed_form = [
            1.9064888157523507,
            6.290996434126971,
            8.23125144264795,
            8.563109343325603,
            8.690188778498625,
            8.578278569801899,
            7.88308033308079,
            5.8351568837634185,
        ];
        for (z, expected) in REDSHIFTS.iter().zip(closed_form) {
            assert_relative(planck18_scale(*z), expected, 1e-9);
        }
    }

    #[test]
    fn planck18_scale_is_within_1e_6_of_astropy_up_to_z_2_and_5e_6_up_to_z_6() {
        let astropy = [
            (1.9064888022022317, 1e-6),
            (6.29099609122652, 1e-6),
            (8.231250238307116, 1e-6),
            (8.563107626407612, 1e-6),
            (8.690186345596157, 1e-6),
            (8.578275309405592, 1e-6),
            (7.883075289584453, 5e-6),
            (5.835148217079811, 5e-6),
        ];
        for (z, (expected, tolerance)) in REDSHIFTS.iter().zip(astropy) {
            assert_relative(planck18_scale(*z), expected, tolerance);
        }
    }

    #[test]
    fn angular_diameter_distance_at_z_1_is_1697_817_mpc() {
        let model = FlatLcdm::planck18();
        let angular = model.angular_diameter_distance_mpc(1.0).unwrap();
        assert_relative(angular, 1697.8174839889118, 1e-9);
        assert_eq!(model.comoving_distance_mpc(1.0).unwrap(), 2.0 * angular);
    }

    #[test]
    fn the_scale_peaks_near_z_1_59() {
        let peak = planck18_scale(1.5921);
        assert!(peak > planck18_scale(1.55));
        assert!(peak > planck18_scale(1.65));
        assert_relative(peak, 8.6901888, 1e-6);
    }

    #[test]
    fn a_redshift_that_is_not_positive_and_finite_has_no_distance() {
        let model = FlatLcdm::planck18();
        for z in [0.0, -0.5, f64::NAN, f64::INFINITY] {
            assert_eq!(model.comoving_distance_mpc(z), None, "comoving at {z}");
            assert_eq!(model.angular_diameter_distance_mpc(z), None, "angular at {z}");
            assert_eq!(model.kpc_per_arcsec(z), None, "scale at {z}");
        }
    }

    #[test]
    fn a_small_redshift_follows_the_hubble_law() {
        assert_relative(planck18_scale(1e-6), 2.1481422682168715e-5, 1e-9);
        let hubble_law_mpc = SPEED_OF_LIGHT_KM_S * 1e-6 / PLANCK18_H0_KM_S_MPC;
        assert_relative(hubble_law_mpc, 4.4309e-3, 1e-4);
        assert_relative(
            FlatLcdm::planck18().comoving_distance_mpc(1e-6).unwrap(),
            hubble_law_mpc,
            1e-6,
        );
    }

    #[test]
    fn with_scales_the_neutrino_and_radiation_terms_by_h_squared() {
        let model = FlatLcdm::with(70.0, 0.3).unwrap();
        assert_relative(model.omega_nu_massive, 0.0013217553827496221, 1e-12);
        assert_relative(model.omega_rad, 7.374408694158886e-5, 1e-12);
        let at_z_1 = model.kpc_per_arcsec(1.0).unwrap();
        let at_z_2 = model.kpc_per_arcsec(2.0).unwrap();
        assert_relative(at_z_1, 8.001803902976224, 1e-9);
        assert_relative(at_z_2, 8.3602362004275, 1e-9);
        assert_relative(at_z_1, 8.001802787925689, 1e-6);
        assert_relative(at_z_2, 8.360233149553347, 1e-6);
        assert_eq!(FlatLcdm::with(67.66, 0.30966).unwrap(), FlatLcdm::planck18());
    }

    #[test]
    fn planck18_carries_the_four_pinned_numbers() {
        let model = FlatLcdm::planck18();
        assert_eq!(model.h0_km_s_mpc, 67.66);
        assert_eq!(model.omega_m, 0.30966);
        assert_eq!(model.omega_nu_massive, 0.0014147613306646316);
        assert_eq!(model.omega_rad, 7.893312479128647e-5);
        let matter: f64 = 0.30966 + 0.0014147613306646316;
        let radiation: f64 = 7.893312479128647e-5;
        let expected = (matter * 8.0 + radiation * 16.0 + (1.0 - matter - radiation)).sqrt();
        assert_relative(model.efunc(1.0), expected, 1e-15);
        assert_relative(model.efunc(0.0), 1.0, 1e-15);
    }

    #[test]
    fn with_refuses_parameters_outside_a_flat_model() {
        for h0 in [0.0, 250.0, f64::NAN] {
            assert_eq!(
                FlatLcdm::with(h0, 0.3).unwrap_err(),
                "H0 must be between 20 and 200 km/s/Mpc"
            );
        }
        for omega_m in [0.0, 1.0, f64::NAN] {
            assert_eq!(
                FlatLcdm::with(70.0, omega_m).unwrap_err(),
                "Omega_m must be above 0 and below 1"
            );
        }
        let no_dark_energy = FlatLcdm::with(20.0, 0.99).unwrap_err();
        assert!(
            no_dark_energy.starts_with("H0 20 and Omega_m 0.99 leave no dark energy"),
            "{no_dark_energy}"
        );
        assert_eq!(
            no_dark_energy,
            "H0 20 and Omega_m 0.99 leave no dark energy in a flat model"
        );
    }

    #[test]
    fn description_names_the_two_editable_numbers() {
        assert_eq!(FlatLcdm::planck18().description(), PLANCK18_DESCRIPTION);
        assert_eq!(
            FlatLcdm::with(70.0, 0.3).unwrap().description(),
            "flat LambdaCDM, H0 70, Omega_m 0.3, plus Planck 2018 neutrinos and radiation"
        );
    }
}
