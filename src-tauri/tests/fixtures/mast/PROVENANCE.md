# Provenance of the MAST test fixtures

Every file in this folder is public NASA archive metadata from the Mikulski Archive for Space Telescopes (MAST) at the Space Telescope Science Institute (STScI), or a short head of a public preview image MAST serves. The answers were captured anonymously with read-only requests; no credential or token was used, and no file carries a personal value. The one exception is the synthetic Roman line of `s_region.txt`, stated in its row.

Capture: the live probe of 2026-10-16 (HTTP `Date` headers from 07:05 to 07:22 UTC) unless the row gives another time. Rows marked "also 2026-10-13" were captured byte for byte the same by the study of that day. Search requests are `POST https://mast.stsci.edu/search/<mission>/api/v0.1/search` with the JSON body that the answer echoes in `search_params`. JPEG heads (`*_sos.bin`) are the first bytes of the image up to the end of the start-of-scan (SOS) segment header, before any entropy-coded data; the parser tests need only the headers and the fitscut COM card with its WCS.

Acknowledgement: data and metadata from the Mikulski Archive for Space Telescopes (MAST) at STScI.

| File | Bytes | Request | Captured |
|---|---|---|---|
| `a.json` | 943 | `POST https://mast.stsci.edu/search/hst/api/v0.1/search`, cone 274.688 -13.792, 10', limit 1, no condition (total 533) | 2026-10-16 |
| `b.json` | 978 | same HST search with `sci_start_time` `>=2014-01-01` (total 256), `https://mast.stsci.edu/search/hst/api/v0.1/search` | 2026-10-16 |
| `c.json` | 977 | same HST search with `sci_start_time` `<=2014-12-31` (total 362), `https://mast.stsci.edu/search/hst/api/v0.1/search` | 2026-10-16 |
| `d.json` | 985 | same HST search with `sci_start_time` `>=2014-01-01,<=2014-12-31` (total 85), `https://mast.stsci.edu/search/hst/api/v0.1/search` | 2026-10-16 |
| `c_nowild.json` | 2061 | `POST https://mast.stsci.edu/search/jwst/api/v0.1/search`, target M16, 10', `count_only`, `exp_type` `NRC_IMAGE` (total 61) | 2026-10-16 |
| `c_wildcard.json` | 29432 | `POST https://mast.stsci.edu/search/jwst/api/v0.1/search`, target M16, 10', limit 200, `exp_type` `NRC_*` (61 rows, total 237) | 2026-10-16 |
| `classy_il.json` | 22 | `GET https://mast.stsci.edu/search/classy/api/v0.1/get_image_list` (FastAPI 404 `{"detail":"Not Found"}`; IUE and ULLYSES answer the same bytes) | 2026-10-16 |
| `column_list_hst.json` | 19442 | `GET https://mast.stsci.edu/search/util/api/v0.1/column_list?mission=hst` (62 columns; `sci_broad_category` qual_type `string`, `sci_target_descrip` qual_type `substring`) | 2026-10-23 17:36 UTC |
| `column_list_iue.json` | 24023 | `GET https://mast.stsci.edu/search/util/api/v0.1/column_list?mission=iue` (82 columns; `iue_category` qual_type `integer`) | 2026-10-23 17:36 UTC |
| `column_list_jwst.json` | 101303 | `GET https://mast.stsci.edu/search/util/api/v0.1/column_list?mission=jwst` (257 columns; `targcat`, `targdesc`, `productLevel` queryable) | 2026-10-23 17:36 UTC |
| `hpL_hst_15238_20_wfc3_ir_total_idnm20_drz_color_sos.bin` | 945 | head of `GET https://mast.stsci.edu/search/hst/api/v0.1/get_image_preview?fileset_name=IDNM20010&filename=hst_15238_20_wfc3_ir_total_idnm20_drz_color.jpg`, served after a 307 by `https://stpubdata.s3.us-east-1.amazonaws.com/hst/public/idnm/hst_15238_20/hst_15238_20_wfc3_ir_total_idnm20_drz_color.jpg` (489,647 bytes in all) | 2026-10-16 07:11 UTC |
| `hst_il_IDNM20010_PREVIEW.json` | 5353 | `GET https://mast.stsci.edu/search/hst/api/v0.1/get_image_list?fileset_name=IDNM20010&category=PREVIEW` | 2026-10-16 |
| `hst_il_IDNM20010_THUMBNAIL.json` | 5673 | `GET https://mast.stsci.edu/search/hst/api/v0.1/get_image_list?fileset_name=IDNM20010&category=THUMBNAIL` | 2026-10-16 |
| `hst_m16.json` | 5604 | `POST https://mast.stsci.edu/search/hst/api/v0.1/search`, target M16, 3', limit 5 (total 265) | 2026-10-16 |
| `hst_m16_ism.json` | 19473 | `POST https://mast.stsci.edu/search/hst/api/v0.1/search`, target M16, 3', limit 200, the 14 `select_cols` AstroBurst 0.7.1 sends (`sci_broad_category`, `sci_target_descrip` added), `sci_broad_category` `ISM, ISM;*, *;ISM, *;ISM;*, EXT-MEDIUM, EXT-MEDIUM;*, *;EXT-MEDIUM, *;EXT-MEDIUM;*` (37 rows, total 37) | 2026-10-23 17:36 UTC |
| `hst_search_wfc3.json` | 3139 | `POST https://mast.stsci.edu/search/hst/api/v0.1/search`, target M16, 3', limit 6, `sci_instrume` `WFC3` (total 29) | 2026-10-16 |
| `il_ICK909030_PREVIEW.json` | 4658 | `GET https://mast.stsci.edu/search/hst/api/v0.1/get_image_list?fileset_name=ICK909030&category=PREVIEW` (also 2026-10-13) | 2026-10-16 |
| `il_ICK909030_THUMBNAIL.json` | 4930 | `GET https://mast.stsci.edu/search/hst/api/v0.1/get_image_list?fileset_name=ICK909030&category=THUMBNAIL` | 2026-10-16 |
| `il_jw02739-o001_t001_nircam_PREVIEW.json` | 1771 | `GET https://mast.stsci.edu/search/jwst/api/v0.1/get_image_list?fileset_name=jw02739-o001_t001_nircam&category=PREVIEW` | 2026-10-16 |
| `il_jw02739-o001_t001_nircam_THUMBNAIL.json` | 1867 | `GET https://mast.stsci.edu/search/jwst/api/v0.1/get_image_list?fileset_name=jw02739-o001_t001_nircam&category=THUMBNAIL` (also 2026-10-13) | 2026-10-16 |
| `il_jw02739001002_02101_00001_PREVIEW.json` | 17419 | `GET https://mast.stsci.edu/search/jwst/api/v0.1/get_image_list?fileset_name=jw02739001002_02101_00001&category=PREVIEW` | 2026-10-16 |
| `il_jw02739001002_02101_00001_THUMBNAIL.json` | 18379 | `GET https://mast.stsci.edu/search/jwst/api/v0.1/get_image_list?fileset_name=jw02739001002_02101_00001&category=THUMBNAIL` | 2026-10-16 |
| `iue_m16_stars.json` | 10691 | `POST https://mast.stsci.edu/search/iue/api/v0.1/search`, target M16, 3', limit 200, `iue_category` `10..66` (24 rows; the echo adds `!=98,!=99`) | 2026-10-23 17:36 UTC |
| `jwst_m16_ism.json` | 97332 | `POST https://mast.stsci.edu/search/jwst/api/v0.1/search`, target M16, 3', limit 2000, the 19 `select_cols` AstroBurst 0.7.1 sends (the 17 of `jwst_m16_l3.json` plus `targcat`, `targdesc`), `targcat` `ISM` (113 rows: 2 at `productLevel` `3`, 111 at `1b, 2a, 2b, 2c`) | 2026-10-23 17:36 UTC |
| `jwst_m16_l3.json` | 5173 | `POST https://mast.stsci.edu/search/jwst/api/v0.1/search`, target M16, 3', limit 20, `productLevel` `3` (2 rows) | 2026-10-16 |
| `jwst_m16_l3_optical.json` | 5317 | `POST https://mast.stsci.edu/search/jwst/api/v0.1/search`, target M16, 3', limit 12, `productLevel` `3`, sorted by `date_obs` descending, `opticalElements` selected (2 rows) | 2026-10-16 |
| `jwst_p2739.json` | 16359 | `POST https://mast.stsci.edu/search/jwst/api/v0.1/search`, `program` `2739`, `productLevel` `3`, limit 10 (total 14) | 2026-10-16 |
| `lookup_m16.json` | 359 | `POST https://mast.stsci.edu/api/v0/invoke`, form field `request` = `{"service":"Mast.Name.Lookup","params":{"input":"M16","format":"json"},"format":"json"}` | 2026-10-16 08:31 UTC |
| `m_tess.json` | 153 | `POST https://mast.stsci.edu/search/tess/api/v0.1/search` (nginx HTML 404; `multi_mission` answers the same bytes) | 2026-10-16 |
| `nl.json` | 37 | `POST https://mast.stsci.edu/api/v0/invoke`, `Mast.Name.Lookup` of a name MAST cannot resolve (`resolvedCoordinate` empty) | 2026-10-16 |
| `p_jw02739001002_02105_00003_nrca1_cal_sos.bin` | 644 | head of `GET https://mast.stsci.edu/search/jwst/api/v0.1/get_image_preview` for `jw02739001002_02105_00003_nrca1_cal.jpg` (507,322 bytes in all) | 2026-10-16 07:08 UTC |
| `prev_hst_head_sos.bin` | 641 | head of `GET https://mast.stsci.edu/search/hst/api/v0.1/get_image_preview?fileset_name=ICK909030&filename=ick909030_drc.jpg`, served after a 307 by `https://stpubdata.s3.us-east-1.amazonaws.com/hst/public/ick9/ick909030/ick909030_drc.jpg` (3,128,075 bytes in all) | 2026-10-16 07:10 UTC |
| `prev_jw_head_sos.bin` | 611 | head of `GET https://mast.stsci.edu/search/jwst/api/v0.1/get_image_preview?fileset_name=jw02739-o001_t001_nircam&filename=jw02739-o001_t001_nircam_clear-f444w_i2d.jpg` (2,582,508 bytes in all) | 2026-10-16 07:08 UTC |
| `prod_hst.json` | 44062 | `POST https://mast.stsci.edu/search/hst/api/v0.1/post_list_products`, body `{"dataset_ids":["ICK909030"]}` (94 products) | 2026-10-16 |
| `prod_iue.json` | 2712 | `POST https://mast.stsci.edu/search/iue/api/v0.1/post_list_products`, body `{"dataset_ids":["LWP00501"]}` (6 products) | 2026-10-16 14:05 UTC |
| `prod_jw.json` | 64755 | `POST https://mast.stsci.edu/search/jwst/api/v0.1/post_list_products`, body `{"dataset_ids":["jw02739-o001_t001_nircam"]}` (108 products) | 2026-10-16 |
| `radius60.json` | 63 | `POST https://mast.stsci.edu/search/jwst/api/v0.1/search` with a 60' radius (HTTP 400) | 2026-10-16 |
| `roman_auth.json` | 20 | `GET https://mast.stsci.edu/search/roman/api/v0.1/auth_check` without a token (also 2026-10-13) | 2026-10-16 |
| `roman_search.json` | 60 | `POST https://mast.stsci.edu/search/roman/api/v0.1/search` without a token (HTTP 401; also 2026-10-13) | 2026-10-16 07:11 UTC |
| `s_region.txt` | 3754 | derived from `jwst_m16_l3.json` (lines 1 and 2) and `jwst_p2739.json` (line 3), each `s_region` copied verbatim with its dataset and vertex count; line 4 `roman_form/romancal_compute_s_region_keyword` is synthetic: a four-corner `POLYGON ICRS` in the form the romancal pipeline writes, with coordinates that are not a MAST record, because Roman searches answer 401 without a token (`roman_search.json`); replace it with a live Roman `s_region` after a token run | 2026-10-16 |
| `t_bad.bin` | 28 | `GET https://mast.stsci.edu/search/jwst/api/v0.1/get_image_thumbnail` for a file MAST does not hold (JSON `{"detail":"Image not found"}`, not an image) | 2026-10-16 |
| `t_cal_1.bin` | 4692 | `GET https://mast.stsci.edu/search/jwst/api/v0.1/get_image_thumbnail` for `jw02739001002_02105_00003_nrca1_cal_thumb.jpg` | 2026-10-16 |
| `t_clear-f187n.bin` | 3507 | `GET https://mast.stsci.edu/search/jwst/api/v0.1/get_image_thumbnail?fileset_name=jw02739-o001_t001_nircam&filename=jw02739-o001_t001_nircam_clear-f187n_i2d_thumb.jpg` (the fileset-only request answers the same bytes) | 2026-10-16 07:08 UTC |
| `thumb_hst.bin` | 4161 | `GET https://mast.stsci.edu/search/hst/api/v0.1/get_image_thumbnail?fileset_name=ICK909030&filename=ick909030_drc_thumb.jpg`, served after a 307 by `https://stpubdata.s3.us-east-1.amazonaws.com/hst/public/ick9/ick909030/ick909030_drc_thumb.jpg` | 2026-10-16 07:10 UTC |
