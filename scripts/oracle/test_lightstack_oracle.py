import contextlib
import io
import json
import unittest
import warnings
from pathlib import Path

import lightstack_oracle as oracle

D1 = Path(__file__).resolve().parents[2] / "src-tauri" / "tests" / "fixtures" / "lightstack" / "d1"
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


if __name__ == "__main__":
    unittest.main()
