#[test]
fn pipeline_re_exports_newton_options_and_vals() {
    let opts = super::pipeline::NewtonOptions::default();
    assert_eq!((opts.tol_px, opts.max_iter, opts.h_px), (1e-7, 10, 0.01));
    let v = super::pipeline::Vals::pair(1.0, 2.0);
    assert_eq!(v.as_slice(), &[1.0, 2.0]);
    let g: super::GwcsPipeline = match super::test_support::try_fixture_pipeline("wcs_gwcs_examples_tan.asdf") {
        Ok(p) => p,
        Err(e) => panic!("{e:#}"),
    };
    let r = g.backward_exact(29.980362905902044, 44.986109428818445, &opts);
    assert!(r.converged, "{r:?}");
    super::test_support::assert_close_slice(&r.pixel, &[0.0, 0.0], 1e-6, "TAN example origin");
}

#[test]
fn real_data_directories_come_only_from_the_environment() {
    use super::test_support::{env_dir, skip_if_absent};
    let unset = env_dir("ASTROBURST_GWCS_TEST_UNSET_VARIABLE");
    assert_eq!(unset.to_str(), Some("<ASTROBURST_GWCS_TEST_UNSET_VARIABLE unset>"));
    assert!(skip_if_absent(&unset.join("any.fits")), "an unset variable skips the real-data test");
}
