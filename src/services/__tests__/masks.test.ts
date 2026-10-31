import { beforeEach, describe, expect, it, vi } from "vitest";

const { typedInvokeMock, announceMock } = vi.hoisted(() => ({
  typedInvokeMock: vi.fn(),
  announceMock: vi.fn(),
}));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  announceCommandOutputs: announceMock,
}));

import { COMPOSITE_MASK_TARGET, createMask, deleteMask, exportMask, previewMask, type MaskTarget } from "../masks";
import type { MaskSpec } from "../../shared/types/masks";

const ROOT = "C:/data/jw02739-o001_t001_nircam_clear-f444w_i2d.fits";
const NODE = "C:/app/history/3f9c0a1e5b7d2c44/n0002.fits";
const FILE: MaskTarget = { imageKey: ROOT, basePath: null, inputPath: NODE };
const REGIONS: MaskSpec = {
  kind: "regions",
  shapes: [{ shape: "circle", x: 83.82, y: -5.39, r: 12 }],
  exclude_shapes: [{ shape: "box", x: 83.81, y: -5.38, width: 4, height: 4, angle: 0 }],
  feather_px: 3,
  invert: false,
};

function previewPayload(width: number, height: number, cells: number[]): ArrayBuffer {
  const buf = new ArrayBuffer(16 + cells.length);
  const view = new DataView(buf);
  view.setUint32(0, width, true);
  view.setUint32(4, height, true);
  new Uint8Array(buf, 16).set(cells);
  return buf;
}

beforeEach(() => {
  typedInvokeMock.mockReset();
  announceMock.mockReset();
});

describe("masks service", () => {
  it("createMask_sends_camelCase_top_level_keys_and_the_flat_snake_case_recipe", async () => {
    const result = { mask_id: "m01", kind: "regions", coverage: 0.0123, dims: [2048, 2048], stars: null, recipe: REGIONS };
    typedInvokeMock.mockResolvedValueOnce(result);
    await expect(createMask(FILE, REGIONS, { shapesSystem: "fk5" })).resolves.toBe(result);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("mask_create_cmd", {
      imageKey: ROOT,
      basePath: null,
      inputPath: NODE,
      recipe: REGIONS,
      shapesSystem: "fk5",
      mods: null,
    });

    typedInvokeMock.mockResolvedValueOnce(result);
    await createMask({ imageKey: ROOT, basePath: "C:/out/stack_ha.fits" }, { kind: "luminance", auto_stf: true }, { mods: { feather_px: 8, invert: true } });
    expect(typedInvokeMock).toHaveBeenLastCalledWith("mask_create_cmd", {
      imageKey: ROOT,
      basePath: "C:/out/stack_ha.fits",
      inputPath: null,
      recipe: { kind: "luminance", auto_stf: true },
      shapesSystem: null,
      mods: { feather_px: 8, invert: true },
    });
  });

  it("previewMask_parses_the_dq_layout_returned_by_mask_preview_cmd", async () => {
    typedInvokeMock.mockResolvedValueOnce(previewPayload(3, 1, [0, 128, 255]));
    const preview = await previewMask(FILE, "m01");
    expect(typedInvokeMock).toHaveBeenLastCalledWith("mask_preview_cmd", { imageKey: ROOT, basePath: null, inputPath: NODE, maskId: "m01", maxDim: null });
    expect([preview.width, preview.height, Array.from(preview.cells)]).toEqual([3, 1, [0, 128, 255]]);

    typedInvokeMock.mockResolvedValueOnce(previewPayload(1, 1, [7]));
    await previewMask(COMPOSITE_MASK_TARGET, "m02", 512);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("mask_preview_cmd", { imageKey: "__composite__", basePath: null, inputPath: null, maskId: "m02", maxDim: 512 });

    typedInvokeMock.mockResolvedValueOnce(new ArrayBuffer(8));
    await expect(previewMask(FILE, "m01")).rejects.toThrow("mask preview: response too small (8 bytes)");
  });

  it("deleteMask_and_exportMask_send_the_documented_arguments", async () => {
    const removed = { mask_id: "m01", recipe: REGIONS, used_by: ["n0001"], kept_for_replay: true };
    typedInvokeMock.mockResolvedValueOnce(removed);
    await expect(deleteMask(FILE, "m01")).resolves.toBe(removed);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("mask_delete_cmd", { imageKey: ROOT, basePath: null, maskId: "m01" });

    const written = { mask_id: "m01", kind: "label", fits_path: "C:/out/f444w_m01_labels.fits", bitpix: 16, dims: [2048, 2048] };
    typedInvokeMock.mockResolvedValueOnce(written);
    await expect(exportMask(FILE, "m01", "C:/out", "label")).resolves.toBe(written);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("mask_export_cmd", { imageKey: ROOT, basePath: null, inputPath: NODE, maskId: "m01", path: "C:/out", kind: "label" });
    expect(announceMock).toHaveBeenCalledTimes(1);
    expect(announceMock).toHaveBeenCalledWith(written);
  });

  it("the_composite_target_selects_the_session_composite", async () => {
    expect(COMPOSITE_MASK_TARGET).toEqual({ imageKey: "__composite__", basePath: null, inputPath: null });
    expect(Object.isFrozen(COMPOSITE_MASK_TARGET)).toBe(true);
    typedInvokeMock.mockResolvedValueOnce({ mask_id: "m01", recipe: REGIONS, used_by: [], kept_for_replay: false });
    await deleteMask(COMPOSITE_MASK_TARGET, "m01");
    expect(typedInvokeMock).toHaveBeenLastCalledWith("mask_delete_cmd", { imageKey: "__composite__", basePath: null, maskId: "m01" });
  });
});
