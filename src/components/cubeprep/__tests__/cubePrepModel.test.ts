import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { addRecipeStep, afterInputChange, applyGoal, applyProfile, broadestBand, changeRecipeInput, closestProfile, commonReference, createCubePrepDraft, cubePrepError, cubePrepProblems, cubePrepPsfTarget, cubePrepStageText, cubePrepSummary, defaultStep, importedDraft, inputDirectory, kernelProfileNote, parseRecipe, profileDifferences, recipeForSave, recipeProblemList, recipeProblems, removeRecipeStepAt, replaceRecipeStepAt, setCommonReference, setRecipeStep, withDefaultReference, workingDirectory } from "../cubePrepModel";
import { normalizeTauriError } from "../../../shared/types/errors";
import type { CubePrepFilter, CubePrepRecipe, CubePrepReport, CubePrepStep } from "../../../shared/types/cubeprep";

const TWO_INPUTS = [{ path: "a.fits", dataset: "JWST", filter: "F200W" }, { path: "b.fits", dataset: "JWST", filter: "F444W" }];
const withSteps = (steps: CubePrepStep[]): CubePrepRecipe => ({ recipe_version: 1, name: "field", inputs: TWO_INPUTS, steps });
const PSFS: CubePrepStep = { step: "psfs", source: "files", files: [{ filter: "F200W", path: "p200.fits" }, { filter: "F444W", path: "p444.fits" }] };
const KERNELS: CubePrepStep = { step: "kernels", reference: "F444W", method: "wiener", eps: 0.001 };

const kernelStep = (matched: Record<string, number | null>, target = 3.471) => ({ step: "kernels", report: { reference: "F444W", pixel_scale_arcsec: 0.04, kernels: Object.entries(matched).map(([filter, matchedPx]) => ({ filter, diagnostics: { source_fwhm_px: 2.5, target_fwhm_px: target, matched_fwhm_px: matchedPx } })) } });
const matchStep = (filters: string[]) => ({ step: "match", parameters: { reference: "F444W" }, report: { reference: "F444W", planes: [...filters.map((filter) => ({ filter, status: "matched" })), { filter: "F444W", status: "reference" }] } });
const D1_EPS_005 = { F814W: 3.842, F150W: 3.512, F200W: 3.538, F277W: 3.626, F356W: 4.21 };
const D1_FILTERS = ["F814W", "F150W", "F200W", "F277W", "F356W", "F444W"];
const D1_SAVED_STEPS: CubePrepStep[] = [
  { step: "crop", mode: "radec", ra: 53.1625, dec: -27.7914, size_arcsec: 1.8, rounding: "astroburst" },
  { step: "units", to: "Jy/pixel" },
  { step: "align", reference: "F444W", method: "bilinear", border_crop: 1, area_rule: "scale" },
  { step: "cube", reference: "F444W", order: "wavelength", output: "d1_cube.fits", layout: "mef" },
  { step: "edit", action: "drop_invalid", threshold: 0, frac_valid: 0.01, use_dq: true },
  PSFS,
  { step: "kernels", reference: "F444W", method: "wiener", eps: 0.05, shape: "reference" },
  { step: "match", reference: "F444W", mode: "astroburst" },
];

describe("Cube Prep recipes", () => {
  it("offers a common PSF only for complete matching of the current final bands", () => {
    const report = { steps: [kernelStep({ F200W: 3.48 }), matchStep(["F200W"]), { step: "export" }] } as unknown as CubePrepReport;
    expect(cubePrepPsfTarget(report, ["F200W", "F444W"])).toBe("F444W");
    expect(cubePrepPsfTarget(report, ["F200W", "F356W", "F444W"])).toBe("");
    expect(cubePrepPsfTarget({ ...report, steps: [...report.steps, { step: "cube" } as CubePrepReport["steps"][number]] }, ["F200W", "F444W"])).toBe("");
    (report.steps[1].report as { planes: { status: string }[] }).planes[0].status = "unmatched";
    expect(cubePrepPsfTarget(report, ["F200W", "F444W"])).toBe("");
  });

  it("leaves the PSF target blank when a matched FWHM misses the target by more than 2 percent", () => {
    const withKernels = (matched: Record<string, number | null>) => ({ steps: [kernelStep(matched), matchStep(D1_FILTERS.slice(0, 5)), { step: "export" }] }) as unknown as CubePrepReport;
    expect(cubePrepPsfTarget(withKernels(D1_EPS_005), D1_FILTERS)).toBe("");
    expect(cubePrepPsfTarget(withKernels({ ...D1_EPS_005, F814W: 3.48, F277W: 3.5, F356W: 3.47 }), D1_FILTERS)).toBe("F444W");
    expect(cubePrepPsfTarget(withKernels({ ...D1_EPS_005, F814W: 3.48, F277W: 3.5, F356W: null }), D1_FILTERS)).toBe("");
    const { F356W: _dropped, ...withoutF356W } = { ...D1_EPS_005, F814W: 3.48, F277W: 3.5 };
    expect(cubePrepPsfTarget(withKernels(withoutF356W), D1_FILTERS)).toBe("");
  });
  it("keeps saved recipes portable when saved outside their original base folder", () => {
    const recipe = createCubePrepDraft().recipe;
    recipe.inputs = [{ path: "image.fits", dataset: "JWST", filter: "F200W" }];
    recipe.steps = [{ step: "crop", mode: "region", region_file: "stars.reg" }, { step: "psfs", source: "files", files: [{ filter: "F200W", path: "psf.fits" }] }, { step: "export", output: "final.fits" }];
    const saved = recipeForSave(recipe, "/original");
    expect(saved.inputs[0].path).toBe("/original/image.fits");
    expect(saved.steps[0].region_file).toBe("/original/stars.reg");
    expect(saved.steps[1].files).toEqual([{ filter: "F200W", path: "/original/psf.fits" }]);
    expect(saved.steps[2].output).toBe("final.fits");
    expect(recipe.inputs[0].path).toBe("image.fits");
  });

  it("records explicit LIGHTSTACK defaults and preserves step order", () => {
    const lightstack = applyGoal(createCubePrepDraft().recipe, "lightstack");
    expect(lightstack.steps.slice(0, 3)).toEqual([
      { step: "units", to: "Jy/pixel" },
      { step: "align", reference: "", method: "bilinear", border_crop: 1, area_rule: "lightstack" },
      { step: "cube", reference: "", order: "wavelength", output: "cube.fits", layout: "lightstack" },
    ]);
    const recipe = setRecipeStep(lightstack, { step: "crop", mode: "common", rounding: "lightstack" });
    expect(recipe.steps.map((s) => s.step)).toEqual(["crop", "units", "align", "cube", "psfs", "kernels", "match"]);
  });

  it("uses physical area rules and MEF without inventing uncertainty data", () => {
    const recipe = createCubePrepDraft().recipe;
    const scientific = applyProfile(recipe, "scientific");
    expect(scientific.steps.find((s) => s.step === "cube")?.layout).toBe("mef");
    expect(scientific.steps.find((s) => s.step === "align")?.area_rule).toBe("scale");
    const brightness = setRecipeStep(recipe, { step: "units", to: "MJy/sr" });
    expect(applyProfile(brightness, "scientific").steps.find((s) => s.step === "align")?.area_rule).toBe("keep");
  });

  it("catches incomplete inputs, duplicate filters and missing references", () => {
    const recipe = createCubePrepDraft().recipe;
    expect(recipeProblems(recipe).join(" ")).toMatch(/input/i);
    recipe.inputs = [{ path: "a.fits", dataset: "JWST", filter: "F200W" }, { path: "b.fits", dataset: "JWST", filter: "f200w" }];
    expect(recipeProblems(recipe).join(" ")).toMatch(/duplicate/i);
    expect(recipeProblems(recipe).join(" ")).toMatch(/reference/i);
  });

  it("keeps imported recipe steps intact and rejects nonrecipes", () => {
    const recipe = { recipe_version: 1, inputs: [{ path: "a.fits", dataset: "JWST", filter: "F200W" }], steps: [{ step: "edit", action: "cut", x_start: -3, x_end: 20, y_start: 0, y_end: 20 }] };
    expect(parseRecipe(JSON.stringify(recipe))).toEqual(recipe);
    expect(() => parseRecipe('{"recipe_version":2}')).toThrow();
    expect(() => parseRecipe('{"recipe_version":1,"inputs":[],"steps":[{"step":"shell"}]}')).toThrow();
  });

  it("creates separate persistent work folders on Windows and Linux", () => {
    expect(inputDirectory("C:\\data\\image.fits")).toBe("C:\\data");
    expect(inputDirectory("/data/image.fits")).toBe("/data");
    expect(workingDirectory("C:\\data", "D1 / field", "20261102-001")).toBe("C:\\data\\cubeprep-D1-field-20261102-001");
    expect(workingDirectory("/data", "D1", "run-2")).toBe("/data/cubeprep-D1-run-2");
  });

  it("adds the backend hint to the message once (SCIENCE-02)", () => {
    const unknownKey = { code: "unknown_parameter", message: "steps[2] (align): unknown crop", hint: "accepted keys: area_rule, border_crop, method, reference" };
    expect(cubePrepError(normalizeTauriError("validate_cubeprep_recipe_cmd", unknownKey))).toBe("steps[2] (align): unknown crop (accepted keys: area_rule, border_crop, method, reference)");
    const remedy = "use \"shape\": \"max\" or two odd integers";
    const shaped = { code: "kernel_shape", message: `07_kernels: the kernel shape 21x21 must hold both PSFs; ${remedy}`, hint: remedy };
    expect(cubePrepError(normalizeTauriError("run_cubeprep_recipe_cmd", shaped))).toBe(shaped.message);
    expect(cubePrepError({ message: "07_kernels: the kernel shape must hold both PSFs; Use \"shape\":  \"max\" or two odd integers", hint: `${remedy}.` })).not.toContain("(");
    expect(cubePrepError({ message: "inputs[0]: a.fits is not a file", hint: "  " })).toBe("inputs[0]: a.fits is not a file");
    expect(cubePrepError(new Error("Missing ERR"))).toBe("Missing ERR");
  });

  it("asks for a value when a required number was cleared, but not for optional ones (SCIENCE-12)", () => {
    expect(recipeProblems(withSteps([{ step: "align", reference: "F444W", method: "bilinear", border_crop: null, area_rule: "scale" }, { step: "cube", output: "cube.fits" }]))).toEqual(["Align: enter a value for border_crop."]);
    const header: CubePrepStep = { step: "edit", action: "header", redshift: null, ra_center: null, dec_center: null, use_brightest_pixel: false };
    const psfs: CubePrepStep = { ...PSFS, reference: "F444W", pixel_scale_arcsec: null };
    expect(recipeProblems(withSteps([{ step: "align", reference: "F444W" }, { step: "cube", output: "cube.fits" }, header, psfs]))).toEqual([]);
  });

  it("mirrors the backend step dependencies (SCIENCE-15)", () => {
    const align: CubePrepStep = { step: "align", reference: "F444W" };
    const cube: CubePrepStep = { step: "cube", reference: "F444W", output: "cube.fits" };
    expect(recipeProblems(withSteps([{ step: "cube", output: "cube.fits" }]))).toContain("Cube needs a reference filter or an earlier Align step.");
    expect(recipeProblems(withSteps([align, { step: "cube", output: "cube.fits" }]))).toEqual([]);
    expect(recipeProblems(withSteps([align, { step: "cube", reference: "", output: "cube.fits" }]))).toContain("Choose a reference filter for Cube.");
    expect(recipeProblems(withSteps([align, cube, KERNELS]))).toContain("Kernels need a PSF for F444W in an earlier PSFs step.");
    expect(recipeProblems(withSteps([align, cube, PSFS, KERNELS, { step: "match", reference: "F444W" }]))).toEqual([]);
    expect(recipeProblems(withSteps([align, cube, PSFS, { ...KERNELS, reference: "F200W" }, { step: "match", reference: "F444W" }]))).toContain("Match needs an earlier Kernels step to F444W.");
    expect(recipeProblems(withSteps([{ step: "crop", mode: "reference", reference: "", rounding: "lightstack" }, align, cube]))).toContain("Choose a reference filter for Crop.");
    expect(recipeProblems(withSteps([align, { step: "edit", action: "drop_invalid" }, cube]))).toContain("Edit needs an earlier Cube step.");
    expect(recipeProblems(withSteps([align, { step: "export", output: "final.fits" }]))).toContain("Export needs an earlier Cube step.");
  });

  it("flags step references that name no input filter (SCIENCE-16)", () => {
    const problems = recipeProblems(withSteps([
      { step: "align", reference: "F356W" },
      { step: "cube", output: "cube.fits" },
      { step: "edit", action: "remove_filter", filter: "F356W" },
      { step: "psfs", source: "files", files: [{ filter: "F356W", path: "p356.fits" }, { filter: "F444W", path: "p444.fits" }] },
    ]));
    expect(problems).toEqual(["Align: reference F356W is not an input filter.", "Edit: F356W is not an input filter.", "PSFs: F356W is not an input filter."]);
  });

  it("carries step references along when an input filter is renamed (SCIENCE-16)", () => {
    const recipe = withSteps([
      { step: "align", reference: "F444W" },
      { step: "cube", reference: "f444w", output: "cube.fits" },
      { step: "edit", action: "remove_filter", filter: "F444W" },
      { ...PSFS, reference: "F444W" },
      { step: "psfs", source: "empirical", dataset: "JWST", filters: ["F200W", "F444W"], data_dir: "" },
      KERNELS,
      { step: "match", reference: "F200W" },
    ]);
    const renamed = changeRecipeInput(recipe, 1, { filter: "F444W2" });
    expect(renamed.inputs[1]).toEqual({ path: "b.fits", dataset: "JWST", filter: "F444W2" });
    expect(renamed.steps).toEqual([
      { step: "align", reference: "F444W2" },
      { step: "cube", reference: "F444W2", output: "cube.fits" },
      { step: "edit", action: "remove_filter", filter: "F444W2" },
      { step: "psfs", source: "files", reference: "F444W2", files: [{ filter: "F200W", path: "p200.fits" }, { filter: "F444W2", path: "p444.fits" }] },
      { step: "psfs", source: "empirical", dataset: "JWST", filters: ["F200W", "F444W2"], data_dir: "" },
      { ...KERNELS, reference: "F444W2" },
      { step: "match", reference: "F200W" },
    ]);
    expect(changeRecipeInput(recipe, 1, { filter: "" }).steps).toEqual(recipe.steps);
    expect(changeRecipeInput(changeRecipeInput(recipe, 1, { filter: "" }), 1, { filter: "F" }).steps).toEqual(recipe.steps);
    expect(changeRecipeInput(recipe, 1, { dataset: "HST" }).steps).toEqual(recipe.steps);
    const twins = { ...recipe, inputs: [{ ...TWO_INPUTS[1], path: "c.fits" }, TWO_INPUTS[1]] };
    expect(changeRecipeInput(twins, 1, { filter: "F444W2" }).steps).toEqual(recipe.steps);
  });

  it("words progress stages and keeps unknown ones readable (SCIENCE-07)", () => {
    const at = (stage: string, current: number, total: number) => cubePrepStageText({ stage, current, total, percent: 0 });
    expect(at("hashing_inputs", 3, 12)).toBe("Hashing inputs");
    expect(at("01_crop", 0, 3)).toBe("Step 1 of 3: Crop");
    expect(at("07_kernels", 6, 9)).toBe("Step 7 of 9: Kernels");
    expect(at("06_psfs", 5, 9)).toBe("Step 6 of 9: PSFs");
    expect(at("verifying_inputs", 12, 12)).toBe("Verifying inputs");
    expect(at("hashing_products", 4, 30)).toBe("Hashing products");
    expect(at("publishing", 3, 3)).toBe("Publishing");
    expect(at("10_write_noise_maps", 9, 10)).toBe("Step 10 of 10: Write noise maps");
    expect(at("01_toString", 0, 1)).toBe("Step 1 of 1: ToString");
    expect(at("", 0, 1)).toBe("Starting");
  });

  it("defaults the scientific profile to automatic Wiener kernels and exact alignment (OD-L006, OD-L007)", () => {
    const recipe = createCubePrepDraft().recipe;
    expect(defaultStep("kernels", recipe, "scientific")).toMatchObject({ method: "wiener", auto: true });
    expect(defaultStep("kernels", recipe, "lightstack")).toMatchObject({ method: "wiener", auto: false });
    expect(defaultStep("align", recipe, "scientific")).toMatchObject({ method: "exact", area_rule: "scale" });
    expect(defaultStep("align", recipe, "lightstack")).toMatchObject({ method: "bilinear", area_rule: "lightstack" });
    const scientific = applyProfile(recipe, "scientific");
    expect(scientific.steps.find((s) => s.step === "align")?.method).toBe("exact");
    expect(applyProfile(scientific, "lightstack").steps.find((s) => s.step === "align")?.method).toBe("bilinear");
    const imported = setRecipeStep(recipe, { step: "kernels", reference: "F444W", method: "wiener", eps: 0.05, eps_mode: "absolute", shape: "reference", auto: false });
    expect(applyProfile(imported, "scientific").steps.find((s) => s.step === "kernels")).toMatchObject({ eps: 0.05, auto: false });
  });

  it("lists what a saved recipe sets apart from the selected defaults and finds the closer profile", () => {
    const d1 = withSteps(D1_SAVED_STEPS);
    expect(profileDifferences(d1, "scientific")).toEqual(["Align method: bilinear → exact"]);
    expect(profileDifferences(d1, "lightstack")).toEqual(["Crop rounding: astroburst → lightstack", "Align area_rule: scale → lightstack", "Cube layout: mef → lightstack", "Edit use_dq: true → false", "Match mode: astroburst → lightstack"]);
    expect(closestProfile(d1, "lightstack")).toBe("scientific");
    expect(profileDifferences(applyProfile(d1, "scientific"), "scientific")).toEqual([]);
    const fresh = createCubePrepDraft().recipe;
    expect(profileDifferences(fresh, "scientific")).toEqual([]);
    expect(closestProfile(fresh, "lightstack")).toBe("scientific");
    expect(closestProfile(withSteps([{ step: "units", to: "Jy/pixel" }]), "scientific")).toBe("scientific");
  });

  it("names a fixed kernel regularization under the scientific profile and leaves it as saved (OD-L006)", () => {
    const d1 = withSteps(D1_SAVED_STEPS);
    expect(kernelProfileNote(d1, "scientific")).toBe("Kernels keep this recipe's fixed eps 0.05; new Kernels steps in this profile choose it automatically. Turn on the automatic choice in Kernels if the matched FWHM misses the target.");
    expect(applyProfile(d1, "scientific").steps.find((s) => s.step === "kernels")).toMatchObject({ eps: 0.05 });
    expect(kernelProfileNote(d1, "lightstack")).toBeNull();
    expect(kernelProfileNote(setRecipeStep(d1, { ...KERNELS, auto: true }), "scientific")).toBeNull();
    expect(kernelProfileNote(setRecipeStep(d1, { step: "kernels", reference: "F444W", method: "window", window: "hanning", regularization: 0.0001, auto: false }), "scientific")).toContain("fixed regularization 0.0001;");
    expect(kernelProfileNote(withSteps([{ step: "units", to: "Jy/pixel" }]), "scientific")).toBeNull();
  });

  it("imports a recipe with the closer profile, beside its own folder", () => {
    const d1 = withSteps(D1_SAVED_STEPS);
    const imported = importedDraft(createCubePrepDraft(), "C:\\recipes\\d1.json", JSON.stringify(d1));
    expect(imported).toMatchObject({ profile: "scientific", baseDir: "C:\\recipes", outputParent: "C:\\recipes" });
    expect(imported.recipe).toEqual(d1);
    expect(importedDraft({ ...createCubePrepDraft(), outputParent: "D:\\runs" }, "C:\\recipes\\d1.json", JSON.stringify(d1)).outputParent).toBe("D:\\runs");
    expect(() => importedDraft(createCubePrepDraft(), "C:\\recipes\\x.json", '{"recipe_version":2}')).toThrow();
  });

  it("rounds a sky crop edit like the profile's crop (OD-L029)", () => {
    const sky = setRecipeStep(createCubePrepDraft().recipe, { step: "edit", action: "cut_sky", ra: 53.16, dec: -27.79, size_arcsec: 1.7, rounding: "astroburst" });
    expect(applyProfile(sky, "lightstack").steps.find((s) => s.step === "edit")?.rounding).toBe("lightstack");
    expect(applyProfile(applyProfile(sky, "lightstack"), "scientific").steps.find((s) => s.step === "edit")?.rounding).toBe("astroburst");
    const pixels = setRecipeStep(createCubePrepDraft().recipe, { step: "edit", action: "cut", x_start: 0, x_end: 2, y_start: 0, y_end: 2 });
    expect(applyProfile(pixels, "lightstack").steps.find((s) => s.step === "edit")).not.toHaveProperty("rounding");
  });
});

const REGISTRY: CubePrepFilter[] = ([["F444W", 44043], ["F200W", 19886], ["F150W", 15007]] as const).map(([filter, pivot_angstrom]) => ({ dataset: "JWST", filter, instrument: "NIRCam", group: "wide", pivot_angstrom, aliases: [] }));
const THREE_INPUTS = [{ path: "a.fits", dataset: "JWST", filter: "F444W" }, { path: "b.fits", dataset: "jwst", filter: "F200W" }, { path: "c.fits", dataset: "JWST", filter: "F150W" }];
const D1_FIXTURE = () => parseRecipe(readFileSync(fileURLToPath(new URL("../../../../src-tauri/tests/fixtures/lightstack/d1/recipe_d1.json", import.meta.url)), "utf8"));
const stepReport = (step: string, report: unknown, parameters: unknown = {}) => ({ step, parameters, report });
const cubeStep = (filters: [string, number][]) => stepReport("cube", { cols: 43, rows: 41, layout: "mef", hdus: ["SCI", "ERR", "DQ", "FILTERS"], planes: filters.map(([filter, pivot_um]) => ({ filter, pivot_um })) });
const D1_PIVOTS: [string, number][] = [["F814W", 0.8046], ["F150W", 1.5007], ["F200W", 1.9886], ["F277W", 2.7617], ["F356W", 3.5684], ["F444W", 4.4043]];

describe("Cube Prep recipe fixes (PREPUI)", () => {
  it("changes only the pixel-area rule when the output unit changes under the scientific profile (PREPUI-01)", () => {
    const align: CubePrepStep = { step: "align", reference: "F444W", method: "bicubic", border_crop: 1, area_rule: "scale" };
    const cube: CubePrepStep = { step: "cube", reference: "F444W", output: "cube.fits", layout: "lightstack" };
    const match: CubePrepStep = { step: "match", reference: "F444W", mode: "lightstack" };
    const recipe = withSteps([{ step: "units", to: "Jy/pixel" }, align, cube, match]);
    expect(replaceRecipeStepAt(recipe, 0, { step: "units", to: "MJy/sr" }, "scientific").steps).toEqual([{ step: "units", to: "MJy/sr" }, { ...align, area_rule: "keep" }, cube, match]);
    const kept = withSteps([{ step: "units", to: "Jy/pixel" }, { ...align, area_rule: "lightstack" }]);
    expect(replaceRecipeStepAt(kept, 0, { step: "units", to: "MJy/sr" }, "scientific").steps[1]).toEqual({ ...align, area_rule: "lightstack" });
    expect(replaceRecipeStepAt(recipe, 0, { step: "units", to: "MJy/sr" }, "lightstack").steps[1]).toEqual(align);
  });

  it("compares an unset key with the value the backend fills in, so the LIGHTSTACK D1 recipe matches its profile (PREPUI-03)", () => {
    const d1 = D1_FIXTURE();
    expect(profileDifferences(d1, "lightstack")).toEqual([]);
    expect(profileDifferences(d1, "scientific")).toEqual(["Crop rounding: unset (lightstack) → astroburst", "Align method: bilinear → exact", "Align area_rule: unset (lightstack) → scale", "Cube layout: unset (lightstack) → mef", "Edit use_dq: unset (false) → true", "Match mode: unset (lightstack) → astroburst"]);
    expect(closestProfile(d1, "scientific")).toBe("lightstack");
    expect(profileDifferences(withSteps([{ step: "edit", action: "cut_sky", ra: 1, dec: 2, size_arcsec: 3 }]), "scientific")).toEqual([]);
  });

  it("keeps the verified target when only PSF-neutral edits follow Match (PREPUI-05)", () => {
    const after = (action: string) => ({ steps: [kernelStep({ F200W: 3.48 }), matchStep(["F200W"]), stepReport("edit", {}, { step: "edit", action }), { step: "export" }] }) as unknown as CubePrepReport;
    for (const action of ["cut", "cut_sky", "header", "reference_nan_mask"]) expect(cubePrepPsfTarget(after(action), ["F200W", "F444W"])).toBe("F444W");
    expect(cubePrepPsfTarget(after("drop_invalid"), ["F200W", "F444W"])).toBe("");
    expect(cubePrepPsfTarget(after("cut"), ["F444W"])).toBe("");
  });

  it("carries step references through a filter cleared before it is retyped (PREPUI-17)", () => {
    const recipe = withSteps([{ step: "align", reference: "F444W" }, { step: "cube", output: "cube.fits" }, KERNELS]);
    const cleared = changeRecipeInput(recipe, 1, { filter: "" });
    expect(changeRecipeInput(cleared, 1, { filter: "F410M" }, "F444W").steps).toEqual([{ step: "align", reference: "F410M" }, { step: "cube", output: "cube.fits" }, { ...KERNELS, reference: "F410M" }]);
    expect(changeRecipeInput(cleared, 1, { filter: "F410M" }).steps).toEqual(recipe.steps);
  });

  it("proposes the band with the broadest PSF as the reference (PREPUI-19)", () => {
    const recipe = { ...withSteps([]), inputs: THREE_INPUTS };
    expect(defaultStep("kernels", recipe, "scientific", REGISTRY).reference).toBe("F444W");
    expect(broadestBand(THREE_INPUTS, REGISTRY)).toBe("F444W");
    expect(broadestBand(THREE_INPUTS, REGISTRY, { "a.fits": 0.15, "b.fits": 0.07, "c.fits": 0.41 })).toBe("F150W");
    expect(broadestBand(THREE_INPUTS, REGISTRY, { "a.fits": 0.15, "b.fits": null, "c.fits": 0.41 })).toBe("F444W");
    expect(broadestBand(THREE_INPUTS, [])).toBe("");
    expect(defaultStep("match", recipe, "scientific").reference).toBe("F150W");
    expect(defaultStep("match", setCommonReference({ ...createCubePrepDraft().recipe, inputs: THREE_INPUTS }, "F200W"), "scientific", REGISTRY).reference).toBe("F200W");
  });

  it("edits, adds and removes one instance of a repeated step (PREPUI-02)", () => {
    const cube: CubePrepStep = { step: "cube", reference: "F444W", output: "cube.fits" };
    const drop: CubePrepStep = { step: "edit", action: "drop_invalid", threshold: 0, frac_valid: 0.01, use_dq: true };
    const header: CubePrepStep = { step: "edit", action: "header", redshift: 2, ra_center: null, dec_center: null, use_brightest_pixel: false };
    const recipe = withSteps([cube, drop, header]);
    expect(removeRecipeStepAt(recipe, 2).steps).toEqual([cube, drop]);
    expect(replaceRecipeStepAt(recipe, 2, { ...header, redshift: 3 }).steps).toEqual([cube, drop, { ...header, redshift: 3 }]);
    const cut: CubePrepStep = { step: "edit", action: "cut", x_start: 0, x_end: 2, y_start: 0, y_end: 2 };
    expect(addRecipeStep(recipe, cut).steps).toEqual([cube, drop, header, cut]);
    expect(addRecipeStep(withSteps([cube, KERNELS]), header).steps).toEqual([cube, header, KERNELS]);
  });

  it("sets one reference band for every step that needs one and tells a mixed recipe apart (proposal 2)", () => {
    const recipe = { ...createCubePrepDraft().recipe, inputs: THREE_INPUTS };
    expect(commonReference(recipe)).toBe("");
    const set = setCommonReference(recipe, "F444W");
    expect(set.steps.map((step) => step.reference)).toEqual([undefined, "F444W", "F444W", "F444W", "F444W", "F444W"]);
    expect(commonReference(set)).toBe("F444W");
    expect(commonReference(replaceRecipeStepAt(set, 4, { ...set.steps[4], reference: "F200W" }))).toBeNull();
    expect(setCommonReference(withSteps([{ step: "crop", mode: "common", rounding: "astroburst" }, { step: "cube", output: "cube.fits" }]), "F444W").steps).toEqual([{ step: "crop", mode: "common", rounding: "astroburst" }, { step: "cube", output: "cube.fits" }]);
    expect(withDefaultReference(recipe, REGISTRY).steps.filter((step) => "reference" in step).every((step) => step.reference === "F444W")).toBe(true);
    expect(withDefaultReference(set, REGISTRY)).toBe(set);
    expect(withDefaultReference({ ...recipe, inputs: [] }, REGISTRY).steps).toEqual(recipe.steps);
  });

  it("starts a new recipe for regional SEDs with SAGUI and switches goals without dropping steps (proposal 1)", () => {
    const draft = createCubePrepDraft();
    expect(draft.profile).toBe("scientific");
    expect(draft.recipe.steps.map((step) => step.step)).toEqual(["units", "align", "cube", "psfs", "kernels", "match"]);
    expect(draft.recipe.steps.find((step) => step.step === "kernels")).toMatchObject({ method: "wiener", auto: true });
    expect(draft.recipe.steps.find((step) => step.step === "cube")).toMatchObject({ layout: "mef" });
    expect(profileDifferences(draft.recipe, "scientific")).toEqual([]);
    const plain = withSteps([{ step: "align", reference: "F444W", method: "bilinear", border_crop: 1, area_rule: "lightstack" }, { step: "cube", reference: "F444W", output: "cube.fits", layout: "lightstack" }]);
    const sagui = applyGoal(plain, "scientific", REGISTRY);
    expect(sagui.steps.map((step) => step.step)).toEqual(["align", "cube", "psfs", "kernels", "match"]);
    expect(sagui.steps.slice(2).map((step) => step.reference)).toEqual(["F444W", "F444W", "F444W"]);
    expect(sagui.steps[3]).toMatchObject({ auto: true });
    expect(applyGoal(sagui, "lightstack").steps).toEqual(applyProfile(sagui, "lightstack").steps);
  });

  it("summarises the final cube and says why the SAGUI target is or is not set (proposal 3)", () => {
    const matched = { steps: [cubeStep([["F200W", 1.9886], ["F444W", 4.4043]]), kernelStep({ F200W: 3.48 }), matchStep(["F200W"]), stepReport("export", { output_path: "final.fits" })] } as unknown as CubePrepReport;
    expect(cubePrepSummary(matched)).toEqual({ bands: [{ filter: "F200W", pivotUm: 1.9886 }, { filter: "F444W", pivotUm: 4.4043 }], cols: 43, rows: 41, layout: "mef", hdus: ["SCI", "ERR", "DQ", "FILTERS"], psfTarget: "F444W", psfNote: "SAGUI PSF target: F444W, every matched band within 2 % of the target FWHM." });
    const d1 = { steps: [cubeStep(D1_PIVOTS), kernelStep(D1_EPS_005), matchStep(D1_FILTERS.slice(0, 5))] } as unknown as CubePrepReport;
    expect(cubePrepSummary(d1)?.psfNote).toBe("SAGUI PSF target not set: F356W +21.3 %, F814W +10.7 %, F277W +4.5 % from the target FWHM (tolerance 2 %). Turn on the automatic choice in Kernels and run again.");
    expect(cubePrepSummary({ steps: [cubeStep(D1_PIVOTS)] } as unknown as CubePrepReport)?.psfNote).toBe("SAGUI PSF target not set: the recipe does not match PSFs. Enable PSFs, Kernels and Match.");
    const removed = { steps: [...matched.steps.slice(0, 3), stepReport("edit", { filters: ["F444W"], cols: 43, rows: 41, layout: "mef", hdus: ["SCI", "ERR", "DQ", "FILTERS"] }, { step: "edit", action: "remove_filter", filter: "F200W" })] } as unknown as CubePrepReport;
    expect(cubePrepSummary(removed)).toMatchObject({ bands: [{ filter: "F444W", pivotUm: 4.4043 }], psfTarget: "", psfNote: "SAGUI PSF target not set: the final cube does not follow a Match step that covers every band." });
    expect(cubePrepSummary({ steps: [] } as unknown as CubePrepReport)).toBeNull();
  });

  it("gives every input filter a PSF entry to fill and drops empty entries of removed inputs (proposal 9)", () => {
    const psfs: CubePrepStep = { step: "psfs", source: "files", reference: "F444W", files: [{ filter: "F356W", path: "" }, { filter: "F090W", path: "p090.fits" }, { filter: "F444W", path: "p444.fits" }] };
    const synced = afterInputChange(withSteps([psfs, { step: "align", reference: "F444W" }]), REGISTRY);
    expect(synced.steps[0].files).toEqual([{ filter: "F090W", path: "p090.fits" }, { filter: "F444W", path: "p444.fits" }, { filter: "F200W", path: "" }]);
    expect(recipeProblems(synced)).toContain("PSFs: choose the PSF file for F200W.");
    const fresh = afterInputChange({ ...createCubePrepDraft().recipe, inputs: THREE_INPUTS }, REGISTRY);
    expect(fresh.steps.find((step) => step.step === "psfs")).toMatchObject({ reference: "F444W", files: THREE_INPUTS.map((input) => ({ filter: input.filter, path: "" })) });
    expect(fresh.steps.find((step) => step.step === "match")?.reference).toBe("F444W");
  });

  it("files each problem under the tab that fixes it and adds the folder checks the run needs (proposal 5, PREPUI-16)", () => {
    expect(recipeProblemList(withSteps([{ step: "align", reference: "" }, { step: "cube", output: "cube.fits" }]))).toEqual([{ tab: "align", text: "Choose a reference filter for Align." }]);
    expect(recipeProblemList({ ...withSteps([{ step: "align", reference: "F444W" }]), inputs: [{ path: "a.fits", dataset: "", filter: "F444W" }] })).toEqual([{ tab: "inputs", text: "Input 1 needs a file, dataset and filter." }]);
    expect(recipeProblems(withSteps([{ step: "align", reference: "F444W" }, { step: "cube", output: "cube.fits" }, { step: "psfs", source: "files", files: [{ filter: "F200W", path: "" }, { filter: "F444W", path: "p444.fits" }] }]))).toEqual(["PSFs: choose the PSF file for F200W."]);
    const draft = { ...createCubePrepDraft(), recipe: withSteps([{ step: "align", reference: "F444W" }, { step: "cube", output: "cube.fits" }]), baseDir: "", outputParent: "" };
    expect(cubePrepProblems(draft, false)).toEqual([{ tab: "folders", text: "Choose the input folder for relative file paths." }]);
    expect(cubePrepProblems(draft)).toEqual([{ tab: "folders", text: "Choose the input folder for relative file paths." }, { tab: "folders", text: "Choose a working folder for this run." }]);
    expect(cubePrepProblems({ ...draft, baseDir: "/data", outputParent: "/runs" })).toEqual([]);
  });
});
