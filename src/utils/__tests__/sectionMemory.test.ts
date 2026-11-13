import { afterEach, describe, expect, it } from "vitest";
import { sectionMemory } from "../sectionMemory";

afterEach(() => sectionMemory.reset());

describe("sectionMemory", () => {
  it("gives the fallback until a section was set", () => {
    expect(sectionMemory.get("stacking", ["calibrate", "mosaic"], "calibrate")).toBe("calibrate");
  });

  it("remembers one section per tool", () => {
    sectionMemory.set("stacking", "mosaic");
    sectionMemory.set("processing", "psf");
    expect(sectionMemory.get("stacking", ["calibrate", "mosaic"], "calibrate")).toBe("mosaic");
    expect(sectionMemory.get("processing", ["background", "psf"], "background")).toBe("psf");
  });

  it("falls back when the remembered section is not offered, and after a reset", () => {
    sectionMemory.set("processing", "mosaic");
    expect(sectionMemory.get("processing", ["background", "psf"], "background")).toBe("background");
    sectionMemory.set("stacking", "mosaic");
    sectionMemory.reset();
    expect(sectionMemory.get("stacking", ["calibrate", "mosaic"], "calibrate")).toBe("calibrate");
  });
});
