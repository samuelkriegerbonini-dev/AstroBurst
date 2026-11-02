import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CubePrepRecipe } from "../../shared/types/cubeprep";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
import { cancelCubePrep, cubePrepFilters, onCubePrepProgress, runCubePrepRecipe, validateCubePrepRecipe } from "../cubeprep";

beforeEach(() => vi.clearAllMocks());

describe("Cube Prep Tauri contract", () => {
  it("routes all execution through the recipe runner and preserves the base folder", async () => {
    const recipe: CubePrepRecipe = { recipe_version: 1, inputs: [], steps: [] };
    await cubePrepFilters();
    await validateCubePrepRecipe(recipe, "C:\\data");
    await runCubePrepRecipe(recipe, "C:\\data", "C:\\jobs\\one", "job-1");
    expect(mocks.invoke.mock.calls).toEqual([
      ["cubeprep_filters_cmd"],
      ["validate_cubeprep_recipe_cmd", { recipe, baseDir: "C:\\data" }],
      ["run_cubeprep_recipe_cmd", { recipe, baseDir: "C:\\data", outputDir: "C:\\jobs\\one", runId: "job-1" }],
    ]);
  });

  it("subscribes and cancels the same isolated event", async () => {
    const callback = vi.fn();
    await onCubePrepProgress("job-2", callback);
    const [event, listener] = mocks.listen.mock.calls[0];
    const payload = { current: 1, total: 3, percent: 33, stage: "crop" };
    listener({ payload });
    await cancelCubePrep("job-2");
    expect(event).toBe("cubeprep-progress-job-2");
    expect(callback).toHaveBeenCalledWith(payload);
    expect(mocks.invoke).toHaveBeenCalledWith("cancel_progress_cmd", { event });
  });
});
