# Fixture provenance

`sesame_positions.json` holds the positions of 551 deep-sky designations as returned by the CDS Sesame name resolver on 2026-10-23 (UTC time in `_meta.retrieved_utc`). It is the source of the offline object list in `src-tauri/src/core/astrometry/object_names.rs` and the reference of the test `every_offline_object_lies_within_1_arcmin_of_its_sesame_position` in `src-tauri/src/core/astrometry/solve_hints.rs`.

## What was queried

- Messier 1 to 110, queried as `M 1` to `M 110`.
- Caldwell 1 to 109. SIMBAD has no Caldwell identifiers, so each Caldwell number was queried under its usual NGC, IC or other designation (`query` field), for example `C 20` as `NGC 7000`, `C 9` as `Sh 2-155`, `C 41` as `Mel 25` (Hyades), `C 99` as `Coalsack`. `C 14` (Double Cluster) is `NGC 869`, `C 33` (East Veil) is `NGC 6992`, `C 49` (Rosette) is `NGC 2237`.
- 288 NGC and 44 IC objects chosen as commonly imaged targets (nebulae, planetary nebulae, galaxies, open and globular clusters). The choice is a list of identifiers only: no text, magnitude, size or description from any catalogue (OpenNGC included) was copied.

## How it was made

One run of 23 HTTP requests to `https://cds.unistra.fr/cgi-bin/nph-sesame/-oxI/SNV?` with 25 names per request (one request per second). Every name resolved; 550 through SIMBAD and one (`NGC 6979`) through NED. Per name the file keeps the resolver, the SIMBAD oid, the main identifier, the object type, `jradeg`/`jdedeg` (ICRS, J2000) and the `M`, `NGC` and `IC` identifiers among SIMBAD's aliases. The raw XML responses and their SHA-256 digests are kept outside the repository by the project owner.

`object_names.rs` is generated from this file: records with the same SIMBAD oid are one object (`M 102` and `NGC 5866`); each object carries the designations that were queried for it plus the `M`, `NGC` and `IC` aliases that no other record claims (for example `M 31` also answers to `NGC 224`); positions are rounded to 1e-6 degree. The generated table has 550 objects and 789 designations.

## Acknowledgement

This research has made use of the Sesame name resolver and of the SIMBAD database, operated at CDS, Strasbourg, France (Wenger et al. 2000, A&AS 143, 9). One position comes from the NASA/IPAC Extragalactic Database (NED), which is funded by the National Aeronautics and Space Administration and operated by the California Institute of Technology.
