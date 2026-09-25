import { describe, it, expect } from "vitest";
import { filterMatchingFrames, orderWithReferenceFirst, pathOf, sameDimensions } from "../useFrameSet";
import type { ProcessedFile } from "../../shared/types";

function file(id: string, path: string, dims: [number, number] | null): ProcessedFile {
  return {
    id,
    name: path.split("/").pop() ?? path,
    path,
    sourcePath: path,
    imageRef: null,
    size: 1,
    status: "done",
    result: dims ? { png_path: "", previewUrl: "", dimensions: dims, elapsed_ms: 0 } : null,
    error: null,
    startedAt: null,
    finishedAt: null,
  } as ProcessedFile;
}

describe("filterMatchingFrames", () => {
  const files = [
    file("a", "/run/f0.fits", [1024, 768]),
    file("b", "/run/f1.fits", [1024, 768]),
    file("c", "/run/other.fits", [2048, 2048]),
    file("d", "/run/pending.fits", null),
  ];

  it("keeps only the done files whose dimensions equal the reference", () => {
    expect(filterMatchingFrames(files, [1024, 768]).map(pathOf)).toEqual(["/run/f0.fits", "/run/f1.fits"]);
    expect(filterMatchingFrames(files, [2048, 2048]).map(pathOf)).toEqual(["/run/other.fits"]);
  });

  it("returns nothing without reference dimensions and treats a transposed size as different", () => {
    expect(filterMatchingFrames(files, null)).toEqual([]);
    expect(filterMatchingFrames(files, [768, 1024])).toEqual([]);
    expect(sameDimensions([1, 2], [1, 2])).toBe(true);
    expect(sameDimensions([1, 2], null)).toBe(false);
  });

  it("moves the reference file to the front and keeps the others in store order", () => {
    const ordered = orderWithReferenceFirst(files, "/run/f1.fits");
    expect(ordered.map(pathOf)).toEqual(["/run/f1.fits", "/run/f0.fits", "/run/other.fits", "/run/pending.fits"]);
    expect(orderWithReferenceFirst(files, "/run/missing.fits")).toBe(files);
    expect(orderWithReferenceFirst(files, null)).toBe(files);
  });

  it("measures a file loaded twice only once, keeping the first store entry", () => {
    const twice = [
      file("a", "/run/f0.fits", [64, 64]),
      file("b", "/run/f1.fits", [64, 64]),
      file("c", "/run/f0.fits", [64, 64]),
      file("d", "/run/f1.fits", [64, 64]),
    ];
    const frames = orderWithReferenceFirst(filterMatchingFrames(twice, [64, 64]), "/run/f1.fits");
    expect(frames.map(pathOf)).toEqual(["/run/f1.fits", "/run/f0.fits"]);
    expect(frames.map((f) => f.id)).toEqual(["b", "a"]);
    const staleFirst = [file("x", "/run/f0.fits", [32, 32]), file("y", "/run/f0.fits", [64, 64])];
    expect(filterMatchingFrames(staleFirst, [64, 64]).map((f) => f.id)).toEqual(["y"]);
  });
});
