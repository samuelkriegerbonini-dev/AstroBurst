import glob
import hashlib
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import tso_golden_lib as tsolib


def main():
    data_root, gold_dir = sys.argv[1], sys.argv[2]
    midx = tsolib.manifest_index(data_root)
    files = []
    for name, m in sorted(midx.items(), key=lambda kv: (kv[1]["set"], kv[0])):
        if not m["set"].startswith("tso"):
            continue
        path = os.path.join(data_root, m["set"], name)
        with open(path, "rb") as fh:
            blob = fh.read()
        head = blob[:9]
        kind = "FITS" if name.endswith(".fits") else ("ECSV" if name.endswith(".ecsv") else "JSON")
        sig = head[:9] == b"SIMPLE  =" if kind == "FITS" else (head[:7] == b"# %ECSV" if kind == "ECSV" else blob.lstrip()[:1] == b"{")
        files.append({"set": m["set"], "file": name, "kind": kind, "bytes_manifest": m["bytes"], "bytes_on_disk": len(blob), "sha256_manifest": m["sha256"], "sha256_matches": hashlib.sha256(blob).hexdigest() == m["sha256"], "signature_ok": bool(sig)})
    goldens = []
    for p in sorted(glob.glob(os.path.join(gold_dir, "*.json"))):
        base = os.path.basename(p)
        if base == "index.json":
            continue
        g = json.load(open(p, "r", encoding="utf-8"))
        goldens.append({"golden_file": base, "kind": g["golden"], "label": g.get("label"), "script": g["produced_by"]["script"], "inputs": [i["file"] for i in g["produced_by"]["inputs"]]})
    out = {
        "golden": "tso/index",
        "produced_by": tsolib.golden_meta("tso_golden_index.py", "<data root> <goldens/tso dir>", [{"file": "MANIFEST.tsv", "set": "", "bytes": os.path.getsize(os.path.join(data_root, "MANIFEST.tsv")), "sha256": hashlib.sha256(open(os.path.join(data_root, "MANIFEST.tsv"), "rb").read()).hexdigest()}]),
        "data_files_checked": len(files),
        "all_sizes_signatures_and_sha256_ok": all(f["bytes_manifest"] == f["bytes_on_disk"] and f["sha256_matches"] and f["signature_ok"] for f in files),
        "data_files": files,
        "goldens": goldens,
        "test_gate": "real-data tests are ignored tests gated by an environment variable that points at the data root; nothing from the data folder is committed",
    }
    tsolib.write_golden(os.path.join(gold_dir, "index.json"), out)
    print("data files", len(files), "all ok", out["all_sizes_signatures_and_sha256_ok"], "goldens", len(goldens))


main()
