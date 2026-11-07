import argparse
import base64
import contextlib
import hashlib
import importlib.metadata
import importlib.util
import inspect
import io
import json
import math
import os
import platform
import re
import shutil
import subprocess
import sys
import time
import warnings
from pathlib import Path
from urllib.parse import urlparse
from urllib.request import url2pathname

MANIFEST_VERSION = 2
RECIPE_VERSION = 1
CREATED_BY = "scripts/oracle/lightstack_oracle.py"
PACKAGES = ("lightstack", "numpy", "scipy", "astropy", "reproject", "photutils", "regions", "matplotlib", "pandas")
ALIGN_METHODS = {"bilinear": "interp", "exact": "exact"}
CUBE_ORDERS = ("wavelength", "given")
CROP_MODES = {
    "radec": ({"ra", "dec", "size_arcsec"}, set()),
    "reference": ({"reference"}, set()),
    "common": (set(), set()),
    "region": ({"region_file"}, {"index"}),
    "region_common": ({"region_file"}, set()),
    "pixel": ({"x_start", "x_end", "y_start", "y_end"}, set()),
}
EDIT_ACTIONS = {
    "drop_invalid": (set(), {"threshold", "frac_valid"}),
    "remove_filter": ({"filter"}, set()),
    "cut": ({"x_start", "x_end", "y_start", "y_end"}, set()),
    "header": (set(), {"redshift", "ra_center", "dec_center", "use_brightest_pixel"}),
    "reference_nan_mask": (set(), {"reference_slice"}),
}
EMPIRICAL_PARAMETERS = {
    "cutout_size",
    "background_border",
    "background_sigma",
    "clip_negative",
    "noise_threshold",
    "radial_cleaning",
    "cleaning_sigma",
    "cleaning_consecutive_bins",
    "normalization_radius",
    "normalization_radius_unit",
    "psf_size",
    "oversampling",
    "oversampling_order",
}
WINDOWS = {
    "top_hat": ("TopHatWindow", ("beta",)),
    "cosine_bell": ("CosineBellWindow", ("alpha",)),
    "split_cosine_bell": ("SplitCosineBellWindow", ("alpha", "beta")),
    "hanning": ("HanningWindow", ()),
    "tukey": ("TukeyWindow", ("alpha",)),
}
COUNT_RATE_UNITS = {"electrons/s", "electron/s", "e-/s", "e/s", "counts/s", "count/s"}
STEP_FUNCTIONS = {
    "units": "MJy_sr_to_jy (MJy/sr planes), counts_s_to_jy (count-rate planes)",
    "align": "align_reproject_fits",
    "cube": "build_datacube",
    "match": "psf_match_datacube",
    "export": "copy of the current cube",
}
CROP_FUNCTIONS = {
    "radec": "crop_from_radec",
    "reference": "crop_using_reference",
    "common": "get_common_sky_bbox, crop_using_reference",
    "region": "crop_reg",
    "region_common": "get_common_region_from_reg, crop_reg",
    "pixel": "cut_region_2d",
}
EDIT_FUNCTIONS = {
    "drop_invalid": "build_valid_datacube",
    "remove_filter": "remove_filter",
    "cut": "cut_region_datacube",
    "header": "update_cube_header",
    "reference_nan_mask": "apply_reference_nan_mask",
}
PHOTUTILS_REGULARIZATION = 1e-4
LIGHTSTACK_VALUES = {
    "crop": {"rounding": "lightstack"},
    "align": {"area_rule": "lightstack"},
    "cube": {"layout": "lightstack"},
    "edit drop_invalid": {"use_dq": False},
    "edit reference_nan_mask": {"mode": "reference"},
    "psfs files": {"recentre": False},
    "psfs empirical": {"stack": "median", "recentre": False},
    "kernels wiener": {"eps_mode": "absolute", "auto": False},
    "kernels window": {"regularization": PHOTUTILS_REGULARIZATION, "auto": False},
    "match": {"mode": "lightstack", "allow_unmatched": False},
}
STEP_VARIANTS = {"edit": "action", "psfs": "source", "kernels": "method"}


class RecipeError(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise RecipeError(message)


def is_number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def is_count(value):
    return isinstance(value, int) and not isinstance(value, bool) and value >= 0


def check_keys(where, item, required, optional):
    require(isinstance(item, dict), f"{where}: expected a JSON object")
    missing = sorted(required - item.keys())
    unknown = sorted(item.keys() - required - optional)
    require(not missing, f"{where}: missing {missing}")
    require(not unknown, f"{where}: unknown {unknown}; LIGHTSTACK has no such parameter")


def check_file(where, data_dir, relative):
    require(isinstance(relative, str) and (data_dir / relative).is_file(), f"{where}: {relative!r} is not a file under {data_dir}")


def check_pixel_box(where, step):
    for key in ("x_start", "x_end", "y_start", "y_end"):
        require(is_count(step[key]), f"{where}: {key} must be a non-negative integer")
    require(step["x_start"] < step["x_end"] and step["y_start"] < step["y_end"], f"{where}: empty pixel box")


def shown(value):
    return value if isinstance(value, str) else json.dumps(value)


def lightstack_step(where, step):
    if not isinstance(step, dict):
        return step
    kind = step.get("step")
    detail = STEP_VARIANTS.get(kind)
    extensions = LIGHTSTACK_VALUES.get(f"{kind} {step.get(detail)}" if detail else kind, {})
    form = {}
    for key, value in step.items():
        if value is None:
            continue
        if key in extensions:
            expected = extensions[key]
            require(type(value) is type(expected) and value == expected, f"{where} ({kind}): AstroBurst-only value {key}={shown(value)}; LIGHTSTACK has no equivalent")
            continue
        form[key] = value
    return form


def lightstack_form(recipe):
    require(isinstance(recipe, dict), "recipe: expected a JSON object")
    form = {key: value for key, value in recipe.items() if not (key == "name" and value is None)}
    if isinstance(form.get("steps"), list):
        form["steps"] = [lightstack_step(f"steps[{number}]", step) for number, step in enumerate(form["steps"])]
    return form


def validate(recipe, data_dir):
    check_keys("recipe", recipe, {"recipe_version", "inputs", "steps"}, {"name"})
    require(recipe["recipe_version"] == RECIPE_VERSION, f"recipe_version {recipe['recipe_version']} is not {RECIPE_VERSION}")
    require(isinstance(recipe["inputs"], list) and recipe["inputs"], "the recipe has no input")
    filters = []
    for number, item in enumerate(recipe["inputs"]):
        where = f"inputs[{number}]"
        check_keys(where, item, {"path", "dataset", "filter"}, set())
        check_file(where, data_dir, item["path"])
        require(item["filter"] not in filters, f"{where}: filter {item['filter']} appears twice")
        filters.append(item["filter"])
    require(isinstance(recipe["steps"], list) and recipe["steps"], "the recipe has no step")
    state = {"aligned": None, "cube": False, "psfs": set(), "kernels": None}
    for number, step in enumerate(recipe["steps"]):
        validate_step(f"steps[{number}]", step, filters, data_dir, state)


def validate_step(where, step, filters, data_dir, state):
    require(isinstance(step, dict) and isinstance(step.get("step"), str), f"{where}: no step name")
    kind = step["step"]
    parameters = {key: value for key, value in step.items() if key != "step"}
    where = f"{where} ({kind})"
    if kind == "units":
        check_keys(where, parameters, {"to"}, set())
        require(parameters["to"] == "Jy/pixel", f"{where}: LIGHTSTACK converts to Jy/pixel only")
    elif kind == "crop":
        mode = parameters.get("mode")
        require(mode in CROP_MODES, f"{where}: mode {mode!r} is not one of {sorted(CROP_MODES)}")
        required, optional = CROP_MODES[mode]
        check_keys(where, parameters, required | {"mode"}, optional)
        if mode == "radec":
            require(all(is_number(parameters[key]) for key in ("ra", "dec", "size_arcsec")), f"{where}: ra, dec and size_arcsec must be numbers")
            require(parameters["size_arcsec"] > 0, f"{where}: size_arcsec must be positive")
        if mode == "reference":
            require(parameters["reference"] in filters, f"{where}: reference {parameters['reference']} is not an input filter")
        if mode in ("region", "region_common"):
            check_file(where, data_dir, parameters["region_file"])
            require(is_count(parameters.get("index", 0)), f"{where}: index must be a non-negative integer")
        if mode == "pixel":
            check_pixel_box(where, parameters)
    elif kind == "align":
        check_keys(where, parameters, {"reference"}, {"method", "border_crop"})
        require(parameters["reference"] in filters, f"{where}: reference {parameters['reference']} is not an input filter")
        require(parameters.get("method", "bilinear") in ALIGN_METHODS, f"{where}: method must be one of {sorted(ALIGN_METHODS)}")
        require(is_count(parameters.get("border_crop", 1)), f"{where}: border_crop must be a non-negative integer")
        state["aligned"] = parameters["reference"]
    elif kind == "cube":
        check_keys(where, parameters, set(), {"reference", "order", "output"})
        reference = parameters.get("reference", state["aligned"])
        require(reference in filters, f"{where}: name a reference filter or align first")
        require(parameters.get("order", "wavelength") in CUBE_ORDERS, f"{where}: order must be one of {list(CUBE_ORDERS)}")
        require(str(parameters.get("output", "cube.fits")).endswith(".fits"), f"{where}: output must end in .fits")
        state["cube"] = True
    elif kind == "edit":
        require(state["cube"], f"{where}: build a cube first")
        action = parameters.get("action")
        require(action in EDIT_ACTIONS, f"{where}: action {action!r} is not one of {sorted(EDIT_ACTIONS)}")
        required, optional = EDIT_ACTIONS[action]
        check_keys(where, parameters, required | {"action"}, optional)
        if action == "cut":
            check_pixel_box(where, parameters)
        if action == "remove_filter":
            require(parameters["filter"] in filters, f"{where}: {parameters['filter']} is not an input filter")
    elif kind == "psfs":
        source = parameters.get("source")
        require(source in ("files", "empirical"), f"{where}: source must be files or empirical")
        if source == "files":
            check_keys(where, parameters, {"source", "files"}, {"reference", "pixel_scale_arcsec"})
            require(isinstance(parameters["files"], list) and parameters["files"], f"{where}: files must be a non-empty list")
            names = []
            for index, item in enumerate(parameters["files"]):
                check_keys(f"{where} files[{index}]", item, {"filter", "path"}, set())
                check_file(f"{where} files[{index}]", data_dir, item["path"])
                names.append(item["filter"])
        else:
            check_keys(where, parameters, {"source", "dataset", "filters"}, EMPIRICAL_PARAMETERS | {"data_dir", "reference", "pixel_scale_arcsec"})
            require(isinstance(parameters["filters"], list) and parameters["filters"], f"{where}: filters must be a non-empty list")
            require((data_dir / parameters.get("data_dir", ".")).is_dir(), f"{where}: data_dir is not a directory under {data_dir}")
            names = list(parameters["filters"])
        require(len(set(names)) == len(names), f"{where}: a filter has two PSFs")
        require(set(names) <= set(filters), f"{where}: PSFs for filters that are not inputs: {sorted(set(names) - set(filters))}")
        if "pixel_scale_arcsec" in parameters:
            require(is_number(parameters["pixel_scale_arcsec"]) and parameters["pixel_scale_arcsec"] > 0, f"{where}: pixel_scale_arcsec must be positive")
            require(parameters.get("reference") in names, f"{where}: resampling needs a reference filter among the PSFs")
        state["psfs"] = set(names)
    elif kind == "kernels":
        method = parameters.get("method")
        require(method in ("wiener", "window"), f"{where}: method must be wiener or window")
        if method == "wiener":
            check_keys(where, parameters, {"reference", "method"}, {"eps", "shape"})
            require(is_number(parameters.get("eps", 1e-3)) and parameters.get("eps", 1e-3) > 0, f"{where}: eps must be positive")
            shape = parameters.get("shape", "reference")
            require(shape == "reference" or (isinstance(shape, list) and len(shape) == 2 and all(is_count(side) and side % 2 == 1 for side in shape)), f"{where}: shape must be reference or two odd integers")
        else:
            check_keys(where, parameters, {"reference", "method", "window"}, {"alpha", "beta"})
            require(parameters["window"] in WINDOWS, f"{where}: window must be one of {sorted(WINDOWS)}")
            names = WINDOWS[parameters["window"]][1]
            require(set(parameters) - {"reference", "method", "window"} <= set(names), f"{where}: window {parameters['window']} takes {list(names)}")
        require(parameters["reference"] in state["psfs"], f"{where}: no PSF for the reference {parameters['reference']}")
        state["kernels"] = parameters["reference"]
    elif kind == "match":
        check_keys(where, parameters, {"reference"}, set())
        require(state["cube"], f"{where}: build a cube first")
        require(state["kernels"] == parameters["reference"], f"{where}: no kernels to {parameters['reference']}")
    elif kind == "export":
        check_keys(where, parameters, {"output"}, set())
        require(state["cube"], f"{where}: build a cube first")
        require(str(parameters["output"]).endswith(".fits") and "/" not in parameters["output"] and "\\" not in parameters["output"], f"{where}: output must be a .fits file name")
    else:
        raise RecipeError(f"{where}: unknown step; known steps are units, crop, align, cube, edit, psfs, kernels, match, export")


def plan(recipe):
    lines = [f"recipe {recipe.get('name', '')}: {len(recipe['inputs'])} inputs, {len(recipe['steps'])} steps"]
    for index, step in enumerate(recipe["steps"], start=1):
        kind = step["step"]
        if kind == "crop":
            functions = CROP_FUNCTIONS[step["mode"]]
        elif kind == "edit":
            functions = EDIT_FUNCTIONS[step["action"]]
        elif kind == "psfs":
            functions = "build_empirical_psf, " if step["source"] == "empirical" else ""
            functions += "pad_psf_for_resampling and resample_psf_to_pixel_scale where the scale differs, " if "pixel_scale_arcsec" in step else ""
            functions += "measure_psf_fwhm"
        elif kind == "kernels":
            functions = "build_kernel, save_kernel" if step["method"] == "wiener" else "photutils create_matching_kernel, save_kernel"
        else:
            functions = STEP_FUNCTIONS[kind]
        lines.append(f"{index:02d}_{kind}: {functions}")
    return lines


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def finite(value):
    if isinstance(value, dict):
        return {key: finite(item) for key, item in value.items()}
    if isinstance(value, list):
        return [finite(item) for item in value]
    if isinstance(value, float) and not math.isfinite(value):
        return None
    return value


def file_entry(path, root):
    path = Path(path)
    return {"path": path.relative_to(root).as_posix(), "bytes": path.stat().st_size, "sha256": sha256(path)}


def versions():
    found = {"python": platform.python_version()}
    for package in PACKAGES:
        found[package] = importlib.metadata.version(package)
    return found


def code_sha256(package_dir):
    root = Path(package_dir).parent
    lines = sorted(f"{path.relative_to(root).as_posix()}\0{hashlib.sha256(path.read_bytes().replace(b'\r\n', b'\n')).hexdigest()}\n" for path in Path(package_dir).rglob("*.py"))
    return hashlib.sha256("".join(lines).encode("utf-8")).hexdigest()


def modified_since_install(distribution):
    files = distribution.files
    if files is None:
        return None
    modified = []
    for file in files:
        if file.hash is None:
            continue
        path = Path(distribution.locate_file(file))
        digest = hashlib.new(file.hash.mode, path.read_bytes()).digest() if path.is_file() else b""
        if base64.urlsafe_b64encode(digest).rstrip(b"=").decode("ascii") != file.hash.value:
            modified.append(file.as_posix())
    return sorted(modified)


def git(directory, *arguments):
    return subprocess.run(["git", "-C", str(directory), *arguments], capture_output=True, text=True)


def lightstack_source(distribution, package_dir):
    source = {"kind": "index"}
    direct = distribution.read_text("direct_url.json")
    if direct:
        recorded = json.loads(direct)
        url = recorded.get("url", "")
        source = {"kind": "direct", "url": url}
        source.update({key: recorded[key] for key in ("vcs_info", "archive_info", "dir_info") if key in recorded})
        directory = Path(url2pathname(urlparse(url).path)) if url.startswith("file:") else None
        if directory is not None and directory.is_dir():
            head = git(directory, "rev-parse", "HEAD")
            if head.returncode == 0:
                source["commit"] = head.stdout.strip()
            status = git(directory, "status", "--porcelain")
            if status.returncode == 0:
                source["dirty"] = bool(status.stdout.strip())
    source["code_sha256"] = code_sha256(package_dir)
    source["modified_since_install"] = modified_since_install(distribution)
    return source


def parse_align(text):
    planes = {}
    for block in re.split(r"\n(?=Processing )", text):
        head = re.match(r"\s*Processing (\S+) / (\S+)", block)
        if head is None:
            continue
        area = re.search(r"Applied area correction: (\S+)", block)
        ratio = re.search(r"Flux ratio \(out/in\): (\S+)", block)
        scales = re.search(r"Pixel scale in/out: (\S+) -> (\S+)", block)
        planes[head.group(2)] = {
            "reprojected": "Already aligned with reference" not in block,
            "area_correction": float(area.group(1)) if area else None,
            "flux_ratio_printed": float(ratio.group(1)) if ratio else None,
            "pixel_scale_in_printed": float(scales.group(1)) if scales else None,
            "pixel_scale_out_printed": float(scales.group(2)) if scales else None,
        }
    return planes


def effective_arguments(function, arguments):
    signature = inspect.signature(function)
    bound = signature.bind(**arguments)
    bound.apply_defaults()
    effective = {}
    for name, value in bound.arguments.items():
        kind = signature.parameters[name].kind
        if kind is inspect.Parameter.VAR_KEYWORD:
            effective.update(value)
        elif kind is not inspect.Parameter.VAR_POSITIONAL:
            effective[name] = value
    return effective


class Oracle:
    def __init__(self, recipe, data_dir, out_dir):
        self.recipe = recipe
        self.data_dir = data_dir.resolve()
        self.out_dir = out_dir.resolve()
        self.images = [{"path": self.data_dir / item["path"], "dataset": item["dataset"], "filter": item["filter"]} for item in recipe["inputs"]]
        self.datasets = {item["filter"]: item["dataset"] for item in recipe["inputs"]}
        self.aligned_reference = None
        self.cube = None
        self.psfs = {}
        self.kernel_dir = None
        self.steps = []

    def scrub(self, text):
        for path, label in ((self.out_dir, "<out>"), (self.data_dir, "<data>")):
            text = text.replace(str(path), label).replace(path.as_posix(), label)
        return text

    def jsonable(self, value):
        if isinstance(value, dict):
            return {str(key): self.jsonable(item) for key, item in value.items()}
        if isinstance(value, (list, tuple)):
            return [self.jsonable(item) for item in value]
        if isinstance(value, (str, Path)):
            return self.scrub(str(value))
        if value is None or isinstance(value, (bool, int, float)):
            return value
        if hasattr(value, "shape") and hasattr(value, "dtype"):
            return value.item() if value.ndim == 0 else f"<array {value.dtype} {list(value.shape)}>"
        if hasattr(value, "item"):
            return value.item()
        return self.scrub(repr(value))

    def entry(self, path):
        path = Path(path).resolve()
        for root, label in ((self.out_dir, "out"), (self.data_dir, "data")):
            if path.is_relative_to(root):
                return {"root": label, **file_entry(path, root)}
        raise RecipeError(f"{path} is outside the data and output folders")

    def call(self, record, name, function, arguments, inputs, tolerate=()):
        buffer = io.StringIO()
        started = time.perf_counter()
        result, error = None, None
        try:
            with warnings.catch_warnings(record=True) as caught, contextlib.redirect_stdout(buffer):
                warnings.simplefilter("always")
                try:
                    result = function(**arguments)
                except tolerate as failure:
                    error = f"{type(failure).__name__}: {failure}"
        except BaseException:
            print(f"{name} raised; it printed and warned before:", file=sys.stderr)
            sys.stderr.write(buffer.getvalue())
            for item in caught:
                print(f"{item.category.__name__}: {item.message}", file=sys.stderr)
            raise
        elapsed = time.perf_counter() - started
        text = buffer.getvalue()
        sys.stdout.write(text)
        entry = {
            "function": name,
            "arguments": self.jsonable(arguments),
            "inputs": [self.entry(path) for path in inputs],
            "outputs": [],
            "metrics": {},
            "stdout": self.scrub(text),
            "warnings": [self.scrub(f"{item.category.__name__}: {item.message}") for item in caught],
            "wall_time_s": elapsed,
        }
        if error is not None:
            entry["error"] = self.scrub(error)
            print(f"{name} failed: {error}")
        record["calls"].append(entry)
        return result, entry

    def image(self, filter_name):
        return next(item for item in self.images if item["filter"] == filter_name)

    def run(self):
        for index, step in enumerate(self.recipe["steps"], start=1):
            kind = step["step"]
            directory = self.out_dir / f"{index:02d}_{kind}"
            directory.mkdir()
            record = {"index": index, "step": kind, "parameters": step, "directory": directory.name, "calls": [], "notes": []}
            print(f"== {directory.name}")
            getattr(self, f"step_{kind}")(step, directory, record)
            record["outputs"] = [self.entry(path) for path in sorted(directory.rglob("*")) if path.is_file()]
            self.steps.append(record)

    def step_units(self, step, directory, record):
        from astropy.io import fits
        from lightstack.utils import MJy_sr_to_jy, counts_s_to_jy, find_ext

        converted = []
        for image in self.images:
            with fits.open(image["path"]) as hdul:
                header = hdul[find_ext(hdul)].header
                unit = str(header.get("BUNIT", "")).strip()
                pixar_sr = header.get("PIXAR_SR")
            normalized = unit.replace(" ", "").lower()
            if normalized == "jy":
                record["notes"].append(f"{image['filter']}: BUNIT Jy, passed through")
                converted.append(image)
                continue
            if normalized == "mjy/sr":
                staged = directory / image["path"].name
                shutil.copyfile(image["path"], staged)
                result, entry = self.call(record, "lightstack.utils.MJy_sr_to_jy", MJy_sr_to_jy, {"aligned_list": [(str(staged), image["filter"])]}, [image["path"]])
                staged.unlink()
                require(result, f"units: MJy_sr_to_jy skipped {image['filter']} (no PIXAR_SR)")
                output = Path(result[0][0])
                entry["metrics"] = {"filter": image["filter"], "bunit_in": unit, "factor": 1e6 * float(pixar_sr)}
            elif normalized in COUNT_RATE_UNITS:
                output = directory / f"{image['path'].stem}_Jy.fits"
                _, entry = self.call(record, "lightstack.utils.counts_s_to_jy", counts_s_to_jy, {"fits_path": str(image["path"]), "output_path": str(output)}, [image["path"]])
                method = re.search(r"Method\s*:\s*(\S+)", entry["stdout"])
                factor = re.search(r"Factor\s*:\s*(\S+)", entry["stdout"])
                entry["metrics"] = {"filter": image["filter"], "bunit_in": unit, "method": method and method.group(1), "factor_printed": factor and float(factor.group(1))}
            else:
                raise RecipeError(f"units: {image['filter']} has BUNIT {unit!r}, which LIGHTSTACK cannot convert to Jy/pixel")
            entry["outputs"] = [self.entry(output)]
            converted.append({**image, "path": output})
        self.images = converted

    def crop_metrics(self, source, output):
        from astropy.io import fits
        from lightstack.utils import find_ext

        with fits.open(source) as before, fits.open(output) as after:
            header_in = before[find_ext(before)].header
            hdu_out = after[find_ext(after)]
            return {
                "shape": list(hdu_out.data.shape),
                "x_min": float(header_in["CRPIX1"] - hdu_out.header["CRPIX1"]),
                "y_min": float(header_in["CRPIX2"] - hdu_out.header["CRPIX2"]),
            }

    def step_crop(self, step, directory, record):
        from astropy.io import fits
        from lightstack import crop

        mode = step["mode"]
        sky_bbox = None
        region = None
        if mode == "common":
            sources = [image["path"] for image in self.images]
            sky_bbox, entry = self.call(record, "lightstack.crop.get_common_sky_bbox", crop.get_common_sky_bbox, {"fits_files": [str(path) for path in sources]}, sources)
            entry["metrics"] = {"sky_bbox": {key: float(value) for key, value in sky_bbox.items()}}
        if mode == "region_common":
            region_file = self.data_dir / step["region_file"]
            (region, bbox), entry = self.call(record, "lightstack.crop.get_common_region_from_reg", crop.get_common_region_from_reg, {"reg_path": str(region_file)}, [region_file])
            entry["metrics"] = {"sky_bbox": {key: float(value) for key, value in bbox.items()}}
        if mode == "region":
            from regions import Regions

            region = Regions.read(str(self.data_dir / step["region_file"]), format="ds9")[step.get("index", 0)]
        reference = self.image(step["reference"])["path"] if mode == "reference" else None
        cropped = []
        for image in self.images:
            source = image["path"]
            output = directory / source.name
            if mode == "radec":
                arguments = {"fits_path": str(source), "ra": step["ra"], "dec": step["dec"], "size_arcsec": step["size_arcsec"], "output_path": str(output)}
                _, entry = self.call(record, "lightstack.crop.crop_from_radec", crop.crop_from_radec, arguments, [source])
            elif mode in ("reference", "common"):
                arguments = {"fits_path": str(source), "output_path": str(output)}
                arguments.update({"ref_cutout": str(reference)} if mode == "reference" else {"sky_bbox": sky_bbox})
                _, entry = self.call(record, "lightstack.crop.crop_using_reference", crop.crop_using_reference, arguments, [source] + ([reference] if reference else []))
            elif mode in ("region", "region_common"):
                (data, header), entry = self.call(record, "lightstack.crop.crop_reg", crop.crop_reg, {"fits_path": str(source), "region": region}, [source])
                fits.PrimaryHDU(data=data, header=header).writeto(output, overwrite=True)
            else:
                arguments = {"fits_file": str(source), "output_path": str(output), **{key: step[key] for key in ("x_start", "x_end", "y_start", "y_end")}}
                _, entry = self.call(record, "lightstack.crop.cut_region_2d", crop.cut_region_2d, arguments, [source])
            entry["outputs"] = [self.entry(output)]
            entry["metrics"].update({"filter": image["filter"], **self.crop_metrics(source, output)})
            cropped.append({**image, "path": output})
        self.images = cropped

    def step_align(self, step, directory, record):
        import numpy as np
        from astropy.io import fits
        from lightstack.datacube import align_reproject_fits
        from lightstack.utils import find_ext

        def total(path):
            with fits.open(path) as hdul:
                return float(np.nansum(hdul[find_ext(hdul)].data.astype(float)))

        method = ALIGN_METHODS[step.get("method", "bilinear")]
        staged = []
        for image in self.images:
            target = directory / image["path"].name
            shutil.copyfile(image["path"], target)
            staged.append({**image, "staged": target})
        reference = next(item["staged"] for item in staged if item["filter"] == step["reference"])
        arguments = {
            "fits_list": [(str(item["staged"]), item["dataset"], item["filter"]) for item in staged],
            "ref_file": str(reference),
            "method": method,
            "crop": step.get("border_crop", 1),
        }
        aligned, entry = self.call(record, "lightstack.datacube.align_reproject_fits", align_reproject_fits, arguments, [item["path"] for item in staged])
        printed = parse_align(entry["stdout"])
        planes = {}
        for path, _, filter_name in aligned:
            source = next(item["staged"] for item in staged if item["filter"] == filter_name)
            sum_in = total(source)
            planes[filter_name] = {**printed.get(filter_name, {}), "flux_ratio": total(path) / sum_in if sum_in else None}
        for item in staged:
            item["staged"].unlink()
        entry["outputs"] = [self.entry(path) for path, _, _ in aligned]
        entry["metrics"] = {"reference": step["reference"], "method": method, "planes": planes}
        self.images = [{"path": Path(path), "dataset": dataset, "filter": filter_name} for path, dataset, filter_name in aligned]
        self.aligned_reference = step["reference"]

    def step_cube(self, step, directory, record):
        from lightstack.datacube import build_datacube
        from lightstack.utils import get_filter_info

        images = list(self.images)
        if step.get("order", "wavelength") == "wavelength":
            images.sort(key=lambda item: get_filter_info(item["filter"], item["dataset"])["lambda"])
        reference = next(item["path"] for item in images if item["filter"] == step.get("reference", self.aligned_reference))
        output = directory / step.get("output", "cube.fits")
        arguments = {"selected_fits": [(str(item["path"]), item["dataset"], item["filter"]) for item in images], "reference_file": str(reference), "output_path": str(output)}
        _, entry = self.call(record, "lightstack.datacube.build_datacube", build_datacube, arguments, [item["path"] for item in images])
        entry["outputs"] = [self.entry(output)]
        entry["metrics"] = {"filters": [item["filter"] for item in images], "datasets": [item["dataset"] for item in images]}
        self.cube = output

    def cube_filters(self):
        from astropy.io import fits

        header = fits.getheader(self.cube)
        return [str(header[f"FILTER{index}"]).strip() for index in range(1, header["NAXIS3"] + 1)]

    def step_edit(self, step, directory, record):
        from lightstack import datacube

        action = step["action"]
        stem = self.cube.stem
        source = str(self.cube)
        if action == "drop_invalid":
            output = directory / f"{stem}_filtered.fits"
            before = self.cube_filters()
            arguments = {"cube_fits_file": source, "output_cube": str(output), "threshold": step.get("threshold", 0.0), "frac_valid": step.get("frac_valid", 0.01)}
            (_, valid), entry = self.call(record, "lightstack.datacube.build_valid_datacube", datacube.build_valid_datacube, arguments, [self.cube])
            entry["metrics"] = {"valid_filters": list(valid), "removed_filters": [name for name in before if name not in valid]}
        elif action == "remove_filter":
            output = directory / f"{stem}_without_{step['filter'].lower()}.fits"
            arguments = {"cube_fits_file": source, "output_cube": str(output), "filter_to_remove": step["filter"]}
            (_, remaining), entry = self.call(record, "lightstack.datacube.remove_filter", datacube.remove_filter, arguments, [self.cube])
            entry["metrics"] = {"filters": list(remaining), "written": output.exists()}
        elif action == "cut":
            output = directory / f"{stem}_cut.fits"
            arguments = {"cube_fits_file": source, "output_path": str(output), **{key: step[key] for key in ("x_start", "x_end", "y_start", "y_end")}}
            _, entry = self.call(record, "lightstack.datacube.cut_region_datacube", datacube.cut_region_datacube, arguments, [self.cube])
        elif action == "header":
            output = directory / f"{stem}_more.fits"
            arguments = {
                "cube_fits_file": source,
                "output_file": str(output),
                "redshift": step.get("redshift"),
                "ra_center": step.get("ra_center"),
                "dec_center": step.get("dec_center"),
                "use_brightest_pixel": step.get("use_brightest_pixel", False),
                "overwrite": True,
            }
            _, entry = self.call(record, "lightstack.datacube.update_cube_header", datacube.update_cube_header, arguments, [self.cube])
        else:
            output = directory / f"{stem}_masked.fits"
            arguments = {"cube_file": source, "reference_slice": step.get("reference_slice", 0), "output_path": str(output)}
            _, entry = self.call(record, "lightstack.datacube.apply_reference_nan_mask", datacube.apply_reference_nan_mask, arguments, [self.cube])
        if output.exists():
            entry["outputs"] = [self.entry(output)]
            self.cube = output

    def empirical(self, step, directory, record):
        from lightstack.psf import build_empirical_psf

        data_dir = self.data_dir / step.get("data_dir", ".")
        parameters = {key: value for key, value in step.items() if key in EMPIRICAL_PARAMETERS}
        inputs = sorted(data_dir.glob("*.fits")) + sorted(data_dir.glob("stars_*.reg"))
        sources = {}
        for filter_name in step["filters"]:
            arguments = effective_arguments(
                build_empirical_psf,
                {
                    "data_dir": str(data_dir),
                    "dataset": step["dataset"],
                    "filter_name": filter_name,
                    "output_dir": str(directory),
                    "plot_stars": False,
                    "plot_diagnostics": False,
                    "show_figures": False,
                    **parameters,
                },
            )
            result, entry = self.call(record, "lightstack.psf.build_empirical_psf", build_empirical_psf, arguments, inputs)
            produced = [path for path in (result["output_path"], result["output_oversampled_path"]) if path is not None]
            entry["outputs"] = [self.entry(path) for path in produced]
            entry["metrics"] = {
                "filter": result["filter"],
                "pixel_scale_arcsec": float(result["pixel_scale"]),
                "nstars": int(len(result["stars"])),
                "centers": self.jsonable(result["centers"]),
                "backgrounds": self.jsonable(result["backgrounds"]),
                "background_stds": self.jsonable(result["background_stds"]),
                "cleaning_radii": self.jsonable(result["cleaning_radii"]),
                "fluxes": self.jsonable(result["fluxes"]),
            }
            sources[filter_name] = Path(result["output_path"])
        return sources

    def step_psfs(self, step, directory, record):
        import numpy as np
        from astropy.io import fits
        from lightstack import psf
        from lightstack.utils import get_pixel_scale

        if step["source"] == "files":
            sources = {item["filter"]: self.data_dir / item["path"] for item in step["files"]}
        else:
            sources = self.empirical(step, directory, record)
        target_scale = step.get("pixel_scale_arcsec")
        reference_shape = None
        if target_scale is not None:
            with fits.open(sources[step["reference"]]) as hdul:
                reference_shape = tuple(hdul[0].data.shape)
        for filter_name, path in sources.items():
            scale = get_pixel_scale(path)
            if target_scale is not None and not np.isclose(scale, target_scale, rtol=1e-6):
                arguments = {"psf_path": str(path), "target_pixel_scale": target_scale, "target_nx": reference_shape[1], "target_ny": reference_shape[0]}
                (padded, info), entry = self.call(record, "lightstack.psf.pad_psf_for_resampling", psf.pad_psf_for_resampling, arguments, [path])
                entry["metrics"] = {"filter": filter_name, **self.jsonable(info)}
                arguments = {"psf": padded, "source_pixel_scale": info["source_pixel_scale"], "target_pixel_scale": target_scale, "output_shape": reference_shape}
                array, entry = self.call(record, "lightstack.psf.resample_psf_to_pixel_scale", psf.resample_psf_to_pixel_scale, arguments, [path])
                used = directory / f"{path.stem}_resampled.fits"
                header = fits.Header()
                header["FILTER"] = filter_name
                header["PIXSCALE"] = target_scale
                fits.PrimaryHDU(array, header=header).writeto(used)
                entry["outputs"] = [self.entry(used)]
                entry["metrics"] = {"filter": filter_name, "shape": list(array.shape), "sum": float(array.sum())}
                scale = target_scale
            else:
                with fits.open(path) as hdul:
                    array = np.asarray(hdul[0].data, dtype=float)
                used = path
            fwhm, entry = self.call(record, "lightstack.psf.measure_psf_fwhm", psf.measure_psf_fwhm, {"psf": array}, [used], tolerate=IndexError)
            entry["metrics"] = {"filter": filter_name, "fwhm_px": None if fwhm is None else float(fwhm), "pixel_scale_arcsec": float(scale)}
            self.psfs[filter_name] = {"array": array, "path": used}

    def step_kernels(self, step, directory, record):
        from astropy.io import fits
        from lightstack.psf import build_kernel, save_kernel

        reference = step["reference"]
        target = self.psfs[reference]
        for filter_name, source in self.psfs.items():
            if filter_name == reference:
                continue
            inputs = [source["path"], target["path"]]
            if step["method"] == "wiener":
                shape = target["array"].shape if step.get("shape", "reference") == "reference" else tuple(step["shape"])
                arguments = {"psf_source": source["array"], "psf_ref": target["array"], "shape": shape, "eps": step.get("eps", 1e-3)}
                kernel, entry = self.call(record, "lightstack.psf.build_kernel", build_kernel, arguments, inputs)
            else:
                from photutils.psf import matching

                window_class, names = WINDOWS[step["window"]]
                window = getattr(matching, window_class)(**{name: step[name] for name in names if name in step})
                arguments = {"source_psf": source["array"], "target_psf": target["array"], "window": window}
                kernel, entry = self.call(record, "photutils.psf.matching.create_matching_kernel", matching.create_matching_kernel, arguments, inputs)
                kernel = kernel / kernel.sum()
            entry["metrics"] = {
                "filter": filter_name,
                "shape": list(kernel.shape),
                "sum": float(kernel.sum()),
                "min": float(kernel.min()),
                "max": float(kernel.max()),
                "negative_fraction": float((kernel < 0).mean()),
            }
            output = directory / f"kernel_{filter_name.lower()}_to_{reference.lower()}.fits"
            header = fits.Header()
            header["SRCPSF"] = f"{self.datasets[filter_name]}_{filter_name}"
            header["TGTPSF"] = f"{self.datasets[reference]}_{reference}"
            _, saved = self.call(record, "lightstack.psf.save_kernel", save_kernel, {"kernel": kernel, "output_path": str(output), "header": header}, [])
            saved["outputs"] = [self.entry(output)]
        self.kernel_dir = directory

    def step_match(self, step, directory, record):
        from lightstack.psf import psf_match_datacube

        output = directory / f"{self.cube.stem}_psfmatched.fits"
        kernels = sorted(self.kernel_dir.glob("kernel_*.fits"))
        arguments = {"cube_path": str(self.cube), "kernel_dir": str(self.kernel_dir), "ref_filter": step["reference"], "output_path": str(output), "overwrite": True}
        _, entry = self.call(record, "lightstack.psf.psf_match_datacube", psf_match_datacube, arguments, [self.cube] + kernels)
        entry["outputs"] = [self.entry(output)]
        unmatched = re.findall(r"Kernel not found for (\S+), skipping", entry["stdout"]) + re.findall(r"No (FILTER\d+) keyword", entry["stdout"])
        entry["metrics"] = {"unmatched": unmatched}
        self.cube = output

    def step_export(self, step, directory, record):
        output = directory / step["output"]
        shutil.copyfile(self.cube, output)
        record["notes"].append(f"copied {self.cube.name} to {output.name}")
        self.cube = output


def input_files(recipe, data_dir):
    paths = [item["path"] for item in recipe["inputs"]]
    for step in recipe["steps"]:
        if step["step"] == "psfs" and step["source"] == "files":
            paths += [item["path"] for item in step["files"]]
        if step["step"] == "crop" and "region_file" in step:
            paths.append(step["region_file"])
    return [file_entry(data_dir / path, data_dir) for path in dict.fromkeys(paths)]


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="lightstack_oracle",
        description="Run a cube-preparation recipe with LIGHTSTACK and write every intermediate FITS file and a manifest.json.",
    )
    parser.add_argument("--recipe", required=True, type=Path, help="recipe JSON (the AstroBurst cube-preparation recipe format)")
    parser.add_argument("--data", required=True, type=Path, help="folder the recipe paths are relative to")
    parser.add_argument("--out", type=Path, help="empty or missing folder for the goldens and manifest.json")
    parser.add_argument("--check", action="store_true", help="validate the recipe and print the LIGHTSTACK calls, without importing LIGHTSTACK")
    args = parser.parse_args(argv)
    try:
        recipe_bytes = args.recipe.read_bytes()
        content = json.loads(recipe_bytes)
        recipe = lightstack_form(content)
        data_dir = args.data.resolve()
        validate(recipe, data_dir)
        if args.check:
            print("\n".join(plan(recipe)))
            return 0
        require(args.out is not None, "--out is required unless --check is given")
        require(not args.out.exists() or not any(args.out.iterdir()), f"{args.out} is not empty")
        args.out.mkdir(parents=True, exist_ok=True)
        os.environ.setdefault("MPLBACKEND", "Agg")
        started = time.perf_counter()
        oracle = Oracle(recipe, data_dir, args.out)
        oracle.run()
        out_dir = args.out.resolve()
        manifest = {
            "manifest_version": MANIFEST_VERSION,
            "kind": "lightstack_oracle",
            "created_by": CREATED_BY,
            "recipe": {"file": args.recipe.name, "sha256": hashlib.sha256(recipe_bytes).hexdigest(), "content": content},
            "versions": versions(),
            "lightstack_source": lightstack_source(importlib.metadata.distribution("lightstack"), Path(importlib.util.find_spec("lightstack").submodule_search_locations[0])),
            "platform": platform.platform(),
            "inputs": input_files(recipe, data_dir),
            "steps": oracle.steps,
            "files": [file_entry(path, out_dir) for path in sorted(out_dir.rglob("*")) if path.is_file() and path.name != "manifest.json"],
            "wall_time_s": time.perf_counter() - started,
        }
        (out_dir / "manifest.json").write_text(json.dumps(finite(manifest), indent=2, allow_nan=False) + "\n", encoding="utf-8", newline="\n")
        print(f"manifest: {out_dir / 'manifest.json'}")
        return 0
    except RecipeError as error:
        print(f"recipe error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
