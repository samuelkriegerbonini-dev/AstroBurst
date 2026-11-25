use crate::core::history::*;
use std::time::Duration;
use std::fs;
use std::path::Path;
use std::time::SystemTime;

use crate::core::ops::{OpSpec, OutputValues};

use crate::core::history::id::{canonical_ref, image_id};

pub(crate) const STAGES: [&str; 5] = ["background", "wavelet_denoise", "rl_deconv", "arcsinh", "tone"];
pub(crate) const PNG_BYTES: usize = 1024;

pub(crate) fn op(name: &str) -> OpSpec {
    let value = match name {
        "background" => serde_json::json!({"op": "background", "v": 1, "model": "polynomial",
            "config": {"gridSize": 8, "polyDegree": 3, "sigmaClip": 2.5, "iterations": 3, "mode": "subtract"}}),
        "wavelet_denoise" => serde_json::json!({"op": "wavelet_denoise", "v": 1, "numScales": 5,
            "thresholds": [3.0, 2.5, 2.0, 1.5, 1.0], "linearDenoise": true, "layerBias": null}),
        "rl_deconv" => serde_json::json!({"op": "rl_deconv", "v": 1, "iterations": 20, "regularization": 0.001,
            "deringing": true, "deringingThreshold": 0.1, "psf": {"kind": "gaussian", "sigma": 2.0, "size": 15}}),
        "arcsinh" => serde_json::json!({"op": "arcsinh", "v": 1, "factor": 50.0}),
        "tone" => serde_json::json!({"op": "tone", "v": 1, "levels": {"black": 0.1, "gamma": 1.0, "white": 0.9}}),
        "lhe" => serde_json::json!({"op": "lhe", "v": 1, "kernelRadius": 64, "contrastLimit": 2.0, "amount": 1.0,
            "histBits": 8, "circular": true}),
        other => panic!("no fixture op {other}"),
    };
    serde_json::from_value(value).unwrap()
}

pub(crate) fn write_bytes(path: &Path, bytes: usize, seconds_ago: u64) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![b'x'; bytes]).unwrap();
    age_file(path, seconds_ago);
}

pub(crate) fn new_doc(image_key: &str) -> HistoryDoc {
    rebased_doc(image_key, None)
}

pub(crate) fn rebased_doc(image_key: &str, base_path: Option<&str>) -> HistoryDoc {
    let image = ImageInfo { source_ref: canonical_ref(image_key), id: image_id(image_key, base_path), size: 0, mtime: None, base: None, sha256: None };
    HistoryDoc::new(image, image_key, base_path)
}

pub(crate) fn age_file(path: &Path, seconds_ago: u64) {
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(seconds_ago)).unwrap();
}

pub(crate) fn record(dir: &Path, doc: &mut HistoryDoc, parent: Option<&str>, op_name: &str, node_bytes: usize) -> String {
    let id = next_node_id(dir, doc);
    let output = NodeOutput::named(&id, [64, 32], 1, OutputValues::Linear, &[]);
    let age = 100_000 - 10 * doc.nodes.len() as u64;
    write_bytes(&dir.join(&output.path), node_bytes, age);
    write_bytes(&dir.join(&output.png), PNG_BYTES, age);
    let node = NewNode { id: id.clone(), parent: parent.map(str::to_string), op: op(op_name), mask: None, output, elapsed_ms: 5, replayable: true };
    append(dir, doc, node).unwrap();
    id
}

pub(crate) fn chain(dir: &Path, doc: &mut HistoryDoc, from: Option<&str>, ops: &[&str], node_bytes: usize) -> Vec<String> {
    let mut parent = from.map(str::to_string);
    ops.iter()
        .map(|name| {
            let id = record(dir, doc, parent.as_deref(), name, node_bytes);
            parent = Some(id.clone());
            id
        })
        .collect()
}

pub(crate) fn three_branches(dir: &Path, node_bytes: usize) -> HistoryDoc {
    let mut doc = new_doc("/data/nrca1_i2d.fits#hdu=1");
    let first = chain(dir, &mut doc, None, &STAGES, node_bytes);
    chain(dir, &mut doc, Some(&first[0]), &STAGES[1..], node_bytes);
    chain(dir, &mut doc, Some(&first[0]), &STAGES[1..], node_bytes);
    doc
}

pub(crate) fn ids_of(paths: &[String]) -> Vec<String> {
    let mut ids: Vec<String> =
        paths.iter().filter(|p| p.ends_with(".fits")).map(|p| p.trim_end_matches(".fits").to_string()).collect();
    ids.sort();
    ids
}
