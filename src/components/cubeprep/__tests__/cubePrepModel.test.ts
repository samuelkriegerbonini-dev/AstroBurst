import { describe, expect, it } from "vitest";
import { applyProfile, changeRecipeInput, createCubePrepDraft, cubePrepError, cubePrepPsfTarget, cubePrepStageText, defaultStep, inputDirectory, parseRecipe, recipeForSave, recipeProblems, setRecipeStep, workingDirectory } from "../cubePrepModel";
import { normalizeTauriError } from "../../../shared/types/errors";
import type { CubePrepRecipe, CubePrepReport, CubePrepStep } from "../../../shared/types/cubeprep";

const TWO_INPUTS = [{ path: "a.fits", dataset: "JWST", filter: "F200W" }, { path: "b.fits", dataset: "JWST", filter: "F444W" }];
const withSteps = (steps: CubePrepStep[]): CubePrepRecipe => ({ recipe_version: 1, name: "field", inputs: TWO_INPUTS, steps });
const PSFS: CubePrepStep = { step: "psfs", source: "files", files: [{ filter: "F200W", path: "p200.fits" }, { filter: "F444W", path: "p444.fits" }] };
const KERNELS: CubePrepStep = { step: "kernels", reference: "F444W", method: "wiener", eps: 0.001 };

describe("Cube Prep recipes", () => {
  it("offers a common PSF only for complete matching of the current final bands", () => {
    const report = { steps: [{ step: "match", parameters: { reference: "F444W" }, report: { reference: "F444W", planes: [{ filter: "F200W", status: "matched" }, { filter: "F444W", status: "reference" }] } }, { step: "export" }] } as CubePrepReport;
    expect(cubePrepPsfTarget(report, ["F200W", "F444W"])).toBe("F444W");
    expect(cubePrepPsfTarget(report, ["F200W", "F356W", "F444W"])).toBe("");
    expect(cubePrepPsfTarget({ ...report, steps: [...report.steps, { step: "cube" } as CubePrepReport["steps"][number]] }, ["F200W", "F444W"])).toBe("");
    (report.steps[0].report as { planes: { status: string }[] }).planes[0].status = "unmatched";
    expect(cubePrepPsfTarget(report, ["F200W", "F444W"])).toBe("");
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
    const draft = createCubePrepDraft();
    expect(draft.recipe.steps).toEqual([
      { step: "units", to: "Jy/pixel" },
      { step: "align", reference: "", method: "bilinear", border_crop: 1, area_rule: "lightstack" },
      { step: "cube", reference: "", order: "wavelength", output: "cube.fits", layout: "lightstack" },
    ]);
    const recipe = setRecipeStep(draft.recipe, { step: "crop", mode: "common", rounding: "lightstack" });
    expect(recipe.steps.map((s) => s.step)).toEqual(["crop", "units", "align", "cube"]);
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

  it("rounds a sky crop edit like the profile's crop (OD-L029)", () => {
    const sky = setRecipeStep(createCubePrepDraft().recipe, { step: "edit", action: "cut_sky", ra: 53.16, dec: -27.79, size_arcsec: 1.7, rounding: "astroburst" });
    expect(applyProfile(sky, "lightstack").steps.find((s) => s.step === "edit")?.rounding).toBe("lightstack");
    expect(applyProfile(applyProfile(sky, "lightstack"), "scientific").steps.find((s) => s.step === "edit")?.rounding).toBe("astroburst");
    const pixels = setRecipeStep(createCubePrepDraft().recipe, { step: "edit", action: "cut", x_start: 0, x_end: 2, y_start: 0, y_end: 2 });
    expect(applyProfile(pixels, "lightstack").steps.find((s) => s.step === "edit")).not.toHaveProperty("rounding");
  });
});
