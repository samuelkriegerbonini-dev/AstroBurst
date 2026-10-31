import { describe, expect, it } from "vitest";
import { pipelineProgressText } from "../pipelineProgress";

describe("pipelineProgressText", () => {
  it("names each pipeline-progress stage, with the lane, k/N and its light count", () => {
    expect(pipelineProgressText("")).toBe("Starting the pipeline...");
    expect(pipelineProgressText("masters:0")).toBe("Building calibration masters");
    expect(pipelineProgressText("loading:1200:1/3:2:R")).toBe("R 1/3: loading 2 lights");
    expect(pipelineProgressText("stacking:1500:2/3:1:G")).toBe("G 2/3: stacking 1 light");
    expect(pipelineProgressText("saving:9000")).toBe("Saving masters");
    expect(pipelineProgressText("complete")).toBe("Finishing");
  });

  it("keeps a lane label that holds a colon whole", () => {
    expect(pipelineProgressText("stacking:10:1/1:3:Ha:6nm")).toBe("Ha:6nm 1/1: stacking 3 lights");
  });

  it("shows a stage it does not know as it comes", () => {
    expect(pipelineProgressText("aligning:10:1/2:4:B")).toBe("B 1/2: aligning 4 lights");
    expect(pipelineProgressText("warming")).toBe("warming");
    expect(pipelineProgressText("stacking:10:x/2:4:B")).toBe("stacking");
  });
});
