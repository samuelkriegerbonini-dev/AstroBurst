import { describe, expect, it } from "vitest";
import { applyProfile, createCubePrepDraft, cubePrepPsfTarget, inputDirectory, parseRecipe, recipeForSave, recipeProblems, setRecipeStep, workingDirectory } from "../cubePrepModel";
import type { CubePrepReport } from "../../../shared/types/cubeprep";

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
});
