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
| cdshealpix | 0.9.1 | Apache-2.0 OR MIT. Copyright (c) 2019 Universite de Strasbourg / CNRS. The HEALPix catalogue reader (`src-tauri/src/core/astrometry/healpix_cat.rs`) calls its NESTED cone coverage and pixel centres. The rows from bincode to iana-time-zone below are the crates cdshealpix 0.9.1 added to the normal dependency trees of the Windows, Linux and macOS builds (`cargo tree -e normal --target <triple>` before and after the change). | https://github.com/cds-astro/cds-healpix-rust | `cdshealpix-LICENSE-MIT`, `cdshealpix-LICENSE-APACHE` |
| bincode | 1.3.3 | MIT | https://github.com/servo/bincode | `bincode-LICENSE.md` |
| chrono | 0.4.44 | MIT OR Apache-2.0. The crate ships both texts in one file. | https://github.com/chronotope/chrono | `chrono-LICENSE.txt` |
| colorous | 1.0.16 | Apache-2.0. `colorous-NOTICE` carries the d3-scale-chromatic (BSD-3-Clause, Mike Bostock) and ColorBrewer (Apache-2.0, Cynthia Brewer, Mark Harrower and The Pennsylvania State University) notices the crate ships. | https://github.com/dtolnay/colorous | `colorous-LICENSE-APACHE`, `colorous-NOTICE` |
| katex-doc | 0.1.0 | Unlicense OR MIT. The published crate ships no licence file and holds only documentation helpers; the file here is the standard Unlicense text as byteorder 1.5.0 ships it (sha256 7e12e5df4bae12cb21581ba157ced20e1986a0508dd10d0e8a4ab9a4cf94e85c). | https://github.com/CAD97/katex-doc | `katex-doc-UNLICENSE` |
| num | 0.4.3 | MIT OR Apache-2.0 | https://github.com/rust-num/num | `num-LICENSE-MIT`, `num-LICENSE-APACHE` |
| num-iter | 0.1.46 | MIT OR Apache-2.0 | https://github.com/rust-num/num-iter | `num-iter-LICENSE-MIT`, `num-iter-LICENSE-APACHE` |
| serde_spanned | 0.6.9 | MIT OR Apache-2.0 | https://github.com/toml-rs/toml | `serde_spanned-LICENSE-MIT`, `serde_spanned-LICENSE-APACHE` |
| toml | 0.8.2 | MIT OR Apache-2.0 | https://github.com/toml-rs/toml | `toml-LICENSE-MIT`, `toml-LICENSE-APACHE` |
| toml_datetime | 0.6.3 | MIT OR Apache-2.0 | https://github.com/toml-rs/toml | `toml_datetime-LICENSE-MIT`, `toml_datetime-LICENSE-APACHE` |
| toml_edit | 0.20.2 | MIT OR Apache-2.0 | https://github.com/toml-rs/toml | `toml_edit-LICENSE-MIT`, `toml_edit-LICENSE-APACHE` |
| winnow | 0.5.40 | MIT | https://github.com/winnow-rs/winnow | `winnow-LICENSE-MIT` |
| iana-time-zone | 0.1.65 | MIT OR Apache-2.0. Linked only by the Linux and macOS builds, through chrono's `clock` feature on Unix targets. | https://github.com/strawlab/iana-time-zone | `iana-time-zone-LICENSE-MIT`, `iana-time-zone-LICENSE-APACHE` |
| bzip2 | 0.5.2 | MIT OR Apache-2.0. A required dependency of every build: the catalogue store (`src-tauri/src/infra/catalog_store.rs`) decodes the bzip2-compressed Siril catalogue extract with it, and the ASDF reader (`src-tauri/src/infra/asdf/blocks.rs`) decodes bzip2 blocks. | https://github.com/trifectatechfoundation/bzip2-rs | `bzip2-LICENSE-MIT`, `bzip2-LICENSE-APACHE` |
| bzip2-sys | 0.1.13+1.0.8 | MIT OR Apache-2.0 for the binding. It compiles the libbzip2 1.0.8 C sources it bundles on Windows, and on Linux and macOS when pkg-config finds no system libbzip2; `libbzip2-LICENSE` is the libbzip2 licence (BSD-style, copyright Julian R Seward) that the crate ships as `bzip2-1.0.8/LICENSE`. | https://github.com/alexcrichton/bzip2-rs | `bzip2-sys-LICENSE-MIT`, `bzip2-sys-LICENSE-APACHE`, `libbzip2-LICENSE` |

## Data

| Name | Version | Licence | URL | File |
|---|---|---|---|---|
| HST/WFPC2 M16 narrowband frames | 1995 observations, as shipped in `exampleFits/sample-data/` | Public-domain Hubble archive data; credit requested by STScI: NASA, ESA, J. Hester and P. Scowen (Arizona State University). Bundled with the installers under `sample-data/` for the Open sample data entry. | https://archive.stsci.edu/ | `hst-m16-sample-data-NOTICE.txt` |
