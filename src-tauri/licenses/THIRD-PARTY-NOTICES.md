# Third-party notices

AstroBurst ships this folder with the desktop bundles and the Linux release tarball. Each row names a third-party work that AstroBurst ports or links, the version in use, its licence, its home page and the licence files in this folder. Where a crate offers a choice of licences, every licence text the crate ships is included.

Scope: the ports and crates listed below. The rest of the Rust and npm dependency trees is not listed here yet. Crate rows match `src-tauri/Cargo.lock`; `src/__tests__/thirdPartyNotices.test.ts` fails when a crate in the fitsio-pure tree has no row at its locked version, a row names a file missing from this folder, a file here has no row, or a repository path named here does not exist.

## Ported code

| Name | Version | Licence | URL | File |
|---|---|---|---|---|
| gwcs | 1.0.3 | BSD-3-Clause. The gWCS evaluator and the TAN-SIP fit (`to_fits_sip`) under `src-tauri/src/core/astrometry/gwcs/` follow its semantics. | https://github.com/spacetelescope/gwcs | `gwcs-LICENSE.rst` |
| asdf-astropy | 0.11.0 | BSD-3-Clause. The ASDF time decode (`src-tauri/src/infra/asdf/time.rs`) follows the semantics of its time converter (`asdf_astropy/converters/time/time.py`). | https://github.com/astropy/asdf-astropy | `asdf-astropy-LICENSE.rst` |
| cfitsio | as ported into fitsio-pure 0.21.6 | CFITSIO licence (NASA/HEASARC). Covers the cfitsio code fitsio-pure ports, including `ricecomp.c`, `quantize.c`, `imcompress.c`, `fits_hdecompress.c`, the PLIO line-list decoder (`pl_l2pi`) and the checksum routines `ffcsum`, `ffesum` and `ffdsum` (`checksum.c`). Text from the CFITSIO User's Reference Guide, section 1.4 "Legal Stuff": https://heasarc.gsfc.nasa.gov/docs/software/fitsio/c/c_user/node6.html | https://heasarc.gsfc.nasa.gov/fitsio/ | `cfitsio-LICENSE.txt` |

## Rust crates

| Name | Version | Licence | URL | File |
|---|---|---|---|---|
| fitsio-pure | 0.21.6 | Apache-2.0. The published crate ships no licence file; the file here is the standard Apache-2.0 text from https://www.apache.org/licenses/LICENSE-2.0.txt (sha256 cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30). | https://github.com/OrbitalCommons/fitsio-pure | `fitsio-pure-LICENSE-APACHE` |
| libm | 0.2.16 | MIT. `libm-LICENSE.txt` carries the musl libc copyright and the musl math notice; `libm-NOTICE-sun.txt` carries the Sun Microsystems notices from the crate's sources. fitsio-pure calls `libm::round` (musl origin); AstroBurst depends on it directly for `libm::j1` in the Airy PSF (`src-tauri/src/core/synth/psf.rs`), from FreeBSD msun `e_j1.c` under the SunSoft notice. | https://github.com/rust-lang/compiler-builtins | `libm-LICENSE.txt`, `libm-NOTICE-sun.txt` |
| miniz_oxide | 0.9.1 | MIT OR Zlib OR Apache-2.0 | https://github.com/Frommi/miniz_oxide | `miniz_oxide-LICENSE`, `miniz_oxide-LICENSE-MIT.md`, `miniz_oxide-LICENSE-ZLIB.md`, `miniz_oxide-LICENSE-APACHE.md` |
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 | https://github.com/oyvindln/adler2 | `adler2-LICENSE-0BSD`, `adler2-LICENSE-MIT`, `adler2-LICENSE-APACHE` |
| bytemuck | 1.25.0 | Zlib OR Apache-2.0 OR MIT | https://github.com/Lokathor/bytemuck | `bytemuck-LICENSE-ZLIB`, `bytemuck-LICENSE-MIT`, `bytemuck-LICENSE-APACHE` |
| rayon | 1.12.0 | MIT OR Apache-2.0 | https://github.com/rayon-rs/rayon | `rayon-LICENSE-MIT`, `rayon-LICENSE-APACHE` |
| rayon-core | 1.13.0 | MIT OR Apache-2.0 | https://github.com/rayon-rs/rayon | `rayon-core-LICENSE-MIT`, `rayon-core-LICENSE-APACHE` |
| crossbeam-deque | 0.8.6 | MIT OR Apache-2.0 | https://github.com/crossbeam-rs/crossbeam | `crossbeam-deque-LICENSE-MIT`, `crossbeam-deque-LICENSE-APACHE` |
| crossbeam-epoch | 0.9.18 | MIT OR Apache-2.0 | https://github.com/crossbeam-rs/crossbeam | `crossbeam-epoch-LICENSE-MIT`, `crossbeam-epoch-LICENSE-APACHE` |
| crossbeam-utils | 0.8.21 | MIT OR Apache-2.0 | https://github.com/crossbeam-rs/crossbeam | `crossbeam-utils-LICENSE-MIT`, `crossbeam-utils-LICENSE-APACHE` |
| either | 1.15.0 | MIT OR Apache-2.0 | https://github.com/rayon-rs/either | `either-LICENSE-MIT`, `either-LICENSE-APACHE` |
