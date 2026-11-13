import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ImageDepth } from "../../shared/types/depth";

const holder = vi.hoisted(() => ({ byPath: {} as Record<string, unknown> }));

vi.mock("react", async (importOriginal) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../useDepthStore", () => ({
  depthStore: { subscribe: () => () => {}, latest: (path: string) => holder.byPath[path] ?? null },
}));

import { hooks } from "../../test/hookRuntime";
import { useImageDepth } from "../usePhotometryLimit";

const LEGACY_DEPTH = { radius_arcsec: 1.0, radius_px: 3.8167938931297707, f_corr: 1.647 } as unknown as ImageDepth;

describe("useImageDepth", () => {
  beforeEach(() => {
    hooks.mount();
    holder.byPath = {};
  });

  it("gives the latest depth of the file and nothing without a file", () => {
    holder.byPath["a.fits"] = LEGACY_DEPTH;
    expect(hooks.render(() => useImageDepth("a.fits"), undefined)).toBe(LEGACY_DEPTH);
    expect(hooks.render(() => useImageDepth(null), undefined)).toBeNull();
    expect(hooks.render(() => useImageDepth("b.fits"), undefined)).toBeNull();
    expect(hooks.render(() => useImageDepth("a.fits"), undefined)).toBe(LEGACY_DEPTH);
  });
});
