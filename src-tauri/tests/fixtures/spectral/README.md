# Synthetic Brγ reference

`brgamma_reference.json` is a generated, identified numerical fixture for Brγ at the requested rest wavelength of 2.16612 µm. It contains six synthetic spaxels on a 3 × 2 grid with 121 spectral channels. It is not the unidentified observational cube or spaxels discussed in the roadmap.

Regenerate it from the repository root with Python, NumPy and SciPy installed:

```powershell
python scripts/oracle/brgamma_reference.py
```

The recorded reference was generated with Python 3.15.0, NumPy 2.5.4 and SciPy 1.18.1. The JSON records those versions, the generator SHA-256 and a SHA-256 for each spaxel's SCI, ERR and DQ arrays encoded as little-endian float32, float32 and uint32, respectively. The Rust test constructs and reopens a MEF with SCI/ERR/DQ from these arrays, including unsigned high flags, rather than calling Python during ordinary tests.

Run the regression with:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test brgamma_reference -- --nocapture
```

## Cases and independent reference

- Emission at positive optical velocity.
- Emission at negative velocity with bit 24 set but DO_NOT_USE clear.
- Outliers in both a line and a continuum channel, excluded by DO_NOT_USE together with high bits; bit 31 alone remains usable.
- An absorption line with negative integrated flux.
- A convergent fit below the integrated S/N detection threshold.
- A fully excluded spaxel with no usable measurement.

The reference first estimates a weighted linear continuum with NumPy least squares. It then uses [SciPy `curve_fit`](https://docs.scipy.org/doc/scipy/reference/generated/scipy.optimize.curve_fit.html), with MINPACK and absolute supplied uncertainties, to fit a Gaussian in optical velocity coordinates. Central refits under perturbations of each continuum coefficient propagate the continuum uncertainty. This provides an implementation independent of AstroBurst's Gaussian solver and analytic continuum-sensitivity calculation. The conditional fit covariance and the propagated continuum covariance are both retained.

The comparison budgets were selected before running the Rust test: 0.02 km/s for velocity and observed width (one thousandth of a channel), 0.05% for amplitude and integrated flux, and 1% for uncertainties and S/N. The uncertainty allowance accounts for the difference between linearized Gauss–Newton covariance and sensitivity estimated by finite refits of a fit with nonzero residuals. Tolerances do not establish equivalent accuracy in real observations.

The flux unit is **Jy/beam km/s**, matching a velocity integral of an input in Jy/beam; it is not automatically an energy flux in erg/s/cm². The fixture uses a linear wavelength axis, optical velocities, independent channel uncertainties and a Gaussian instrumental width derived from resolving power 2700. It does not test barycentric correction, correlated noise, an empirical line-spread function, two-component identification or the missing real Brγ case.
