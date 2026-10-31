import { describe, expect, it } from "vitest";
import { backgroundControlsFor, withinRange } from "../backgroundControls";

describe("backgroundControlsFor", () => {
  it("gives De-band only the axis, the sigma clip and iterations up to 5", () => {
    const deband = backgroundControlsFor("deband");
    expect(deband).not.toHaveProperty("gridSize");
    expect(deband).not.toHaveProperty("polyDegree");
    expect(deband.iterations?.max).toBe(5);
    expect(deband).toEqual({
      sigmaClip: { min: 1, max: 5, step: 0.1 },
      iterations: { min: 1, max: 5, step: 1 },
      axis: true,
      applyAs: false,
      dbe: false,
    });
  });

  it("does not let the model change what De-band shows", () => {
    expect(backgroundControlsFor("deband", "spline")).toEqual(backgroundControlsFor("deband", "polynomial"));
  });

  it("gives the polynomial gradient its grid, degree, sigma clip, iterations up to 10 and Apply as", () => {
    expect(backgroundControlsFor("gradient", "polynomial")).toEqual({
      gridSize: { min: 3, max: 24, step: 1 },
      polyDegree: { min: 1, max: 5, step: 1 },
      sigmaClip: { min: 1, max: 5, step: 0.1 },
      iterations: { min: 1, max: 10, step: 1 },
      axis: false,
      applyAs: true,
      dbe: false,
    });
    expect(backgroundControlsFor("gradient")).toEqual(backgroundControlsFor("gradient", "polynomial"));
  });

  it("gives the spline gradient the DBE settings and Apply as, without the polynomial sliders", () => {
    expect(backgroundControlsFor("gradient", "spline")).toEqual({ axis: false, applyAs: true, dbe: true });
  });
});

describe("withinRange", () => {
  it("brings a value carried over from another operation into the control's range", () => {
    const iterations = backgroundControlsFor("deband").iterations ?? { min: 0, max: 0, step: 1 };
    expect(withinRange(8, iterations)).toBe(5);
    expect(withinRange(3, iterations)).toBe(3);
    expect(withinRange(0, iterations)).toBe(1);
  });
});
