import base64
import contextlib
import hashlib
import importlib.metadata
import io
import json
import subprocess
import tempfile
import unittest
import warnings
from pathlib import Path

import lightstack_oracle as oracle

FIXTURES = Path(__file__).resolve().parents[2] / "src-tauri" / "tests" / "fixtures"
D1 = FIXTURES / "lightstack" / "d1"
D1_STARS = FIXTURES / "lightstack" / "d1_stars"
SAVED_D1 = FIXTURES / "cubeprep" / "recipe_d1_saved.json"
PACKAGE_CODE = {"__init__.py": b"from .crop import crop\r\n", "crop.py": b"def crop():\r\n    return 1\r\n"}
ALIGN_STDOUT = "".join(
    [
        "\nProcessing JWST / F150W\nUnit: Jy\nPixel scale in/out: 0.0200 -> 0.0400\n",
        "Applied area correction: 4.0000\nFlux ratio (out/in): 0.9991\nSaved: a_aligned_interp.fits\n",
        "\nProcessing JWST / F277W\nUnit: Jy\nPixel scale in/out: 0.0400 -> 0.0400\n",
        "Already aligned with reference: skipping reprojection.\nSaved: b_aligned_interp.fits\n",
        "\nProcessing JWST / F356W\nUnit: Jy\nPixel scale in/out: 0.0400 -> 0.0400\n",
        "Pixel scales identical: no area correction applied.\nFlux ratio (out/in): 1.0002\nSaved: c_aligned_interp.fits\n",
    ]
)


def d1_recipe():
    return json.loads((D1 / "recipe_d1.json").read_text(encoding="utf-8"))


def saved_d1_recipe():
    return json.loads(SAVED_D1.read_text(encoding="utf-8"))


def d1_stars_recipe():
    return json.loads((D1_STARS / "recipe_d1_stars.json").read_text(encoding="utf-8"))


def saved_d1_stars_recipe():
    recipe = d1_stars_recipe()
    step = recipe["steps"][0]
    recipe["steps"][0] = {
        "step": "psfs",
        "source": "empirical",
        "dataset": step["dataset"],
        "filters": step["filters"],
        "data_dir": None,
        "oversampling_order": None,
        "cutout_size": step["cutout_size"],
        "background_border": step["background_border"],
        "background_sigma": 4.0,
        "clip_negative": True,
        "noise_threshold": 0.5,
        "radial_cleaning": False,
        "cleaning_sigma": 1.0,
        "cleaning_consecutive_bins": 3,
        "normalization_radius": step["normalization_radius"],
        "normalization_radius_unit": step["normalization_radius_unit"],
        "psf_size": step["psf_size"],
        "oversampling": None,
        "stack": "median",
        "reference": None,
        "pixel_scale_arcsec": None,
    }
    return recipe


def run_main(arguments):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = oracle.main(arguments)
    return code, out.getvalue(), err.getvalue()


def record_hash(data):
    return "sha256=" + base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode("ascii")


def fake_install(site, direct_url=None, changed=()):
    package = site / "lightstack"
    package.mkdir(parents=True)
    info = site / "lightstack-0.4.0.dist-info"
    info.mkdir()
    rows = []
    for name, data in PACKAGE_CODE.items():
        (package / name).write_bytes(data + b"# edited\n" if name in changed else data)
        rows.append(f"lightstack/{name},{record_hash(data)},{len(data)}")
    (info / "METADATA").write_text("Metadata-Version: 2.1\nName: lightstack\nVersion: 0.4.0\n", encoding="utf-8")
    rows.append(f"lightstack-0.4.0.dist-info/METADATA,{record_hash((info / 'METADATA').read_bytes())},{(info / 'METADATA').stat().st_size}")
    rows.append("lightstack-0.4.0.dist-info/RECORD,,")
    if direct_url is not None:
        (info / "direct_url.json").write_text(json.dumps(direct_url), encoding="utf-8")
    (info / "RECORD").write_text("\n".join(rows) + "\n", encoding="utf-8")
    return importlib.metadata.PathDistribution(info), package


def expected_code_sha256():
    lines = sorted(f"lightstack/{name}\0{hashlib.sha256(data.replace(b'\r\n', b'\n')).hexdigest()}\n" for name, data in PACKAGE_CODE.items())
    return hashlib.sha256("".join(lines).encode("utf-8")).hexdigest()


class RecipeTest(unittest.TestCase):
    def assert_refused(self, recipe, fragment):
        with self.assertRaises(oracle.RecipeError) as raised:
            oracle.validate(recipe, D1)
        self.assertIn(fragment, str(raised.exception))

    def test_d1_recipe_is_valid_and_plans_the_lightstack_calls(self):
        recipe = d1_recipe()
        oracle.validate(recipe, D1)
        lines = oracle.plan(recipe)
        self.assertEqual(lines[1], "01_crop: crop_from_radec")
        self.assertEqual(lines[3], "03_align: align_reproject_fits")
        self.assertEqual(lines[8], "08_match: psf_match_datacube")

    def test_unknown_step_is_refused(self):
        recipe = d1_recipe()
        recipe["steps"].append({"step": "stack"})
        self.assert_refused(recipe, "unknown step")

    def test_parameter_lightstack_lacks_is_refused(self):
        recipe = d1_recipe()
        recipe["steps"][2]["area_rule"] = "scale"
        self.assert_refused(recipe, "area_rule")

    def test_bicubic_alignment_is_refused(self):
        recipe = d1_recipe()
        recipe["steps"][2]["method"] = "bicubic"
        self.assert_refused(recipe, "method must be one of")

    def test_missing_input_is_refused(self):
        recipe = d1_recipe()
        recipe["inputs"][0]["path"] = "absent.fits"
        self.assert_refused(recipe, "absent.fits")

    def test_match_without_kernels_is_refused(self):
        recipe = d1_recipe()
        recipe["steps"] = [step for step in recipe["steps"] if step["step"] != "kernels"]
        self.assert_refused(recipe, "no kernels to F444W")


class OutputTest(unittest.TestCase):
    def test_align_output_gives_area_factor_skip_and_flux_ratio(self):
        planes = oracle.parse_align(ALIGN_STDOUT)
        self.assertEqual(planes["F150W"]["area_correction"], 4.0)
        self.assertEqual(planes["F150W"]["flux_ratio_printed"], 0.9991)
        self.assertFalse(planes["F277W"]["reprojected"])
        self.assertIsNone(planes["F277W"]["flux_ratio_printed"])
        self.assertTrue(planes["F356W"]["reprojected"])
        self.assertIsNone(planes["F356W"]["area_correction"])

    def test_non_finite_numbers_become_null(self):
        self.assertEqual(oracle.finite({"radius": [float("nan"), 1.5], "ratio": float("inf")}), {"radius": [None, 1.5], "ratio": None})


class CallTest(unittest.TestCase):
    def setUp(self):
        self.oracle = oracle.Oracle(d1_recipe(), D1, D1)
        self.record = {"calls": []}

    def test_a_tolerated_failure_is_recorded_and_the_run_goes_on(self):
        def fit(psf):
            raise IndexError("index 21 is out of bounds for axis 0 with size 21")

        result, entry = self.oracle.call(self.record, "lightstack.psf.measure_psf_fwhm", fit, {"psf": None}, [], tolerate=IndexError)
        self.assertIsNone(result)
        self.assertEqual(entry["error"], "IndexError: index 21 is out of bounds for axis 0 with size 21")
        self.assertIs(self.record["calls"][0], entry)

    def test_a_failure_that_is_not_tolerated_stops_the_run(self):
        def fit(psf):
            raise ValueError("bad psf")

        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(ValueError):
            self.oracle.call(self.record, "lightstack.psf.measure_psf_fwhm", fit, {"psf": None}, [], tolerate=IndexError)

    def test_a_failure_that_is_not_tolerated_shows_what_the_call_printed_and_warned(self):
        def fit(psf):
            print("fitting 3 stars")
            warnings.warn("All-NaN slice encountered", RuntimeWarning)
            raise ValueError("bad psf")

        report = io.StringIO()
        with contextlib.redirect_stderr(report), self.assertRaises(ValueError):
            self.oracle.call(self.record, "lightstack.psf.measure_psf_fwhm", fit, {"psf": None}, [], tolerate=IndexError)
        self.assertIn("fitting 3 stars", report.getvalue())
        self.assertIn("RuntimeWarning: All-NaN slice encountered", report.getvalue())


class EffectiveArgumentsTest(unittest.TestCase):
    def build(self, data_dir, dataset, cutout_size=61, oversampling=None, show_figures=True):
        return None

    def test_every_default_the_call_leaves_out_is_recorded(self):
        arguments = oracle.effective_arguments(self.build, {"data_dir": "d", "dataset": "jwst", "show_figures": False})
        self.assertEqual(arguments, {"data_dir": "d", "dataset": "jwst", "cutout_size": 61, "oversampling": None, "show_figures": False})

    def build_with_options(self, data_dir, cutout_size=61, *extra, **options):
        return None

    def test_variadic_parameters_pass_through_as_plain_keywords(self):
        arguments = oracle.effective_arguments(self.build_with_options, {"data_dir": "d", "flag": True})
        self.assertEqual(arguments, {"data_dir": "d", "cutout_size": 61, "flag": True})

    def test_an_argument_the_function_lacks_is_refused(self):
        with self.assertRaises(TypeError):
            oracle.effective_arguments(self.build, {"data_dir": "d", "dataset": "jwst", "cutout": 31})


class LightstackFormTest(unittest.TestCase):
    def assert_refused(self, recipe, fragment):
        with self.assertRaises(oracle.RecipeError) as raised:
            oracle.lightstack_form(recipe)
        self.assertIn(fragment, str(raised.exception))

    def test_the_saved_d1_recipe_reads_as_the_lightstack_recipe(self):
        self.assertEqual(oracle.lightstack_form(saved_d1_recipe()), d1_recipe())

    def test_the_saved_star_recipe_loses_its_nulls_and_its_median_stack(self):
        form = oracle.lightstack_form(saved_d1_stars_recipe())
        step = form["steps"][0]
        for key in ("data_dir", "oversampling_order", "oversampling", "stack", "reference", "pixel_scale_arcsec"):
            self.assertNotIn(key, step)
        self.assertEqual(step["noise_threshold"], 0.5)
        oracle.validate(form, D1_STARS)

    def test_a_null_name_is_dropped(self):
        recipe = d1_recipe()
        recipe["name"] = None
        self.assertNotIn("name", oracle.lightstack_form(recipe))

    def test_a_null_step_key_is_dropped(self):
        recipe = d1_recipe()
        recipe["steps"][3]["reference"] = None
        self.assertNotIn("reference", oracle.lightstack_form(recipe)["steps"][3])

    def test_an_astroburst_value_is_refused_with_its_key(self):
        cases = [
            (0, "rounding", "astroburst", "rounding=astroburst"),
            (2, "area_rule", "scale", "area_rule=scale"),
            (3, "layout", "mef", "layout=mef"),
            (4, "use_dq", True, "use_dq=true"),
            (6, "eps_mode", "relative", "eps_mode=relative"),
            (6, "auto", True, "auto=true"),
            (7, "mode", "astroburst", "mode=astroburst"),
            (7, "allow_unmatched", True, "allow_unmatched=true"),
        ]
        for index, key, value, shown in cases:
            recipe = d1_recipe()
            recipe["steps"][index][key] = value
            with self.subTest(key=key):
                self.assert_refused(recipe, f"steps[{index}] ({recipe['steps'][index]['step']}): AstroBurst-only value {shown}; LIGHTSTACK has no equivalent")

    def test_the_crop_mode_is_a_lightstack_key(self):
        self.assertEqual(oracle.lightstack_form(d1_recipe())["steps"][0]["mode"], "radec")

    def test_the_reference_nan_mask_mode_is_dropped_only_at_reference(self):
        recipe = d1_recipe()
        recipe["steps"].insert(5, {"step": "edit", "action": "reference_nan_mask", "reference_slice": 0, "mode": "reference"})
        self.assertEqual(oracle.lightstack_form(recipe)["steps"][5], {"step": "edit", "action": "reference_nan_mask", "reference_slice": 0})
        recipe["steps"][5]["mode"] = "union"
        self.assert_refused(recipe, "AstroBurst-only value mode=union")

    def test_a_window_regularization_is_dropped_only_at_the_photutils_default(self):
        recipe = d1_recipe()
        recipe["steps"][6] = {"step": "kernels", "reference": "F444W", "method": "window", "window": "hanning", "regularization": 1e-4, "auto": False}
        self.assertEqual(oracle.lightstack_form(recipe)["steps"][6], {"step": "kernels", "reference": "F444W", "method": "window", "window": "hanning"})
        recipe["steps"][6]["regularization"] = 1e-3
        self.assert_refused(recipe, "AstroBurst-only value regularization=0.001")

    def test_recentre_is_dropped_only_when_off(self):
        recipe = d1_recipe()
        recipe["steps"][5]["recentre"] = False
        self.assertNotIn("recentre", oracle.lightstack_form(recipe)["steps"][5])
        recipe["steps"][5]["recentre"] = True
        self.assert_refused(recipe, "AstroBurst-only value recentre=true")

    def test_check_accepts_the_saved_forms(self):
        with tempfile.TemporaryDirectory() as scratch:
            stars = Path(scratch) / "recipe.json"
            stars.write_text(json.dumps(saved_d1_stars_recipe(), indent=2), encoding="utf-8")
            for recipe, data in ((SAVED_D1, D1), (stars, D1_STARS)):
                with self.subTest(recipe=recipe.name):
                    code, out, err = run_main(["--recipe", str(recipe), "--data", str(data), "--check"])
                    self.assertEqual((code, err), (0, ""))
                    self.assertIn("01_", out)

    def test_check_refuses_the_scientific_profile_naming_the_first_astroburst_key(self):
        recipe = saved_d1_recipe()
        recipe["steps"][0]["rounding"] = "astroburst"
        recipe["steps"][2]["area_rule"] = "scale"
        with tempfile.TemporaryDirectory() as scratch:
            path = Path(scratch) / "recipe.json"
            path.write_text(json.dumps(recipe, indent=2), encoding="utf-8")
            code, _, err = run_main(["--recipe", str(path), "--data", str(D1), "--check"])
        self.assertEqual(code, 2)
        self.assertIn("rounding=astroburst", err)


class JsonableTest(unittest.TestCase):
    def test_a_zero_dimensional_value_is_recorded_as_its_number(self):
        class Scalar:
            shape = ()
            dtype = "int64"
            ndim = 0

            def item(self):
                return 11

        class Array:
            shape = (21, 21)
            dtype = "float64"
            ndim = 2

        jsonable = oracle.Oracle(d1_recipe(), D1, D1).jsonable
        self.assertEqual(jsonable({"s1": Scalar()}), {"s1": 11})
        self.assertEqual(jsonable(Array()), "<array float64 [21, 21]>")


class LightstackSourceTest(unittest.TestCase):
    def test_an_index_install_records_the_code_fingerprint_and_no_modified_file(self):
        with tempfile.TemporaryDirectory() as scratch:
            distribution, package = fake_install(Path(scratch))
            source = oracle.lightstack_source(distribution, package)
        self.assertEqual(source, {"kind": "index", "code_sha256": expected_code_sha256(), "modified_since_install": []})

    def test_the_fingerprint_hashes_site_relative_paths_with_lf_line_ends(self):
        with tempfile.TemporaryDirectory() as scratch:
            package = Path(scratch) / "lightstack"
            package.mkdir()
            for name, data in PACKAGE_CODE.items():
                (package / name).write_bytes(data.replace(b"\r\n", b"\n"))
            self.assertEqual(oracle.code_sha256(package), expected_code_sha256())

    def test_a_vcs_install_keeps_its_commit_and_requested_revision(self):
        vcs = {"vcs": "git", "commit_id": "0123456789abcdef0123456789abcdef01234567", "requested_revision": "v0.4.0"}
        with tempfile.TemporaryDirectory() as scratch:
            distribution, package = fake_install(Path(scratch), {"url": "https://github.com/example/lightstack.git", "vcs_info": vcs})
            source = oracle.lightstack_source(distribution, package)
        self.assertEqual(source["kind"], "direct")
        self.assertEqual(source["url"], "https://github.com/example/lightstack.git")
        self.assertEqual(source["vcs_info"], vcs)
        self.assertEqual(source["modified_since_install"], [])

    def test_an_archive_install_keeps_its_hashes(self):
        archive = {"hashes": {"sha256": "ab" * 32}}
        with tempfile.TemporaryDirectory() as scratch:
            distribution, package = fake_install(Path(scratch), {"url": "https://files.example/lightstack-0.4.0.tar.gz", "archive_info": archive})
            source = oracle.lightstack_source(distribution, package)
        self.assertEqual(source["archive_info"], archive)
        self.assertNotIn("dirty", source)

    def test_an_editable_checkout_with_local_changes_is_dirty_and_lists_the_edited_file(self):
        with tempfile.TemporaryDirectory() as scratch:
            checkout = Path(scratch) / "checkout"
            checkout.mkdir()
            subprocess.run(["git", "init", "-q", str(checkout)], check=True)
            (checkout / "psf.py").write_text("x = 1\n", encoding="utf-8")
            distribution, package = fake_install(Path(scratch) / "site", {"url": checkout.as_uri(), "dir_info": {"editable": True}}, changed=("crop.py",))
            source = oracle.lightstack_source(distribution, package)
        self.assertEqual(source["dir_info"], {"editable": True})
        self.assertIs(source["dirty"], True)
        self.assertEqual(source["modified_since_install"], ["lightstack/crop.py"])
        self.assertNotEqual(source["code_sha256"], expected_code_sha256())

    def test_an_install_without_a_record_cannot_be_checked(self):
        with tempfile.TemporaryDirectory() as scratch:
            distribution, package = fake_install(Path(scratch))
            (Path(scratch) / "lightstack-0.4.0.dist-info" / "RECORD").unlink()
            self.assertIsNone(oracle.lightstack_source(distribution, package)["modified_since_install"])


if __name__ == "__main__":
    unittest.main()
