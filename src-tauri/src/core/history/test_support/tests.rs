use super::fixtures::*;
use crate::core::history::*;
use std::time::{Duration, Instant};
use std::fs;
use std::time::SystemTime;

use ndarray::Array2;
use serde_json::Value;

use crate::core::ops::OutputValues;

use crate::core::history::id::{canonical_ref, image_id};
use crate::core::history::stamp::utc_timestamp;

const GOLDEN_V1: &str = include_str!("../golden/history-v1.json");

fn unix(text: &str) -> String {
    text.replace("\r\n", "\n")
}

#[test]
fn golden_history_v1_round_trips() {
    let golden = unix(GOLDEN_V1);
    let doc: HistoryDoc = serde_json::from_str(&golden).unwrap();
    assert_eq!(doc.format, HISTORY_FORMAT);
    assert_eq!(doc.version, HISTORY_VERSION);
    assert_eq!(doc.read_only_reason(), None);
    assert_eq!(doc.nodes.len(), 5);
    assert_eq!(doc.head.as_deref(), Some("n0004"));
    assert_eq!(doc.nodes[3].mask, Some(MaskRef::Id("m01".to_string())));
    assert!(matches!(doc.nodes[4].mask, Some(MaskRef::Recipe(_))));
    assert_eq!(doc.nodes[0].output.extras, ["n0001_model.png"]);
    assert!(!doc.nodes[1].present);
    assert_eq!(serde_json::to_string_pretty(&doc).unwrap(), golden, "the v1 sidecar did not re-serialise byte-identically");
}

#[test]
fn a_node_mask_resolves_through_the_masks_map_or_inline() {
    let mut doc = new_doc("/data/m42.fits");
    doc.masks.insert("m02".to_string(), serde_json::json!({"kind": "range", "lo": 0.1, "hi": 0.9, "invert": true}));
    let stored = doc.mask_spec(&MaskRef::Id("m02".to_string())).unwrap();
    assert_eq!(stored.recipe.kind(), "range");
    assert!(stored.mods.invert);
    let inline = doc.mask_spec(&MaskRef::Recipe(serde_json::json!({"kind": "luminance", "feather_px": 2}))).unwrap();
    assert_eq!((inline.recipe.kind(), inline.mods.feather_px), ("luminance", 2.0));

    let missing = doc.mask_spec(&MaskRef::Id("m09".to_string())).unwrap_err().to_string();
    assert_eq!(missing, "mask m09 is not in this image's history");
    let unknown_kind = doc.mask_spec(&MaskRef::Recipe(serde_json::json!({"kind": "blur"}))).unwrap_err().to_string();
    assert!(unknown_kind.starts_with("inline mask: unknown variant `blur`"), "{unknown_kind}");

    let golden: HistoryDoc = serde_json::from_str(&unix(GOLDEN_V1)).unwrap();
    let inline_on_n0005 = golden.nodes[4].mask.as_ref().unwrap();
    assert_eq!(golden.mask_spec(inline_on_n0005).unwrap().recipe.kind(), "luminance");
    let m01 = golden.mask_spec(golden.nodes[3].mask.as_ref().unwrap()).unwrap_err().to_string();
    assert!(m01.starts_with("mask m01: unknown field `growth`"), "{m01}");
}

#[test]
fn a_node_mask_is_rasterised_from_the_op_input_and_refused_at_another_size() {
    use crate::core::mask::rasterize;
    use crate::infra::fits::writer::write_fits_mono;

    let dir = tempfile::tempdir().unwrap();
    let small = dir.path().join("mask8.fits").to_string_lossy().replace('\\', "/");
    write_fits_mono(&small, &Array2::from_elem((8, 8), 1.0f32), None).unwrap();
    let mut doc = new_doc("/data/m42.fits");
    doc.masks.insert("m03".to_string(), serde_json::json!({"kind": "file", "path": small}));
    let plane = Array2::from_shape_fn((12, 16), |(y, x)| (y * 16 + x) as f32);

    let stored = MaskRef::Id("m03".to_string());
    assert_eq!(stored.label(), "m03");
    assert_eq!(doc.mask_raster(&stored, &[&plane]).unwrap_err().to_string(), "mask m03 is 8×8, image is 16×12");
    let inline_file = MaskRef::Recipe(serde_json::json!({"kind": "file", "path": small}));
    assert_eq!(inline_file.label(), "file");
    assert_eq!(doc.mask_raster(&inline_file, &[&plane]).unwrap_err().to_string(), "mask file is 8×8, image is 16×12");
    assert_eq!(MaskRef::Recipe(serde_json::json!({"feather_px": 1})).label(), "inline");

    let inline = MaskRef::Recipe(serde_json::json!({"kind": "range", "lo": 40.0, "hi": 120.0, "fuzz": 8.0, "invert": true}));
    let spec = doc.mask_spec(&inline).unwrap();
    let expected = rasterize(&spec.recipe, &spec.mods, &[&plane]).unwrap();
    assert_eq!(doc.mask_raster(&inline, &[&plane]).unwrap(), expected);
    assert!(expected.iter().any(|&m| m > 0.0 && m < 1.0), "precondition: the range mask has a soft edge");
}

#[test]
fn append_then_set_head_survive_a_crash_between_tmp_write_and_rename() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = new_doc("/data/m42.fits");
    let ids = chain(dir.path(), &mut doc, None, &["background", "wavelet_denoise"], 64);
    set_head(dir.path(), &mut doc, Some(&ids[0])).unwrap();

    let sidecar = sidecar_path(dir.path());
    let before = fs::read(&sidecar).unwrap();
    let temp = temp_path(&sidecar);
    fs::write(&temp, &before[..before.len() / 2]).unwrap();
    age_file(&temp, STALE_TEMP_AGE.as_secs() + 1);

    let loaded = load(dir.path()).unwrap().expect("the sidecar vanished after a crash");
    assert_eq!(fs::read(&sidecar).unwrap(), before, "the old sidecar was touched by the interrupted write");
    assert_eq!(loaded.nodes.len(), 2);
    assert_eq!(loaded.head.as_deref(), Some(ids[0].as_str()));
    assert!(!temp.exists(), "a stale temp sidecar was left behind");
    assert!(load(tempfile::tempdir().unwrap().path()).unwrap().is_none());
}

#[test]
fn a_version_2_sidecar_opens_read_only_with_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    let bumped = unix(GOLDEN_V1).replace("\"version\": 1", "\"version\": 2");
    fs::write(sidecar_path(dir.path()), &bumped).unwrap();

    let Err(HistoryError::ReadOnly(reason)) = load(dir.path()) else {
        panic!("a v2 sidecar was treated as writable");
    };
    assert!(reason.contains("version 2") && reason.contains("read-only"), "{reason}");

    let mut doc: HistoryDoc = serde_json::from_str(&bumped).unwrap();
    assert_eq!(doc.read_only_reason().as_deref(), Some(reason.as_str()));
    let err = set_head(dir.path(), &mut doc, Some("n0001")).unwrap_err();
    assert!(matches!(err, HistoryError::ReadOnly(_)), "{err}");
    let node = NewNode {
        id: "n0009".to_string(),
        parent: None,
        op: op("arcsinh"),
        mask: None,
        output: NodeOutput::named("n0009", [4, 4], 1, OutputValues::DisplayReferred, &[]),
        elapsed_ms: 1,
        replayable: true,
    };
    assert!(matches!(append(dir.path(), &mut doc, node), Err(HistoryError::ReadOnly(_))));
    assert_eq!(fs::read_to_string(sidecar_path(dir.path())).unwrap(), bumped, "a read-only sidecar was rewritten");

    fs::write(sidecar_path(dir.path()), b"{\"format\": \"something-else\"}").unwrap();
    let err = load(dir.path()).unwrap_err();
    assert!(matches!(err, HistoryError::Unreadable { .. }), "{err}");
}

#[test]
fn a_sidecar_from_a_newer_build_opens_read_only_even_when_its_ops_do_not_parse() {
    let dir = tempfile::tempdir().unwrap();
    let mut newer: Value = serde_json::from_str(&unix(GOLDEN_V1)).unwrap();
    newer["version"] = 2.into();
    newer["nodes"][0]["op"]["v"] = 2.into();
    newer["nodes"][1]["op"]["op"] = "star_removal".into();
    let bytes = serde_json::to_vec_pretty(&newer).unwrap();
    fs::write(sidecar_path(dir.path()), &bytes).unwrap();

    match load(dir.path()) {
        Err(HistoryError::ReadOnly(reason)) => {
            assert!(reason.contains("version 2") && reason.contains("read-only"), "{reason}")
        }
        other => panic!("a version 2 sidecar did not open read-only: {other:?}"),
    }
    assert_eq!(fs::read(sidecar_path(dir.path())).unwrap(), bytes);

    fs::write(sidecar_path(dir.path()), &bytes[..bytes.len() / 2]).unwrap();
    assert!(matches!(load(dir.path()), Err(HistoryError::Unreadable { .. })), "a truncated sidecar was not reported unreadable");
}

#[test]
fn a_sidecar_with_an_operation_or_spec_version_this_build_does_not_know_opens_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let mut unknown_op: Value = serde_json::from_str(&unix(GOLDEN_V1)).unwrap();
    unknown_op["nodes"][1]["op"]["op"] = "curves_v9".into();
    let mut unknown_spec: Value = serde_json::from_str(&unix(GOLDEN_V1)).unwrap();
    unknown_spec["nodes"][0]["op"]["v"] = 2.into();
    for (case, expected) in [(unknown_op, "curves_v9"), (unknown_spec, "version 2")] {
        let bytes = serde_json::to_vec_pretty(&case).unwrap();
        fs::write(sidecar_path(dir.path()), &bytes).unwrap();
        match load(dir.path()) {
            Err(HistoryError::ReadOnly(reason)) => {
                assert!(reason.contains(expected) && reason.contains("read-only"), "{reason}")
            }
            other => panic!("a history with a step this build cannot run did not open read-only: {other:?}"),
        }
        assert_eq!(fs::read(sidecar_path(dir.path())).unwrap(), bytes, "a read-only sidecar was rewritten");
    }

    let mut broken: Value = serde_json::from_str(&unix(GOLDEN_V1)).unwrap();
    broken["nodes"][0]["output"] = Value::Null;
    fs::write(sidecar_path(dir.path()), serde_json::to_vec(&broken).unwrap()).unwrap();
    assert!(matches!(load(dir.path()), Err(HistoryError::Unreadable { .. })), "a damaged sidecar was opened read-only");
}

#[test]
fn mark_missing_and_save_leave_a_read_only_sidecar_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let bumped = unix(GOLDEN_V1).replace("\"version\": 1", "\"version\": 2");
    fs::write(sidecar_path(dir.path()), &bumped).unwrap();
    let mut doc: HistoryDoc = serde_json::from_str(&bumped).unwrap();
    let present_before = doc.nodes.iter().filter(|n| n.present).count();

    assert!(mark_missing(dir.path(), &mut doc).is_empty(), "eviction marks were recorded in a read-only history");
    assert_eq!(doc.nodes.iter().filter(|n| n.present).count(), present_before);
    assert!(matches!(save(dir.path(), &doc), Err(HistoryError::ReadOnly(_))), "a read-only history was saved");
    assert_eq!(fs::read_to_string(sidecar_path(dir.path())).unwrap(), bumped, "a read-only sidecar was rewritten");
}

#[test]
fn concurrent_writers_never_share_a_temp_file_and_load_spares_one_in_flight() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = new_doc("/data/m42.fits");
    chain(dir.path(), &mut doc, None, &["background"], 64);
    let sidecar = sidecar_path(dir.path());
    let first = temp_path(&sidecar);
    let second = temp_path(&sidecar);
    assert_ne!(first, second, "two writers would rename each other's half-written sidecar");

    fs::write(&first, b"{\"half\":").unwrap();
    load(dir.path()).unwrap().unwrap();
    assert!(first.exists(), "load deleted the temp file of a write still in flight");

    write_bytes(&second, 8, 3600);
    load(dir.path()).unwrap().unwrap();
    assert!(!second.exists(), "an hour-old interrupted write was left behind");
    assert!(first.exists());
}

#[test]
fn a_head_at_the_root_still_pins_the_redo_target() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = three_branches(dir.path(), 1024);
    set_head(dir.path(), &mut doc, None).unwrap();
    assert_eq!(ids_of(&pinned_paths(&doc, PinLevel::Full)), ["n0001"], "Revert to original left the redo target unpinned");
    assert_eq!(ids_of(&pinned_paths(&doc, PinLevel::HeadAndParent)), ["n0001"]);
    assert_eq!(doc.redo_target().map(|n| n.id.as_str()), Some("n0001"));
    assert_eq!(ids_of(&active_paths(&doc)), ["n0001"]);

    set_head(dir.path(), &mut doc, Some("n0001")).unwrap();
    assert_eq!(doc.redo_target().map(|n| n.id.as_str()), Some("n0010"), "redo is not the most recent child");
    assert_eq!(ids_of(&active_paths(&doc)), ["n0001", "n0010"]);
}

#[test]
fn a_rebased_history_never_releases_its_first_node() {
    let dir = tempfile::tempdir().unwrap();
    let key = "/data/m42.fits#hdu=0";
    let mut doc = rebased_doc(key, Some("/app/output/m42_calibrated.fits"));
    assert!(doc.is_rebased());
    chain(dir.path(), &mut doc, None, &STAGES, 1 << 20);
    let shrunk = pin_levels(&[(&doc, SystemTime::now())], 0);
    assert_eq!(ids_of(&pinned_paths(&doc, shrunk[0])), ["n0001", "n0004", "n0005"], "the only bridge from a wiped chain base was released");
    assert_eq!(ids_of(&pinned_paths(&doc, PinLevel::Floor)), ["n0001"]);

    let plain_dir = tempfile::tempdir().unwrap();
    let mut plain = new_doc(key);
    assert!(!plain.is_rebased());
    chain(plain_dir.path(), &mut plain, None, &STAGES, 1 << 20);
    assert_eq!(ids_of(&pinned_paths(&plain, shrunk[0])), ["n0004", "n0005"]);
    assert!(pinned_paths(&plain, PinLevel::Floor).is_empty());
}

#[test]
fn node_paths_are_unique_and_never_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = new_doc("/data/m42.fits");
    let a = record(dir.path(), &mut doc, None, "arcsinh", 64);
    let b = record(dir.path(), &mut doc, None, "arcsinh", 64);
    assert_eq!((a.as_str(), b.as_str()), ("n0001", "n0002"));
    assert_ne!(doc.nodes[0].output.path, doc.nodes[1].output.path);

    write_bytes(&dir.path().join("n0003.fits"), 8, 1);
    assert_eq!(next_node_id(dir.path(), &doc), "n0004", "the allocator reused a file already on disk");
    write_bytes(&dir.path().join("n0004.png"), 8, 1);
    assert_eq!(next_node_id(dir.path(), &doc), "n0005");

    let duplicate = NewNode {
        id: a.clone(),
        parent: None,
        op: op("arcsinh"),
        mask: None,
        output: NodeOutput::named(&a, [64, 32], 1, OutputValues::DisplayReferred, &[]),
        elapsed_ms: 1,
        replayable: true,
    };
    assert!(matches!(append(dir.path(), &mut doc, duplicate), Err(HistoryError::NodeExists(_))));
    assert_eq!(doc.nodes.len(), 2);

    let orphan = NewNode {
        id: "n0010".to_string(),
        parent: Some("n0042".to_string()),
        op: op("arcsinh"),
        mask: None,
        output: NodeOutput::named("n0010", [64, 32], 1, OutputValues::DisplayReferred, &[]),
        elapsed_ms: 1,
        replayable: true,
    };
    assert!(matches!(append(dir.path(), &mut doc, orphan), Err(HistoryError::UnknownNode(_))));
    assert_eq!(load(dir.path()).unwrap().unwrap().nodes.len(), 2);
}

#[test]
fn node_ids_are_never_handed_out_twice_after_a_branch_is_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = new_doc("/data/nrca1_i2d.fits#hdu=1");
    chain(dir.path(), &mut doc, None, &["background", "wavelet_denoise", "arcsinh", "tone"], 64);
    for gone in remove_subtree(dir.path(), &mut doc, "n0003").unwrap() {
        for file in gone.output.files() {
            fs::remove_file(dir.path().join(file)).unwrap();
        }
    }
    assert_eq!(next_node_id(dir.path(), &doc), "n0005", "the id of a deleted step was handed out again");
    let reloaded = load(dir.path()).unwrap().unwrap();
    assert_eq!(next_node_id(dir.path(), &reloaded), "n0005", "the issued ids were lost on reload");

    let golden: HistoryDoc = serde_json::from_str(&unix(GOLDEN_V1)).unwrap();
    assert_eq!(next_node_id(tempfile::tempdir().unwrap().path(), &golden), "n0006");
}

#[test]
fn node_output_names_follow_the_node_id() {
    let out = NodeOutput::named("n0007", [2058, 2058], 1, OutputValues::Linear, &["model"]);
    assert_eq!((out.path.as_str(), out.png.as_str()), ("n0007.fits", "n0007.png"));
    assert_eq!(out.extras, ["n0007_model.png"]);
    assert_eq!(out.files().collect::<Vec<_>>(), ["n0007.fits", "n0007.png", "n0007_model.png"]);
}

#[test]
fn node_paths_have_no_dot_prefixed_component() {
    let app_data = tempfile::tempdir().unwrap();
    let root = history_root(app_data.path());
    assert_eq!(root, app_data.path().join("history"));
    let dir = image_dir(&root, &image_id("/data/m42.fits#hdu=0", None));
    let mut doc = new_doc("/data/m42.fits#hdu=0");
    let id = record(&dir, &mut doc, None, "background", 64);
    let model = NodeOutput::named(&id, [4, 4], 1, OutputValues::Linear, &["model"]);
    let hash = kernels::write_kernel(&dir, &ndarray::Array2::<f32>::ones((3, 3))).unwrap();

    let mut produced = vec![root.clone(), dir.clone(), sidecar_path(&dir), kernels_dir(&dir), masks_dir(&dir)];
    produced.push(kernels::kernel_path(&dir, &hash).unwrap());
    produced.extend(model.files().map(|f| dir.join(f)));
    produced.extend(doc.nodes.iter().flat_map(|n| n.output.files().map(|f| dir.join(f)).collect::<Vec<_>>()));
    produced.extend(protected_paths(&doc, &dir, PinLevel::Full));
    produced.push(temp_path(&sidecar_path(&dir)));

    for path in &produced {
        let below = path.strip_prefix(app_data.path()).unwrap();
        for part in below.components() {
            let name = part.as_os_str().to_string_lossy();
            assert!(!name.starts_with('.'), "{} has the dot-prefixed component {name}", path.display());
        }
    }
}

#[test]
fn fingerprint_matches_until_the_source_is_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("m42.fits");
    write_bytes(&source, 2880, 60);
    let key = format!("{}#hdu=0", source.display());
    let info = ImageInfo::of(&key, None).unwrap();
    assert_eq!(info.size, 2880);
    assert_eq!(info.id, image_id(&key, None));
    assert_eq!(info.source_ref, canonical_ref(&key));
    assert!(info.mtime.is_some());
    assert!(info.fingerprint_matches(&key, None));

    write_bytes(&source, 2880, 5);
    assert!(!info.fingerprint_matches(&key, None), "a rewritten source of the same size kept its fingerprint");
    write_bytes(&source, 5760, 60);
    assert!(!info.fingerprint_matches(&key, None));
    fs::remove_file(&source).unwrap();
    assert!(!info.fingerprint_matches(&key, None));
}

#[test]
fn fingerprint_notices_a_rewritten_chain_base() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("m42_light.fits");
    let base = dir.path().join("output").join("m42_light_calibrated.fits");
    write_bytes(&source, 2880, 600);
    write_bytes(&base, 5760, 300);
    let key = format!("{}#hdu=0", source.display());
    let base_key = base.to_string_lossy().to_string();

    let info = ImageInfo::of(&key, Some(&base_key)).unwrap();
    let stamp = info.base.clone().expect("a rebased history did not stamp its chain base");
    assert_eq!((stamp.file_ref, stamp.size), (canonical_ref(&base_key), 5760));
    assert!(info.fingerprint_matches(&key, Some(&base_key)), "an untouched source and base lost their fingerprint");
    assert!(!info.fingerprint_matches(&key, None));

    write_bytes(&base, 5760, 5);
    assert!(!info.fingerprint_matches(&key, Some(&base_key)), "a recalibrated base of the same size kept the old history");
    fs::remove_file(&base).unwrap();
    assert!(!info.fingerprint_matches(&key, Some(&base_key)));

    let plain = ImageInfo::of(&key, None).unwrap();
    assert_eq!(plain.base, None);
    assert!(!serde_json::to_string(&plain).unwrap().contains("\"base\""), "a plain history serialised a base stamp");
    assert!(plain.fingerprint_matches(&key, Some(&key)));
}

#[test]
fn estimate_next_node_bytes_is_four_bytes_per_sample_and_plane() {
    assert_eq!(estimate_next_node_bytes([14344, 8589], 1), 492_802_464);
    assert_eq!(estimate_next_node_bytes([2058, 2058], 1), 16_941_456);
    assert_eq!(estimate_next_node_bytes([7065, 4177], 3), 354_126_060);
}

#[test]
fn pinned_set_keeps_head_parent_root_and_every_fourth_node() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = three_branches(dir.path(), 1024);
    assert_eq!(doc.nodes.len(), 13);
    assert_eq!(doc.head.as_deref(), Some("n0013"));
    assert_eq!(doc.nodes.iter().filter(|n| n.checkpoint).map(|n| n.id.as_str()).collect::<Vec<_>>(), ["n0004", "n0008", "n0012"]);
    assert_eq!(ids_of(&pinned_paths(&doc, PinLevel::Full)), ["n0001", "n0012", "n0013"]);

    set_head(dir.path(), &mut doc, Some("n0011")).unwrap();
    assert_eq!(ids_of(&pinned_paths(&doc, PinLevel::Full)), ["n0001", "n0010", "n0011"]);

    let long = tempfile::tempdir().unwrap();
    let mut seven = new_doc("/data/long.fits");
    chain(long.path(), &mut seven, None, &["background", "wavelet_denoise", "rl_deconv", "lhe", "arcsinh", "tone", "tone"], 64);
    let pinned = pinned_paths(&seven, PinLevel::Full);
    assert_eq!(ids_of(&pinned), ["n0001", "n0004", "n0006", "n0007"]);
    assert!(pinned.contains(&"n0004.png".to_string()), "{pinned:?}");
}

#[test]
fn pinned_set_always_keeps_nodes_that_cannot_be_replayed() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = three_branches(dir.path(), 1 << 20);
    doc.nodes[2].replayable = false;
    for level in [PinLevel::Full, PinLevel::HeadAndParent, PinLevel::Floor] {
        assert!(ids_of(&pinned_paths(&doc, level)).contains(&"n0003".to_string()), "{level:?} released a step replay cannot rebuild");
    }
    assert_eq!(ids_of(&pinned_paths(&doc, PinLevel::Floor)), ["n0003"]);
    let shrunk = pin_levels(&[(&doc, SystemTime::now())], 0);
    assert_eq!(ids_of(&pinned_paths(&doc, shrunk[0])), ["n0003", "n0012", "n0013"]);

    let text = serde_json::to_value(&doc).unwrap();
    assert_eq!(text["nodes"][2]["replayable"], false);
    assert!(text["nodes"][0].get("replayable").is_none(), "a replayable node serialised the flag: {}", text["nodes"][0]);
    let back: HistoryDoc = serde_json::from_value(text).unwrap();
    assert!(!back.nodes[2].replayable && back.nodes[0].replayable);
}

#[test]
fn pinned_set_shrinks_to_head_and_parent_when_it_exceeds_the_cap() {
    let dir = tempfile::tempdir().unwrap();
    let doc = three_branches(dir.path(), 1 << 20);
    let full = 3 * ((1 << 20) + PNG_BYTES as u64);
    let now = SystemTime::now();
    assert_eq!(pin_levels(&[(&doc, now)], full), [PinLevel::Full]);
    assert_eq!(pin_levels(&[(&doc, now)], full - 1), [PinLevel::HeadAndParent]);
    assert_eq!(pin_levels(&[(&doc, now)], 0), [PinLevel::HeadAndParent], "the only history lost its current step");
    assert_eq!(ids_of(&pinned_paths(&doc, PinLevel::HeadAndParent)), ["n0012", "n0013"]);
    assert!(pinned_paths(&doc, PinLevel::Floor).is_empty());
    assert_eq!(doc.nodes[0].bytes, (1 << 20) + PNG_BYTES as u64);
}

#[test]
fn pin_levels_release_the_oldest_images_first_and_keep_the_newest_head() {
    let dirs: Vec<_> = (0..3).map(|_| tempfile::tempdir().unwrap()).collect();
    let docs: Vec<HistoryDoc> = dirs
        .iter()
        .zip(["/data/a.fits", "/data/b.fits", "/data/c.fits"])
        .map(|(dir, key)| {
            let mut doc = new_doc(key);
            chain(dir.path(), &mut doc, None, &STAGES, 1 << 20);
            doc
        })
        .collect();
    let now = SystemTime::now();
    let used = [now - Duration::from_secs(300), now - Duration::from_secs(200), now - Duration::from_secs(100)];
    let tracked: Vec<(&HistoryDoc, SystemTime)> = docs.iter().zip(used).collect();
    let shuffled = [tracked[2], tracked[0], tracked[1]];
    use PinLevel::{Floor, Full, HeadAndParent};

    assert_eq!(pin_levels(&tracked, u64::MAX), [Full, Full, Full]);
    assert_eq!(pin_levels(&tracked, 8 << 20), [HeadAndParent, HeadAndParent, Full]);
    assert_eq!(pin_levels(&tracked, 4 << 20), [Floor, Floor, HeadAndParent]);
    assert_eq!(pin_levels(&tracked, 0), [Floor, Floor, HeadAndParent]);
    assert_eq!(pin_levels(&shuffled, 4 << 20), [HeadAndParent, Floor, Floor], "release followed list order, not last use");
    assert!(pin_levels(&[], 0).is_empty());
}

#[test]
fn protected_paths_cover_the_sidecar_and_side_files() {
    let dir = tempfile::tempdir().unwrap();
    let doc = three_branches(dir.path(), 64);
    let protected = protected_paths(&doc, dir.path(), PinLevel::Full);
    for expected in [sidecar_path(dir.path()), kernels_dir(dir.path()), masks_dir(dir.path())] {
        assert!(protected.contains(&expected), "{} is not protected", expected.display());
    }
    assert!(protected.contains(&dir.path().join("n0013.fits")));
    assert!(!protected.contains(&dir.path().join("n0002.fits")));
}

#[test]
fn replay_plan_starts_at_the_nearest_present_ancestor() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = new_doc("/data/m42.fits");
    let ids = chain(dir.path(), &mut doc, None, &["background", "wavelet_denoise", "rl_deconv", "arcsinh"], 64);
    for id in &ids[1..] {
        fs::remove_file(dir.path().join(format!("{id}.fits"))).unwrap();
    }
    let missing = mark_missing(dir.path(), &mut doc);
    assert_eq!(missing, ["n0002", "n0003", "n0004"]);
    assert_eq!(load(dir.path()).unwrap().unwrap().nodes.iter().filter(|n| !n.present).count(), 3, "eviction marks were not saved");
    assert!(mark_missing(dir.path(), &mut doc).is_empty());

    let plan = replay_plan(&doc, "n0004").unwrap();
    assert_eq!(plan, ReplayPlan { from: Some("n0001".to_string()), steps: vec!["n0002".into(), "n0003".into(), "n0004".into()] });
    assert_eq!(replay_plan(&doc, "n0001").unwrap(), ReplayPlan { from: Some("n0001".to_string()), steps: vec![] });

    fs::remove_file(dir.path().join("n0001.png")).unwrap();
    mark_missing(dir.path(), &mut doc);
    assert_eq!(replay_plan(&doc, "n0002").unwrap(), ReplayPlan { from: None, steps: vec!["n0001".into(), "n0002".into()] });
    assert!(matches!(replay_plan(&doc, "n0042"), Err(HistoryError::UnknownNode(_))));
}

#[test]
#[ignore = "timing: checked locally in A10, three CI runners make the bound flaky"]
fn recorder_overhead_under_15_ms_for_a_200_node_doc() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = new_doc("/data/nrca1_i2d.fits#hdu=1");
    let mut parent: Option<String> = None;
    for i in 0..199 {
        let id = format!("n{:04}", i + 1);
        let output = NodeOutput::named(&id, [2058, 2058], 1, OutputValues::Linear, &[]);
        doc.nodes.push(Node {
            id: id.clone(),
            parent: parent.clone(),
            op: op(STAGES[i % STAGES.len()]),
            mask: None,
            output,
            at: utc_timestamp(SystemTime::now()),
            elapsed_ms: 812,
            app: APP_VERSION.to_string(),
            threads: 16,
            present: true,
            bytes: 16_941_456,
            checkpoint: false,
            replayable: true,
            rebuilt_app: None,
            rebuilt_at: None,
        });
        parent = if i % 7 == 6 { None } else { Some(id) };
    }
    save(dir.path(), &doc).unwrap();
    let id = next_node_id(dir.path(), &doc);
    let output = NodeOutput::named(&id, [2058, 2058], 1, OutputValues::Linear, &[]);
    write_bytes(&dir.path().join(&output.path), 64, 1);
    write_bytes(&dir.path().join(&output.png), 64, 1);
    let node = NewNode { id, parent, op: op("tone"), mask: None, output, elapsed_ms: 3, replayable: true };

    let started = Instant::now();
    append(dir.path(), &mut doc, node).unwrap();
    let elapsed = started.elapsed();
    let written = fs::metadata(sidecar_path(dir.path())).unwrap().len();
    println!("recorder: 200-node append {:.2} ms, sidecar {written} bytes", elapsed.as_secs_f64() * 1000.0);
    assert!(elapsed < Duration::from_millis(15), "append took {elapsed:?}");
    assert!(written <= 100 * 1024, "the sidecar rewrite is {written} bytes");
}

fn downstream_ids(doc: &HistoryDoc, id: &str) -> Vec<String> {
    downstream_of(doc, id).into_iter().map(|n| n.id).collect()
}

#[test]
fn downstream_of_follows_the_line_the_history_tool_shows() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = new_doc("/data/nrca1_i2d.fits#hdu=1");
    let first = chain(dir.path(), &mut doc, None, &["background", "wavelet_denoise", "arcsinh", "tone"], 64);
    assert_eq!(downstream_ids(&doc, "n0002"), ["n0003", "n0004"]);

    set_head(dir.path(), &mut doc, Some("n0002")).unwrap();
    assert_eq!(downstream_ids(&doc, "n0002"), ["n0003", "n0004"], "re-run downstream on the head dropped its undone steps");
    set_head(dir.path(), &mut doc, Some("n0003")).unwrap();
    assert_eq!(downstream_ids(&doc, "n0002"), ["n0003", "n0004"], "a step undone after the head was dropped");
    set_head(dir.path(), &mut doc, None).unwrap();
    assert_eq!(downstream_ids(&doc, "n0001"), ["n0002", "n0003", "n0004"]);

    chain(dir.path(), &mut doc, Some(&first[0]), &["wavelet_denoise", "arcsinh"], 64);
    assert_eq!(doc.head.as_deref(), Some("n0006"));
    assert_eq!(downstream_ids(&doc, "n0001"), ["n0005", "n0006"]);
    assert_eq!(downstream_ids(&doc, "n0002"), ["n0003", "n0004"], "a step off the head path did not follow its newest descent");
    assert!(downstream_ids(&doc, "n0006").is_empty());
    assert!(downstream_ids(&doc, "n0042").is_empty());
}

#[test]
fn remove_subtree_drops_descendants_and_reparents_the_head() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = new_doc("/data/nrca1_i2d.fits#hdu=1");
    let first = chain(dir.path(), &mut doc, None, &STAGES[..3], 64);
    let side = chain(dir.path(), &mut doc, Some(&first[0]), &STAGES[1..3], 64);
    assert_eq!(doc.head.as_deref(), Some(side[1].as_str()));

    let removed: Vec<String> = remove_subtree(dir.path(), &mut doc, &first[1]).unwrap().into_iter().map(|n| n.id).collect();
    assert_eq!(removed, [first[1].clone(), first[2].clone()]);
    assert_eq!(doc.head.as_deref(), Some(side[1].as_str()), "removing another branch moved the head");
    assert_eq!(load(dir.path()).unwrap().unwrap().nodes.len(), 3);

    let removed: Vec<String> = remove_subtree(dir.path(), &mut doc, &side[0]).unwrap().into_iter().map(|n| n.id).collect();
    assert_eq!(removed, side);
    assert_eq!(doc.head.as_deref(), Some(first[0].as_str()), "a head inside the removed branch did not move to its parent");

    remove_subtree(dir.path(), &mut doc, &first[0]).unwrap();
    assert!(doc.nodes.is_empty());
    assert_eq!(doc.head, None);
    assert!(matches!(remove_subtree(dir.path(), &mut doc, "n0042"), Err(HistoryError::UnknownNode(id)) if id == "n0042"));
    assert_eq!(load(dir.path()).unwrap().unwrap().head, None);
}
