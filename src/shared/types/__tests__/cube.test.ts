import { describe, expect, expectTypeOf, it } from "vitest";
import type { LineFitComponentFit, LineFitConfig, LineFitPlane } from "../cube";

describe("cube line-fit types with a redshift", () => {
  it("a line-fit config may carry a redshift and old configs still type-check", () => {
    const withZ: LineFitConfig = {
      z0: 530,
      z1: 560,
      rest_um: 1.875613,
      convention: "optical",
      continuum: null,
      snr_threshold: 3,
      emission_only: true,
      use_err: true,
      use_dq: true,
      resolving_power: null,
      components: "one",
      redshift: 0.5,
    };
    const plain: LineFitConfig = {
      z0: 530,
      z1: 560,
      rest_um: 1.875613,
      convention: "optical",
      continuum: null,
      snr_threshold: 3,
      emission_only: true,
      use_err: true,
      use_dq: true,
      resolving_power: null,
      components: "one",
    };
    expect(withZ.redshift).toBe(0.5);
    expect(plain.redshift).toBeUndefined();
    expectTypeOf<LineFitConfig["redshift"]>().toEqualTypeOf<number | null | undefined>();
  });

  it("the plane names include the source-frame maps the backend writes", () => {
    const planes: LineFitPlane[] = [
      "flux",
      "velocity",
      "sigma_obs",
      "sigma_corr",
      "flux_err",
      "v_err",
      "sigma_err",
      "chi2_red",
      "snr",
      "mask",
      "ncomp",
      "source_flux",
      "source_velocity",
      "source_sigma_obs",
      "source_sigma_corr",
      "source_flux_err",
      "source_v_err",
      "source_sigma_err",
      "c1_flux",
      "c2_sigma_err",
      "c1_source_flux",
      "c1_source_velocity",
      "c1_source_sigma_obs",
      "c1_source_flux_err",
      "c1_source_v_err",
      "c1_source_sigma_err",
      "c2_source_flux",
      "c2_source_velocity",
      "c2_source_sigma_obs",
      "c2_source_flux_err",
      "c2_source_v_err",
      "c2_source_sigma_err",
    ];
    expect(planes).toHaveLength(32);
    expectTypeOf<"c1_source_sigma_corr">().not.toMatchTypeOf<LineFitPlane>();
    expectTypeOf<"source_snr">().not.toMatchTypeOf<LineFitPlane>();
  });

  it("a fitted component may carry the seven source-frame values", () => {
    const full: LineFitComponentFit = {
      amplitude: 2,
      centre: 2.8134195,
      sigma: 0.0014,
      amplitude_err: 0.1,
      centre_err: 0.0001,
      sigma_err: 0.0001,
      velocity_kms: 149896.229,
      v_err_kms: 10,
      sigma_kms: 149.2,
      sigma_err_kms: 10.7,
      sigma_corr_kms: null,
      flux: 0.00702,
      flux_err: 0.0006,
      snr: 20,
      source_velocity_kms: 0,
      source_v_err_kms: 6.7,
      source_sigma_kms: 99.5,
      source_sigma_err_kms: 7.1,
      source_sigma_corr_kms: null,
      source_flux: 0.00468,
      source_flux_err: 0.0004,
    };
    const short: LineFitComponentFit = {
      amplitude: 2,
      centre: 2.8134195,
      sigma: 0.0014,
      amplitude_err: 0.1,
      centre_err: 0.0001,
      sigma_err: 0.0001,
      velocity_kms: 149896.229,
      v_err_kms: 10,
      sigma_kms: 149.2,
      sigma_err_kms: 10.7,
      sigma_corr_kms: null,
      flux: 0.00702,
      flux_err: 0.0006,
      snr: 20,
    };
    expect(short.source_flux).toBeUndefined();
    expect(Object.keys(short)).toHaveLength(14);
    expectTypeOf<LineFitComponentFit["source_sigma_corr_kms"]>().toEqualTypeOf<number | null | undefined>();
    expect(Object.keys(full)).toHaveLength(21);
  });
});
