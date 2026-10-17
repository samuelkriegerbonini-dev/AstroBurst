use super::errors::bad_input;
use super::ids::validate_dataset_id;
use super::types::{
    MastCmdError, MastFootprint, MastFootprintInput, MastFootprintNote, MastFootprintRegions,
    MastFootprintSet,
};
use crate::core::astrometry::stcs::{
    bounding_cone, parse_stcs, to_ds9_sky_region_text, StcsShape, CIRCLE_VERTICES,
    MAX_VERTICES_PER_SHAPE,
};
use crate::core::astrometry::wcs::WcsTransform;
use crate::types::header::HduHeader;

pub const MAX_FOOTPRINT_VERTICES: usize = 20_000;
pub const MOVING_NOTE: &str = "moving target: the archive footprint is not sidereal";
pub const MISSING_NOTE: &str = "MAST lists no footprint for this dataset";
pub const UNPROJECTED_NOTE: &str = "the footprint does not project onto this image";
const MOVING_TARGTYPE: &str = "MOVING";
const BOX_VERTICES: usize = 4;
const MAX_DEC_DEG: f64 = 90.0;
const PIXEL_EDGE: f64 = 0.5;

type Point = [f64; 2];

pub struct SkyFootprint {
    pub dataset_id: String,
    pub shapes: Result<Vec<StcsShape>, String>,
}

pub fn is_moving(targtype: Option<&str>) -> bool {
    targtype.is_some_and(|t| t.trim().eq_ignore_ascii_case(MOVING_TARGTYPE))
}

fn vertex_count(shape: &StcsShape) -> usize {
    match shape {
        StcsShape::Polygon(points) => points.len(),
        StcsShape::Circle { .. } => CIRCLE_VERTICES,
        StcsShape::Box { .. } => BOX_VERTICES,
    }
}

fn sky_polygon(points: &[Point]) -> Result<StcsShape, String> {
    if points.len() < 3 {
        return Err(format!(
            "a polygon needs at least 3 vertices, got {}",
            points.len()
        ));
    }
    if points.len() > MAX_VERTICES_PER_SHAPE {
        return Err(format!(
            "a polygon has {} vertices; at most {MAX_VERTICES_PER_SHAPE} are accepted",
            points.len()
        ));
    }
    let off_sky = |p: &&Point| !p[0].is_finite() || !p[1].is_finite() || p[1].abs() > MAX_DEC_DEG;
    if let Some(p) = points.iter().find(off_sky) {
        return Err(format!("vertex ({}, {}) is not a sky position", p[0], p[1]));
    }
    Ok(StcsShape::Polygon(
        points.iter().map(|p| (p[0], p[1])).collect(),
    ))
}

fn item_shapes(item: &MastFootprintInput) -> Result<Vec<StcsShape>, String> {
    if is_moving(item.targtype.as_deref()) {
        return Err(MOVING_NOTE.to_owned());
    }
    if let Some(polygons) = item.polygons.as_ref().filter(|p| !p.is_empty()) {
        return polygons.iter().map(|p| sky_polygon(p)).collect();
    }
    match item
        .s_region
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        Some(text) => parse_stcs(text).map_err(|e| format!("s_region: {e}")),
        None => Err(MISSING_NOTE.to_owned()),
    }
}

pub fn sky_footprints(items: &[MastFootprintInput]) -> Result<Vec<SkyFootprint>, MastCmdError> {
    let mut total = 0usize;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        validate_dataset_id(&item.dataset_id)?;
        let shapes = item_shapes(item);
        let vertices = shapes
            .as_ref()
            .map_or(0, |s| s.iter().map(vertex_count).sum());
        total = total.saturating_add(vertices);
        if total > MAX_FOOTPRINT_VERTICES {
            return Err(bad_input(
                "items",
                &format!(
                    "more than {MAX_FOOTPRINT_VERTICES} footprint vertices in one call; send fewer rows"
                ),
            ));
        }
        out.push(SkyFootprint {
            dataset_id: item.dataset_id.clone(),
            shapes,
        });
    }
    Ok(out)
}

pub fn project(
    path: &str,
    sky: Vec<SkyFootprint>,
    wcs: &WcsTransform,
    width: usize,
    height: usize,
) -> MastFootprintSet {
    let footprints = sky
        .into_iter()
        .map(|f| match f.shapes {
            Ok(shapes) => project_shapes(f.dataset_id, &shapes, wcs, width, height),
            Err(note) => noted(f.dataset_id, note),
        })
        .collect();
    MastFootprintSet {
        path: path.to_owned(),
        width,
        height,
        footprints,
    }
}

fn noted(dataset_id: String, note: String) -> MastFootprint {
    MastFootprint {
        dataset_id,
        polygons: Vec::new(),
        on_image: false,
        vertices: 0,
        cone: None,
        note: Some(note),
    }
}

fn project_shapes(
    dataset_id: String,
    shapes: &[StcsShape],
    wcs: &WcsTransform,
    width: usize,
    height: usize,
) -> MastFootprint {
    let mut polygons: Vec<Vec<Point>> = Vec::with_capacity(shapes.len());
    for shape in shapes {
        let pixels = wcs.world_to_pixel_batch(&shape.to_polygon(CIRCLE_VERTICES));
        if pixels.iter().all(|(x, y)| x.is_finite() && y.is_finite()) {
            polygons.push(pixels.into_iter().map(|(x, y)| [x, y]).collect());
        }
    }
    let dropped = shapes.len() - polygons.len();
    let note = match dropped {
        0 => None,
        _ if polygons.is_empty() => Some(UNPROJECTED_NOTE.to_owned()),
        _ => Some(format!(
            "{dropped} of {} polygons do not project onto this image",
            shapes.len()
        )),
    };
    MastFootprint {
        dataset_id,
        on_image: polygons.iter().any(|p| overlaps_image(p, width, height)),
        polygons,
        vertices: shapes.iter().map(vertex_count).sum(),
        cone: bounding_cone(shapes),
        note,
    }
}

fn edges(points: &[Point]) -> impl Iterator<Item = (Point, Point)> + '_ {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| (*a, *b))
}

fn contains_point(polygon: &[Point], p: Point) -> bool {
    let mut inside = false;
    for (a, b) in edges(polygon) {
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
            if p[0] < x {
                inside = !inside;
            }
        }
    }
    inside
}

fn turn(o: Point, a: Point, b: Point) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

fn opposite(u: f64, v: f64) -> bool {
    (u > 0.0 && v < 0.0) || (u < 0.0 && v > 0.0)
}

fn segments_cross(a: Point, b: Point, c: Point, d: Point) -> bool {
    opposite(turn(c, d, a), turn(c, d, b)) && opposite(turn(a, b, c), turn(a, b, d))
}

fn overlaps_image(polygon: &[Point], width: usize, height: usize) -> bool {
    if width == 0 || height == 0 || polygon.len() < 3 {
        return false;
    }
    let (x0, y0) = (-PIXEL_EDGE, -PIXEL_EDGE);
    let (x1, y1) = (width as f64 - PIXEL_EDGE, height as f64 - PIXEL_EDGE);
    let inside = |p: &Point| (x0..=x1).contains(&p[0]) && (y0..=y1).contains(&p[1]);
    if polygon.iter().any(inside) {
        return true;
    }
    let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    if corners.iter().any(|c| contains_point(polygon, *c)) {
        return true;
    }
    edges(polygon).any(|(a, b)| edges(&corners).any(|(c, d)| segments_cross(a, b, c, d)))
}

pub fn region_text(items: &[MastFootprintInput]) -> Result<MastFootprintRegions, MastCmdError> {
    let sky = sky_footprints(items)?;
    let mut groups: Vec<(&str, &[StcsShape])> = Vec::new();
    let mut notes = Vec::new();
    for footprint in &sky {
        match &footprint.shapes {
            Ok(shapes) => groups.push((footprint.dataset_id.as_str(), shapes.as_slice())),
            Err(note) => notes.push(MastFootprintNote {
                dataset_id: footprint.dataset_id.clone(),
                note: note.clone(),
            }),
        }
    }
    Ok(MastFootprintRegions {
        reg_text: to_ds9_sky_region_text(&groups),
        polygons: groups.iter().map(|(_, shapes)| shapes.len()).sum(),
        notes,
    })
}

pub fn image_size(header: &HduHeader) -> (usize, usize) {
    let compressed = header
        .get("ZIMAGE")
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("T"));
    let prefix = if compressed { "ZNAXIS" } else { "NAXIS" };
    let axis = |n: u8| {
        header
            .get_i64(&format!("{prefix}{n}"))
            .filter(|v| *v > 0)
            .and_then(|v| usize::try_from(v).ok())
            .unwrap_or(0)
    };
    (axis(1), axis(2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::astrometry::wcs::angular_separation;
    use crate::core::imaging::region::test_support::{make_header, rotated_cd, wcs_cards};
    use crate::core::imaging::region::PhysicalMap;
    use crate::core::imaging::region::RegionShape;
    use crate::core::imaging::region_file::parse_reg_with_physical;
    use crate::core::mast::types::{MastErrorCode, MastFootprintNote};

    const SQUARE_PX: [(f64, f64); 4] = [(10.0, 10.0), (80.0, 10.0), (80.0, 70.0), (10.0, 70.0)];

    fn tan() -> WcsTransform {
        let cards = wcs_cards(rotated_cd(30.0));
        let pairs: Vec<(&str, &str)> = cards
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        WcsTransform::from_header(&make_header(&pairs)).unwrap()
    }

    fn sky_of(wcs: &WcsTransform, pixels: &[(f64, f64)]) -> Vec<[f64; 2]> {
        pixels
            .iter()
            .map(|&(x, y)| {
                let c = wcs.pixel_to_world(x, y);
                [c.ra, c.dec]
            })
            .collect()
    }

    fn stcs_polygon(points: &[[f64; 2]]) -> String {
        let coords: Vec<String> = points
            .iter()
            .map(|p| format!("{} {}", p[0], p[1]))
            .collect();
        format!("POLYGON ICRS  {}", coords.join(" "))
    }

    fn item(id: &str, s_region: Option<String>) -> MastFootprintInput {
        MastFootprintInput {
            dataset_id: id.to_owned(),
            s_region,
            polygons: None,
            targtype: None,
        }
    }

    fn run(items: &[MastFootprintInput]) -> MastFootprintSet {
        project("a.fits", sky_footprints(items).unwrap(), &tan(), 100, 100)
    }

    fn assert_pixels(got: &[[f64; 2]], want: &[(f64, f64)], tolerance: f64) {
        assert_eq!(got.len(), want.len(), "{got:?}");
        for (g, w) in got.iter().zip(want) {
            assert!(
                (g[0] - w.0).abs() < tolerance && (g[1] - w.1).abs() < tolerance,
                "{g:?} vs {w:?}"
            );
        }
    }

    #[test]
    fn an_s_region_polygon_projects_back_onto_the_pixels_it_came_from() {
        let wcs = tan();
        let sky = sky_of(&wcs, &SQUARE_PX);
        let set = run(&[item("jw-square", Some(stcs_polygon(&sky)))]);
        assert_eq!(
            (set.path.as_str(), set.width, set.height),
            ("a.fits", 100, 100)
        );
        assert_eq!(set.footprints.len(), 1);
        let fp = &set.footprints[0];
        assert_eq!(fp.dataset_id, "jw-square");
        assert_eq!(fp.polygons.len(), 1);
        assert_pixels(&fp.polygons[0], &SQUARE_PX, 1e-6);
        assert!(fp.on_image);
        assert_eq!(fp.vertices, 4);
        assert_eq!(fp.note, None);
        let cone = fp.cone.expect("a bounding cone");
        for p in &sky {
            assert!(angular_separation(cone.ra, cone.dec, p[0], p[1]) <= cone.radius_deg + 1e-12);
        }
    }

    #[test]
    fn a_polygons_item_projects_exactly_like_its_s_region_twin() {
        let wcs = tan();
        let sky = sky_of(&wcs, &SQUARE_PX);
        let twin = MastFootprintInput {
            polygons: Some(vec![sky.clone()]),
            ..item("hst-peek", None)
        };
        let set = run(&[item("jw-square", Some(stcs_polygon(&sky))), twin]);
        assert_eq!(set.footprints[1].dataset_id, "hst-peek");
        assert_eq!(set.footprints[1].polygons, set.footprints[0].polygons);
        assert_eq!(set.footprints[1].vertices, 4);
        assert_eq!(set.footprints[1].cone, set.footprints[0].cone);

        let both = MastFootprintInput {
            s_region: Some("CIRCLE ICRS 150.0 2.0 0.001".into()),
            polygons: Some(vec![sky]),
            ..item("both", None)
        };
        assert_eq!(
            run(&[both]).footprints[0].vertices,
            4,
            "peek polygons win over s_region"
        );
    }

    #[test]
    fn circles_boxes_and_unions_become_one_polygon_per_shape() {
        let wcs = tan();
        let sky = sky_of(&wcs, &SQUARE_PX);
        let centre = wcs.pixel_to_world(49.5, 49.5);
        let union = format!(
            "UNION ICRS ( {} CIRCLE {} {} 0.004 ) BOX ICRS {} {} 0.01 0.005",
            stcs_polygon(&sky).replace("POLYGON ICRS  ", "POLYGON "),
            centre.ra,
            centre.dec,
            centre.ra,
            centre.dec
        );
        let set = run(&[item("mixed", Some(union))]);
        let fp = &set.footprints[0];
        assert_eq!(fp.note, None, "{:?}", fp.note);
        let lengths: Vec<usize> = fp.polygons.iter().map(Vec::len).collect();
        assert_eq!(lengths, [4, 64, 4]);
        assert_eq!(fp.vertices, 72);
        let radius_px = 0.004 * 3600.0;
        for p in &fp.polygons[1] {
            let r = (p[0] - 49.5).hypot(p[1] - 49.5);
            assert!((r - radius_px).abs() < 0.01, "{p:?} at {r} px");
        }
    }

    #[test]
    fn moving_missing_and_malformed_footprints_get_a_note_and_no_polygon() {
        let wcs = tan();
        let sky = stcs_polygon(&sky_of(&wcs, &SQUARE_PX));
        for (targtype, drawn) in [
            (Some("MOVING"), false),
            (Some(" moving "), false),
            (Some("FIXED"), true),
            (Some("UNKNOWN"), true),
            (Some("GENERIC"), true),
            (None, true),
        ] {
            assert_eq!(is_moving(targtype), !drawn, "{targtype:?}");
            let fp = &run(&[MastFootprintInput {
                targtype: targtype.map(str::to_owned),
                ..item("t", Some(sky.clone()))
            }])
            .footprints[0];
            assert_eq!(!fp.polygons.is_empty(), drawn, "{targtype:?}");
            if !drawn {
                assert_eq!(fp.note.as_deref(), Some(MOVING_NOTE));
                assert!(!fp.on_image);
                assert_eq!((fp.vertices, fp.cone), (0, None));
            }
        }

        let set = run(&[
            item("none", None),
            item("blank", Some("  ".into())),
            item("bad", Some("POLYGON ICRS 150 2 150.01 2 x 2.01".into())),
            item("galactic", Some("CIRCLE GALACTIC 10 20 0.1".into())),
            MastFootprintInput {
                polygons: Some(vec![vec![[150.0, 2.0], [150.01, 2.0]]]),
                ..item("short", None)
            },
            MastFootprintInput {
                polygons: Some(vec![vec![[150.0, 2.0], [150.01, 95.0], [150.0, 2.01]]]),
                ..item("pole", None)
            },
        ]);
        let notes: Vec<Option<&str>> = set.footprints.iter().map(|f| f.note.as_deref()).collect();
        assert_eq!(notes[0], Some(MISSING_NOTE));
        assert_eq!(notes[1], Some(MISSING_NOTE));
        assert_eq!(notes[2], Some("s_region: unexpected STC-S token 'x'"));
        assert!(notes[3].unwrap().contains("'GALACTIC'"), "{:?}", notes[3]);
        assert!(
            notes[4].unwrap().contains("at least 3 vertices"),
            "{:?}",
            notes[4]
        );
        assert!(notes[5].unwrap().contains("95"), "{:?}", notes[5]);
        assert!(set
            .footprints
            .iter()
            .all(|f| f.polygons.is_empty() && !f.on_image));
    }

    #[test]
    fn on_image_follows_the_overlap_with_the_pixel_grid() {
        let wcs = tan();
        let case = |pixels: &[(f64, f64)]| {
            let set = run(&[item("x", Some(stcs_polygon(&sky_of(&wcs, pixels))))]);
            assert_eq!(set.footprints[0].polygons.len(), 1);
            set.footprints[0].on_image
        };
        assert!(!case(&[(300.0, 300.0), (400.0, 300.0), (400.0, 400.0)]));
        assert!(case(&[
            (-50.0, -50.0),
            (150.0, -50.0),
            (150.0, 150.0),
            (-50.0, 150.0)
        ]));
        assert!(case(&[
            (-20.0, 40.0),
            (120.0, 40.0),
            (120.0, 45.0),
            (-20.0, 45.0)
        ]));
        assert!(case(&[(-0.4, 99.4), (-10.0, 120.0), (-20.0, 99.0)]));
        assert!(!case(&[(-0.6, 50.0), (-10.0, 60.0), (-20.0, 40.0)]));
    }

    #[test]
    fn a_polygon_on_the_far_side_of_the_sky_is_dropped_with_a_note() {
        let wcs = tan();
        let near = stcs_polygon(&sky_of(&wcs, &SQUARE_PX)).replace("POLYGON ICRS  ", "POLYGON ");
        let far = "POLYGON 330.0 -2.0 330.01 -2.0 330.01 -1.99";
        let set = run(&[
            item("half", Some(format!("UNION ICRS ( {near} {far} )"))),
            item("gone", Some(far.to_owned())),
        ]);
        let half = &set.footprints[0];
        assert_eq!(half.polygons.len(), 1);
        assert_pixels(&half.polygons[0], &SQUARE_PX, 1e-6);
        assert!(half.on_image);
        assert_eq!(half.vertices, 7);
        assert_eq!(
            half.note.as_deref(),
            Some("1 of 2 polygons do not project onto this image")
        );
        let gone = &set.footprints[1];
        assert!(gone.polygons.is_empty() && !gone.on_image);
        assert_eq!(gone.note.as_deref(), Some(UNPROJECTED_NOTE));
        assert!(gone.cone.is_some());
    }

    #[test]
    fn the_vertex_cap_counts_circles_and_refuses_before_any_projection() {
        let circles = |n: usize| -> Vec<MastFootprintInput> {
            (0..n)
                .map(|i| item(&format!("c{i}"), Some("CIRCLE ICRS 150 2 0.001".into())))
                .collect()
        };
        assert_eq!(MAX_FOOTPRINT_VERTICES, 20_000);
        assert_eq!(sky_footprints(&circles(312)).unwrap().len(), 312);
        let error = sky_footprints(&circles(313)).err().expect("over the cap");
        assert_eq!(error.code, MastErrorCode::BadInput);
        assert!(error.message.starts_with("items: "), "{}", error.message);
        assert!(error.message.contains("20000"), "{}", error.message);
        let mut moving = circles(400);
        for m in &mut moving[312..] {
            m.targtype = Some("MOVING".into());
        }
        assert_eq!(
            sky_footprints(&moving).unwrap().len(),
            400,
            "noted items carry no vertex"
        );

        let bad = sky_footprints(&[item("../x", None)]).err().expect("bad id");
        assert_eq!(bad.code, MastErrorCode::BadInput);
        assert!(bad.message.starts_with("datasetId: "), "{}", bad.message);
    }

    #[test]
    fn region_text_writes_icrs_polygons_per_dataset_and_notes_the_rest() {
        let wcs = tan();
        let sky = stcs_polygon(&sky_of(&wcs, &SQUARE_PX));
        let centre = wcs.pixel_to_world(30.0, 60.0);
        let items = [
            item("jw-square", Some(sky.clone())),
            MastFootprintInput {
                targtype: Some("MOVING".into()),
                ..item("comet", Some(sky))
            },
            item(
                "disc",
                Some(format!("CIRCLE ICRS {} {} 0.002", centre.ra, centre.dec)),
            ),
            item("none", None),
        ];
        let regions = region_text(&items).unwrap();
        assert_eq!(regions.polygons, 2);
        assert_eq!(
            regions.notes,
            [
                MastFootprintNote {
                    dataset_id: "comet".into(),
                    note: MOVING_NOTE.into()
                },
                MastFootprintNote {
                    dataset_id: "none".into(),
                    note: MISSING_NOTE.into()
                },
            ]
        );
        let lines: Vec<&str> = regions.reg_text.lines().collect();
        assert_eq!(lines[2], "icrs");
        assert!(lines[3].ends_with(" # text={jw-square}"), "{}", lines[3]);
        assert!(lines[4].ends_with(" # text={disc}"), "{}", lines[4]);
        assert_eq!(lines.len(), 5);

        let projected = project("a.fits", sky_footprints(&items).unwrap(), &wcs, 100, 100);
        let parsed =
            parse_reg_with_physical(&regions.reg_text, Some(&wcs), &PhysicalMap::identity())
                .unwrap();
        let drawn: Vec<&MastFootprint> = projected
            .footprints
            .iter()
            .filter(|f| !f.polygons.is_empty())
            .collect();
        assert_eq!(parsed.regions.len(), drawn.len());
        for (region, fp) in parsed.regions.iter().zip(drawn) {
            assert_eq!(region.props.text.as_deref(), Some(fp.dataset_id.as_str()));
            let RegionShape::Polygon { points } = &region.shape else {
                panic!("expected a polygon");
            };
            let want: Vec<(f64, f64)> = fp.polygons[0].iter().map(|p| (p[0], p[1])).collect();
            assert_pixels(points, &want, 1e-6);
        }
    }

    #[test]
    fn image_size_reads_the_decompressed_axes_of_a_tile_compressed_header() {
        let plain = make_header(&[("NAXIS1", "2048"), ("NAXIS2", "1024")]);
        assert_eq!(image_size(&plain), (2048, 1024));
        let tiled = make_header(&[
            ("ZIMAGE", "T"),
            ("NAXIS1", "8"),
            ("NAXIS2", "1024"),
            ("ZNAXIS1", "4096"),
            ("ZNAXIS2", "2048"),
        ]);
        assert_eq!(image_size(&tiled), (4096, 2048));
        assert_eq!(image_size(&make_header(&[("NAXIS1", "-3")])), (0, 0));
    }
}
