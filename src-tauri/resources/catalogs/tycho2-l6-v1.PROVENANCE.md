# Tier-0 catalogue provenance

- File: `tycho2-l6-v1.abcat`
- SHA-256: `bf01b058e7c9babe03d38d367c98d6a0dbb0e9785741a9c842d053574fa17076`
- Size: 15541742 bytes
- Container: Siril HEALPix catalogue format (zenodo record 14697486, CC BY 4.0), whole sky, HEALPix NESTED level 6, record type 128 (AstroBurst compact 6-byte), AstroBurst record kind 1, title `ASTROBURST-TYC2-V1`, epoch 2026.0 in the header.
- Source: Tycho-2 Catalogue (Hog E. et al. 2000, A&A 355, L27), CDS catalogue I/259, https://cdsarc.cds.unistra.fr/ftp/I/259/
- Licence: the sources give two statements. The CDS ReadMe of I/259 (https://cdsarc.cds.unistra.fr/ftp/I/259/ReadMe, last modified on 17 August 2026) states CC-BY-NC-3.0 IGO, as ESA states for the Hipparcos and Tycho Catalogues (https://www.cosmos.esa.int/web/hipparcos/catalogues, which asks for "Credit: ESA"); the Debian copyright file of the tycho2 package records BSD-2-Clause with an author's consent of 2015, https://tracker.debian.org/media/packages/t/tycho2/copyright-2-4. `licenses/tycho2-catalogue-NOTICE.txt` in the installed app reproduces both.
- Acknowledgement: built from data obtained from the CDS, Strasbourg, France.

## Inputs

| File | Bytes | Lines | SHA-256 |
|---|---|---|---|
| `tyc2.dat.00.gz` | 8407799 | 127000 | `1b224570fc6eb151984ce106fdf797728649d1ea77aaf3effd2d3444cfac6df6` |
| `tyc2.dat.01.gz` | 8375063 | 127000 | `36221f4a5cdd9c5009d64299d2e6409f8d4cdf924321768780165bd3e2a2a99a` |
| `tyc2.dat.02.gz` | 8351549 | 127000 | `e2ea4eeb7d204dde70ac54d172b1c429540c0d23d62ec0d7500561adc44cc57b` |
| `tyc2.dat.03.gz` | 8426693 | 127000 | `29e0cb57cba7e1651455efcb528276b0c28e1fe83f47f8ebe963e48d8b180afb` |
| `tyc2.dat.04.gz` | 8487663 | 127000 | `d04868329b470aa320f0fe5e9c525be619a7a228b364433e5f114f2178946d9b` |
| `tyc2.dat.05.gz` | 8247134 | 127000 | `51f08772197dcf0cd26ff098c866036340b797f71ee04be22712517c10c73c4e` |
| `tyc2.dat.06.gz` | 8039302 | 127000 | `f55503a66abce0e11d1a4ccbcde1f4c97a6e3e769de85e005f57ba963086ab03` |
| `tyc2.dat.07.gz` | 8129883 | 127000 | `bc15b5b1f6308477360fbb93f6a98d9029601cab68973202be0f8764c127f524` |
| `tyc2.dat.08.gz` | 8224049 | 127000 | `315ac34c678bc2e0f568b43b6b2d7c3d7c9cf9c089b51115d18b872864e50426` |
| `tyc2.dat.09.gz` | 8424804 | 127000 | `d66c671fa29aad10bc5fd697f68c918bb774c1ffc3c7db9c20234895268390d5` |
| `tyc2.dat.10.gz` | 8512497 | 127000 | `d86529df819ebdf3f4b8892510973ee10e1db06930814e014dd467193e740d52` |
| `tyc2.dat.11.gz` | 8497732 | 127000 | `fc0c3203b3d2787da54c43d9963e363c8a3926576c383b101a15350a7ca23e9c` |
| `tyc2.dat.12.gz` | 8350212 | 127000 | `68fd9d7e7353d52dea0de043cc62b45c04d234104eb05c063dc23ea7b6759576` |
| `tyc2.dat.13.gz` | 8360023 | 127000 | `a08ecc092e6d134742c0f594c56c5c2e02b8fd5e76cffb1779877f2fcefac3fc` |
| `tyc2.dat.14.gz` | 8472405 | 127000 | `2846e8cc489795a4d8160d2d5b64ff0e9a00598adba35a52a4f8ccf4c5e38b3e` |
| `tyc2.dat.15.gz` | 8455626 | 127000 | `74784a3656382090a70d778820335f4a781509dbe3fcb9e92d18cf96b2c46c72` |
| `tyc2.dat.16.gz` | 8416328 | 127000 | `320560e3d551cc40fa1f54ef7133709bf4bd45efa6688f808be7c09be5544a4d` |
| `tyc2.dat.17.gz` | 8171031 | 127000 | `a8154e940aa0a0e8f31d91b4bd9fd56e6849da42cddb13c7dda772439b06991a` |
| `tyc2.dat.18.gz` | 8077674 | 127000 | `5e951a1a51df7956f205b83cbfa5501d357c843640a253c1e6d3917bebe7d928` |
| `tyc2.dat.19.gz` | 8195964 | 126913 | `f59605a38116f517a31a7dbdee3469c077658f2f40b8afe5da2aeb832eaee3dd` |
| `suppl_1.dat.gz` | 671712 | 17588 | `d256a9fc47259d506e4849b054e9392a62b2ed128e48ac6a25a3a60fcc317f0e` |

## Processing

- tyc2.dat stars with a mean position (pflag blank or P): RAmdeg, DEmdeg (ICRS, epoch J2000.0) moved linearly with pmRA, pmDE to J2026.0: 2430468 stars.
- tyc2.dat stars without a mean position (pflag X): observed RAdeg, DEdeg, no proper motion: 109445 stars.
- suppl_1.dat stars: RAdeg, DEdeg (ICRS, epoch J1991.25) moved with pmRA, pmDE when given: 17588 stars.
- Magnitude: VTmag in 0.1 mag steps; 25 stars without VTmag carry the no-magnitude code 255. Colour: BTmag - VTmag in 0.02 mag steps when both exist.
- Clamped to the record range: 0 magnitudes, 21886 colours. Largest star offset from its pixel centre: 3393.7 arcsec.
- Records inside each pixel are ordered brightest first (no-magnitude records last), then by Dec and RA, so the build is reproducible.
- Builder: `cargo run --release --features tools --bin build_catalog -- tycho2` (AstroBurst `src-tauri/src/bin/build_catalog.rs`).
