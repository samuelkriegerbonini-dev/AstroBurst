import { describe, it, expect } from "vitest";
import {
  applyWaitsForDetection,
  arrowDelta,
  clampMargins,
  clientDeltaToGrid,
  CROP_BODY_ROLE_DESCRIPTION,
  CROP_EDGE_HANDLES,
  CROP_HANDLE_ROLE_DESCRIPTION,
  CROP_HANDLES,
  CROP_TARGET_CURSORS,
  CROP_TARGET_LABELS,
  cropHitLayout,
  cropOverlayKeys,
  croppedSize,
  cropSizeLabel,
  dragHandle,
  dragMargins,
  draftMargins,
  EDITOR_MAX_HEIGHT_PX,
  EDITOR_MIN_HEIGHT_PX,
  editDraft,
  editorBoxHeight,
  editorBoxSize,
  editorNaturalWidth,
  fitPreviewBox,
  gridSize,
  INITIAL_MARGIN_DRAFT,
  KEY_STEP_FAST_PX,
  KEY_STEP_PX,
  marginEdgeMax,
  marginsFromBounds,
  marginsFromRect,
  moveBody,
  nudgeTarget,
  pointerDragStep,
  prefillDraft,
  rebaseDrag,
  rectFromMargins,
  replaceDraft,
  sameMargins,
  setMarginEdge,
  suppressTextSelection,
  typeMarginDraft,
  unitsPerScreenPx,
  ZERO_MARGINS,
  type CropMargins,
  type CropTarget,
  type GridSize,
  type HitRect,
  type MarginDraft,
  type PointerDrag,
} from "../cropRect";
import { INITIAL_STATE, invalidateDownstream, MAX_OVERLAY_CHANNELS } from "../wizard";

const grid: GridSize = { width: 100, height: 80 };
const margins: CropMargins = { top: 10, bottom: 20, left: 5, right: 15 };
const box = { left: 200, top: 50, width: 250, height: 200 };

function screenX(edge: number, size: GridSize, drawn: { left: number; width: number }): number {
  return drawn.left + (edge * drawn.width) / size.width;
}

function screenY(edge: number, size: GridSize, drawn: { top: number; height: number }): number {
  return drawn.top + (edge * drawn.height) / size.height;
}

describe("gridSize", () => {
  it("reads [cols, rows] as width and height", () => {
    expect(gridSize([640, 480])).toEqual({ width: 640, height: 480 });
  });

  it("rejects missing, empty, negative and fractional dimensions", () => {
    expect(gridSize(null)).toBeNull();
    expect(gridSize(undefined)).toBeNull();
    expect(gridSize([0, 10])).toBeNull();
    expect(gridSize([10, -1])).toBeNull();
    expect(gridSize([10.5, 10])).toBeNull();
    expect(gridSize([Number.NaN, 10])).toBeNull();
  });
});

describe("clientDeltaToGrid", () => {
  it("scales a screen distance to whole grid pixels on that axis", () => {
    expect(clientDeltaToGrid(125, 250, 100)).toBe(50);
    expect(clientDeltaToGrid(-1, 208, 4096)).toBe(-20);
    expect(clientDeltaToGrid(1.2, 250, 100)).toBe(0);
    expect(clientDeltaToGrid(1.3, 250, 100)).toBe(1);
  });

  it("gives 0 for an empty box or a non-finite distance", () => {
    expect(clientDeltaToGrid(50, 0, 100)).toBe(0);
    expect(clientDeltaToGrid(50, -5, 100)).toBe(0);
    expect(clientDeltaToGrid(Number.NaN, 250, 100)).toBe(0);
  });
});

describe("rectFromMargins and marginsFromRect", () => {
  it("turns margins into half-open column and row ranges", () => {
    expect(rectFromMargins(margins, grid)).toEqual({ x0: 5, y0: 10, x1: 85, y1: 60 });
  });

  it("round-trips", () => {
    expect(marginsFromRect(rectFromMargins(margins, grid), grid)).toEqual(margins);
    const rect = { x0: 0, y0: 3, x1: 100, y1: 79 };
    expect(rectFromMargins(marginsFromRect(rect, grid), grid)).toEqual(rect);
  });

  it("matches the Rust crop size for margins top 2, bottom 4, left 3, right 5 on a 50x40 grid", () => {
    const rustGrid: GridSize = { width: 50, height: 40 };
    expect(croppedSize({ top: 2, bottom: 4, left: 3, right: 5 }, rustGrid)).toEqual({ width: 42, height: 34 });
  });

  it("keeps the first kept row at the top margin and the last at H - bottom - 1", () => {
    const rect = rectFromMargins(margins, grid);
    expect(rect.y0).toBe(margins.top);
    expect(rect.y1 - 1).toBe(grid.height - margins.bottom - 1);
  });
});

describe("marginsFromBounds", () => {
  it("copies the detection margins", () => {
    expect(
      marginsFromBounds({ crop_top: 1, crop_bottom: 2, crop_left: 3, crop_right: 4 }),
    ).toEqual({ top: 1, bottom: 2, left: 3, right: 4 });
  });
});

describe("clampMargins", () => {
  it("leaves valid margins alone", () => {
    expect(clampMargins(margins, grid)).toEqual(margins);
  });

  it("turns negative, fractional and non-finite values into whole numbers >= 0", () => {
    expect(clampMargins({ top: -3, bottom: 2.6, left: Number.NaN, right: Number.POSITIVE_INFINITY }, grid)).toEqual({
      top: 0,
      bottom: 3,
      left: 0,
      right: 0,
    });
  });

  it("keeps at least one row and one column", () => {
    expect(clampMargins({ top: 500, bottom: 500, left: 500, right: 500 }, grid)).toEqual({
      top: 79,
      bottom: 0,
      left: 99,
      right: 0,
    });
    const kept = croppedSize(clampMargins({ top: 60, bottom: 60, left: 70, right: 70 }, grid), grid);
    expect(kept.width).toBeGreaterThanOrEqual(1);
    expect(kept.height).toBeGreaterThanOrEqual(1);
  });

  it("only sanitises when the grid is unknown", () => {
    expect(clampMargins({ top: 5000, bottom: -1, left: 2.2, right: 7 }, null)).toEqual({
      top: 5000,
      bottom: 0,
      left: 2,
      right: 7,
    });
  });

  it("gives zero margins for an empty grid", () => {
    expect(clampMargins(margins, { width: 0, height: 0 })).toEqual(ZERO_MARGINS);
  });
});

describe("marginEdgeMax", () => {
  it("leaves one pixel beside the opposite margin", () => {
    expect(marginEdgeMax(margins, "top", grid)).toBe(80 - 20 - 1);
    expect(marginEdgeMax(margins, "bottom", grid)).toBe(80 - 10 - 1);
    expect(marginEdgeMax(margins, "left", grid)).toBe(100 - 15 - 1);
    expect(marginEdgeMax(margins, "right", grid)).toBe(100 - 5 - 1);
  });
});

describe("setMarginEdge", () => {
  it("changes only the edited margin", () => {
    expect(setMarginEdge(margins, "left", 12, grid)).toEqual({ ...margins, left: 12 });
  });

  it("clamps the edited margin against the opposite one", () => {
    expect(setMarginEdge(margins, "top", 1000, grid)).toEqual({ ...margins, top: 59 });
    expect(setMarginEdge(margins, "right", 1000, grid)).toEqual({ ...margins, right: 94 });
  });

  it("rounds and floors typed values at 0", () => {
    expect(setMarginEdge(margins, "bottom", -4, grid)).toEqual({ ...margins, bottom: 0 });
    expect(setMarginEdge(margins, "bottom", 7.5, grid)).toEqual({ ...margins, bottom: 8 });
    expect(setMarginEdge(margins, "bottom", Number.NaN, grid)).toEqual({ ...margins, bottom: 0 });
  });

  it("has no upper bound when the grid is unknown", () => {
    expect(setMarginEdge(margins, "top", 1000, null)).toEqual({ ...margins, top: 1000 });
  });
});

describe("dragHandle", () => {
  it("moves only the top edge for the top handle", () => {
    expect(dragHandle(margins, "n", { x: 70, y: 4 }, grid)).toEqual({ ...margins, top: 4 });
  });

  it("moves only the bottom edge for the bottom handle", () => {
    expect(dragHandle(margins, "s", { x: 0, y: 70 }, grid)).toEqual({ ...margins, bottom: 10 });
  });

  it("moves only the left and right edges for the side handles", () => {
    expect(dragHandle(margins, "w", { x: 20, y: 0 }, grid)).toEqual({ ...margins, left: 20 });
    expect(dragHandle(margins, "e", { x: 90, y: 0 }, grid)).toEqual({ ...margins, right: 10 });
  });

  it("moves two edges for a corner", () => {
    expect(dragHandle(margins, "nw", { x: 0, y: 0 }, grid)).toEqual({ ...margins, top: 0, left: 0 });
    expect(dragHandle(margins, "ne", { x: 100, y: 1 }, grid)).toEqual({ ...margins, top: 1, right: 0 });
    expect(dragHandle(margins, "sw", { x: 2, y: 80 }, grid)).toEqual({ ...margins, bottom: 0, left: 2 });
    expect(dragHandle(margins, "se", { x: 50, y: 40 }, grid)).toEqual({ ...margins, bottom: 40, right: 50 });
  });

  it("stops one pixel before the opposite edge instead of flipping", () => {
    expect(dragHandle(margins, "n", { x: 0, y: 75 }, grid)).toEqual({ ...margins, top: 59 });
    expect(dragHandle(margins, "s", { x: 0, y: 0 }, grid)).toEqual({ ...margins, bottom: 69 });
    expect(dragHandle(margins, "w", { x: 99, y: 0 }, grid)).toEqual({ ...margins, left: 84 });
    expect(dragHandle(margins, "e", { x: 0, y: 0 }, grid)).toEqual({ ...margins, right: 94 });
  });

  it("keeps edges inside the grid", () => {
    expect(dragHandle(margins, "nw", { x: -40, y: -40 }, grid)).toEqual({ ...margins, top: 0, left: 0 });
    expect(dragHandle(margins, "se", { x: 400, y: 400 }, grid)).toEqual({ ...margins, bottom: 0, right: 0 });
  });

  it("rounds fractional points and ignores non-finite ones", () => {
    expect(dragHandle(margins, "n", { x: 0, y: 3.4 }, grid)).toEqual({ ...margins, top: 3 });
    expect(dragHandle(margins, "n", { x: 0, y: Number.NaN }, grid)).toEqual(margins);
  });
});

describe("moveBody", () => {
  it("translates the box and keeps its size", () => {
    const moved = moveBody(margins, 3, -4, grid);
    expect(moved).toEqual({ top: 6, bottom: 24, left: 8, right: 12 });
    expect(croppedSize(moved, grid)).toEqual(croppedSize(margins, grid));
  });

  it("stops at every border", () => {
    expect(moveBody(margins, -100, -100, grid)).toEqual({ top: 0, bottom: 30, left: 0, right: 20 });
    expect(moveBody(margins, 100, 100, grid)).toEqual({ top: 30, bottom: 0, left: 20, right: 0 });
  });

  it("cannot move a full-frame box", () => {
    expect(moveBody(ZERO_MARGINS, 10, -10, grid)).toEqual(ZERO_MARGINS);
  });

  it("rounds fractional deltas and ignores non-finite ones", () => {
    expect(moveBody(margins, 0.6, Number.NaN, grid)).toEqual({ ...margins, left: 6, right: 14 });
  });
});

describe("arrowDelta", () => {
  it("moves 1 px per arrow and 10 px with Shift, y down", () => {
    expect(arrowDelta("ArrowUp", false)).toEqual({ dx: 0, dy: -KEY_STEP_PX });
    expect(arrowDelta("ArrowDown", false)).toEqual({ dx: 0, dy: KEY_STEP_PX });
    expect(arrowDelta("ArrowLeft", true)).toEqual({ dx: -KEY_STEP_FAST_PX, dy: 0 });
    expect(arrowDelta("ArrowRight", true)).toEqual({ dx: KEY_STEP_FAST_PX, dy: 0 });
    expect(KEY_STEP_PX).toBe(1);
    expect(KEY_STEP_FAST_PX).toBe(10);
  });

  it("ignores other keys", () => {
    expect(arrowDelta("Enter", false)).toBeNull();
    expect(arrowDelta("a", true)).toBeNull();
  });
});

describe("nudgeTarget", () => {
  it("moves the top edge up and down", () => {
    expect(nudgeTarget(margins, "n", 0, -1, grid)).toEqual({ ...margins, top: 9 });
    expect(nudgeTarget(margins, "n", 0, 10, grid)).toEqual({ ...margins, top: 20 });
  });

  it("ignores the axis an edge does not control", () => {
    expect(nudgeTarget(margins, "n", 10, 0, grid)).toEqual(margins);
    expect(nudgeTarget(margins, "e", 0, 10, grid)).toEqual(margins);
  });

  it("moves the bottom and right edges outward with Down and Right", () => {
    expect(nudgeTarget(margins, "s", 0, 1, grid)).toEqual({ ...margins, bottom: 19 });
    expect(nudgeTarget(margins, "e", 1, 0, grid)).toEqual({ ...margins, right: 14 });
  });

  it("moves one edge of a corner per arrow", () => {
    expect(nudgeTarget(margins, "nw", -1, 0, grid)).toEqual({ ...margins, left: 4 });
    expect(nudgeTarget(margins, "nw", 0, -1, grid)).toEqual({ ...margins, top: 9 });
    expect(nudgeTarget(margins, "se", 0, -10, grid)).toEqual({ ...margins, bottom: 30 });
  });

  it("moves the whole box for the body", () => {
    expect(nudgeTarget(margins, "body", 10, 0, grid)).toEqual({ ...margins, left: 15, right: 5 });
  });

  it("stops at the grid border and at one pixel", () => {
    expect(nudgeTarget({ ...margins, top: 0 }, "n", 0, -10, grid)).toEqual({ ...margins, top: 0 });
    const thin = { top: 39, bottom: 40, left: 0, right: 0 };
    expect(nudgeTarget(thin, "n", 0, 1, grid)).toEqual(thin);
    expect(nudgeTarget(thin, "s", 0, -1, grid)).toEqual(thin);
  });
});

describe("dragMargins", () => {
  const full: GridSize = { width: 4096, height: 3072 };
  const drawn = { left: 0, top: 0, ...fitPreviewBox(1024, 768, 400, 156) };
  const start: CropMargins = { top: 300, bottom: 300, left: 400, right: 400 };
  const grabOffsets = [-4.5, -4, -2, 0, 2, 4, 4.5];
  const far = 5000;

  function grab(target: CropTarget, from: CropMargins, x: number, y: number) {
    return { target, margins: from, clientX: x, clientY: y };
  }

  it("uses the 208x156 box of a 1024x768 preview, about 20 grid pixels per screen pixel", () => {
    expect(drawn.width).toBe(208);
    expect(drawn.height).toBe(156);
  });

  it("follows round(u*W) of the pointer when an edge is grabbed on its own screen position", () => {
    const down = grab("w", margins, screenX(margins.left, grid, box), 120);
    expect(dragMargins(down, 325, 120, box, grid)).toEqual({ ...margins, left: 50 });
    const top = grab("n", margins, 300, screenY(margins.top, grid, box));
    expect(dragMargins(top, 300, 100, box, grid)).toEqual({ ...margins, top: 20 });
  });

  it("moves the top edge down when the pointer moves down, with no vertical flip", () => {
    const down = grab("n", margins, 300, screenY(margins.top, grid, box));
    const next = dragMargins(down, 300, down.clientY + 25, box, grid);
    expect(next.top).toBe(margins.top + 10);
    expect(next.bottom).toBe(margins.bottom);
  });

  it("does not move the edge until the pointer moves, wherever the handle was grabbed", () => {
    for (const offset of grabOffsets) {
      const down = grab("w", start, screenX(start.left, full, drawn) + offset, 50);
      expect(dragMargins(down, down.clientX, down.clientY, drawn, full)).toEqual(start);
      expect(dragMargins(down, down.clientX - 1, down.clientY, drawn, full).left).toBe(start.left - 20);
    }
  });

  it("reaches margin 0 on every side when dragged past the border, for every grab offset", () => {
    for (const offset of grabOffsets) {
      const left = grab("w", start, screenX(start.left, full, drawn) + offset, 50);
      expect(dragMargins(left, -far, 50, drawn, full)).toEqual({ ...start, left: 0 });
      const right = grab("e", start, screenX(full.width - start.right, full, drawn) + offset, 50);
      expect(dragMargins(right, far, 50, drawn, full)).toEqual({ ...start, right: 0 });
      const top = grab("n", start, 100, screenY(start.top, full, drawn) + offset);
      expect(dragMargins(top, 100, -far, drawn, full)).toEqual({ ...start, top: 0 });
      const bottom = grab("s", start, 100, screenY(full.height - start.bottom, full, drawn) + offset);
      expect(dragMargins(bottom, 100, far, drawn, full)).toEqual({ ...start, bottom: 0 });
      const corner = grab(
        "se",
        start,
        screenX(full.width - start.right, full, drawn) + offset,
        screenY(full.height - start.bottom, full, drawn) + offset,
      );
      expect(dragMargins(corner, far, far, drawn, full)).toEqual({ ...start, bottom: 0, right: 0 });
    }
  });

  it("reaches the border once the pointer has covered the edge's own screen distance", () => {
    const down = grab("w", start, screenX(start.left, full, drawn) + 4, 50);
    const distance = screenX(start.left, full, drawn) - drawn.left;
    expect(dragMargins(down, down.clientX - distance, 50, drawn, full).left).toBe(0);
  });

  it("reaches 0 for an edge near the border grabbed on the part of the handle outside the image", () => {
    const near = { ...start, left: 30 };
    const down = grab("w", near, screenX(near.left, full, drawn) - 4, 50);
    expect(down.clientX).toBeLessThan(drawn.left);
    expect(dragMargins(down, -far, 50, drawn, full).left).toBe(0);
  });

  it("moves the body to every border and keeps its size", () => {
    const down = grab("body", start, 104, 78);
    expect(dragMargins(down, -far, -far, drawn, full)).toEqual({ top: 0, bottom: 600, left: 0, right: 800 });
    expect(dragMargins(down, far, far, drawn, full)).toEqual({ top: 600, bottom: 0, left: 800, right: 0 });
  });

  it("still moves the grabbed edge of a box collapsed to one row", () => {
    const thin: CropMargins = { top: 39, bottom: 40, left: 0, right: 0 };
    const top = grab("n", thin, 300, screenY(39, grid, box));
    expect(dragMargins(top, 300, top.clientY - 10, box, grid)).toEqual({ ...thin, top: 35 });
    expect(dragMargins(top, 300, top.clientY + 10, box, grid)).toEqual(thin);
    const bottom = grab("s", thin, 300, screenY(40, grid, box));
    expect(dragMargins(bottom, 300, bottom.clientY + 10, box, grid)).toEqual({ ...thin, bottom: 36 });
  });

  it("ignores an empty box and non-finite pointers", () => {
    const down = grab("nw", margins, 220, 90);
    expect(dragMargins(down, 0, 0, { width: 0, height: 0 }, grid)).toEqual(margins);
    expect(dragMargins(down, Number.NaN, Number.NaN, box, grid)).toEqual(margins);
  });
});

describe("margin drafts", () => {
  const detected: CropMargins = { top: 40, bottom: 25, left: 60, right: 30 };
  const dragged: CropMargins = { top: 5, bottom: 0, left: 0, right: 0 };

  it("starts as zero auto margins", () => {
    expect(INITIAL_MARGIN_DRAFT).toEqual({ margins: ZERO_MARGINS, source: "auto", revision: 0, typedEdges: [] });
  });

  it("bumps the revision whenever the margins are replaced from outside the editor", () => {
    const typed = replaceDraft(INITIAL_MARGIN_DRAFT, margins, "manual");
    expect(typed).toEqual({ margins, source: "manual", revision: 1, typedEdges: [] });
    expect(replaceDraft(typed, detected, "auto").revision).toBe(2);
  });

  it("pre-fills auto margins but never overwrites manual ones", () => {
    expect(prefillDraft(INITIAL_MARGIN_DRAFT, detected)).toEqual({ margins: detected, source: "auto", revision: 1, typedEdges: [] });
    const manual: MarginDraft = { margins, source: "manual", revision: 3, typedEdges: ["top"] };
    expect(prefillDraft(manual, detected)).toBe(manual);
  });

  it("takes an editor change made on the current revision as manual, keeping the revision", () => {
    const current: MarginDraft = { margins: detected, source: "auto", revision: 4, typedEdges: [] };
    expect(editDraft(current, dragged, 4)).toEqual({ margins: dragged, source: "manual", revision: 4, typedEdges: [] });
  });

  it("drops a drag that started before detection pre-filled the box", () => {
    const atPointerDown = INITIAL_MARGIN_DRAFT;
    const prefilled = prefillDraft(atPointerDown, detected);
    expect(editDraft(prefilled, dragged, atPointerDown.revision)).toBe(prefilled);
  });

  it("drops a drag that started before the step reloaded for a new alignment", () => {
    const edited: MarginDraft = { margins: dragged, source: "manual", revision: 2, typedEdges: [] };
    const reloaded = replaceDraft(edited, ZERO_MARGINS, "auto");
    expect(editDraft(reloaded, { ...dragged, top: 9 }, edited.revision)).toBe(reloaded);
    expect(prefillDraft(reloaded, detected).margins).toEqual(detected);
  });

  it("holds Apply only while detection may still replace auto margins", () => {
    expect(applyWaitsForDetection(INITIAL_MARGIN_DRAFT, true)).toBe(true);
    expect(applyWaitsForDetection(INITIAL_MARGIN_DRAFT, false)).toBe(false);
    expect(applyWaitsForDetection({ margins: dragged, source: "manual", revision: 1, typedEdges: [] }, true)).toBe(false);
  });
});

describe("aligned input identity", () => {
  it("is kept by a crop, so Apply does not reload the editor", () => {
    const state = {
      ...INITIAL_STATE,
      alignedPaths: { r: "__wizard_ch_r_aligned", g: "__wizard_ch_g_aligned" },
    };
    const afterCrop = { ...state, ...invalidateDownstream(state, "crop") };
    expect(afterCrop.alignedPaths).toBe(state.alignedPaths);
  });
});

describe("typed margins against a grid that arrives later", () => {
  const detected: CropMargins = { top: 40, bottom: 25, left: 60, right: 30 };
  const big: GridSize = { width: 1200, height: 800 };

  it("clamps the typed edge and keeps the opposite edge when the typed value exceeds the grid", () => {
    expect(clampMargins({ ...detected, top: 5000 }, big, ["top"])).toEqual({ ...detected, top: 774 });
    expect(clampMargins({ ...detected, left: 5000 }, big, ["left"])).toEqual({ ...detected, left: 1169 });
    expect(clampMargins({ ...detected, bottom: 5000 }, big, ["bottom"])).toEqual({ ...detected, bottom: 759 });
    expect(clampMargins({ ...detected, right: 5000 }, big, ["right"])).toEqual({ ...detected, right: 1139 });
  });

  it("lets the most recently typed edge of each axis yield", () => {
    const both = { top: 5000, bottom: 5000, left: 5000, right: 5000 };
    expect(clampMargins(both, big, ["bottom", "top", "right", "left"])).toEqual({
      top: 0,
      bottom: 799,
      left: 0,
      right: 1199,
    });
    expect(clampMargins(both, big, ["top", "bottom", "left", "right"])).toEqual({
      top: 799,
      bottom: 0,
      left: 1199,
      right: 0,
    });
  });

  it("keeps the untyped edge of an axis when another axis was typed after it", () => {
    let draft = replaceDraft(INITIAL_MARGIN_DRAFT, detected, "auto");
    draft = typeMarginDraft(draft, setMarginEdge(draftMargins(draft, null), "top", 5000, null), "top");
    draft = typeMarginDraft(draft, setMarginEdge(draftMargins(draft, null), "left", 10, null), "left");
    expect(draft.typedEdges).toEqual(["top", "left"]);
    expect(draftMargins(draft, null)).toEqual({ ...detected, top: 5000, left: 10 });
    expect(draftMargins(draft, big)).toEqual({ top: 774, bottom: 25, left: 10, right: 30 });
  });

  it("records each typed edge once, last typed last, and bumps the revision", () => {
    let draft = typeMarginDraft(INITIAL_MARGIN_DRAFT, detected, "top");
    draft = typeMarginDraft(draft, detected, "left");
    draft = typeMarginDraft(draft, detected, "top");
    expect(draft).toEqual({ margins: detected, source: "manual", revision: 3, typedEdges: ["left", "top"] });
  });

  it("forgets typed edges after a drag, Auto or a reload", () => {
    const typed = typeMarginDraft(INITIAL_MARGIN_DRAFT, detected, "top");
    expect(editDraft(typed, detected, typed.revision).typedEdges).toEqual([]);
    expect(replaceDraft(typed, detected, "auto").typedEdges).toEqual([]);
  });

  it("matches clampMargins when nothing was typed", () => {
    const over = { top: 900, bottom: 900, left: 0, right: 0 };
    expect(draftMargins({ ...INITIAL_MARGIN_DRAFT, margins: over }, big)).toEqual(clampMargins(over, big));
    expect(clampMargins(over, big)).toEqual({ top: 799, bottom: 0, left: 0, right: 0 });
  });
});

describe("pointer drags that outlive a margin change", () => {
  const big: GridSize = { width: 4096, height: 3072 };
  const drawn = fitPreviewBox(1024, 768, 400, 156);
  const detected: CropMargins = { top: 300, bottom: 300, left: 400, right: 400 };

  function startDrag(revision: number, target: CropTarget, from: CropMargins, x: number, y: number): PointerDrag {
    return { revision, start: { target, margins: from, clientX: x, clientY: y } };
  }

  it("keeps the drag unchanged while the revision is the same", () => {
    const drag = startDrag(1, "w", detected, 20, 50);
    expect(rebaseDrag(drag, 1, ZERO_MARGINS, 90, 90)).toBe(drag);
  });

  it("restarts from the margins on screen and the current pointer when the revision changed", () => {
    const drag = startDrag(0, "w", ZERO_MARGINS, 1, 50);
    expect(rebaseDrag(drag, 1, detected, 30, 60)).toEqual(startDrag(1, "w", detected, 30, 60));
  });

  it("applies a detection pre-fill that lands between pointer-down and the first move, then drags from it", () => {
    const atDown = INITIAL_MARGIN_DRAFT;
    const drag = startDrag(atDown.revision, "w", draftMargins(atDown, big), 1, 50);
    const prefilled = prefillDraft(atDown, detected);
    const shown = draftMargins(prefilled, big);

    const first = pointerDragStep(drag, prefilled.revision, shown, 6, 50, drawn, big);
    expect(first.margins).toEqual(detected);
    expect(first.drag.revision).toBe(prefilled.revision);
    expect(editDraft(prefilled, first.margins, first.drag.revision)).toEqual({
      margins: detected,
      source: "manual",
      revision: prefilled.revision,
      typedEdges: [],
    });

    const second = pointerDragStep(first.drag, prefilled.revision, shown, 7, 50, drawn, big);
    expect(second.margins).toEqual({ ...detected, left: 420 });
    const kept = editDraft(prefilled, second.margins, second.drag.revision);
    expect(kept.margins).toEqual({ ...detected, left: 420 });
    expect(kept.source).toBe("manual");
  });

  it("still drags past every border with any grab offset after a rebase", () => {
    for (const offset of [-4, 0, 4]) {
      const drag = startDrag(0, "e", ZERO_MARGINS, 300 + offset, 50);
      const first = pointerDragStep(drag, 2, detected, 150 + offset, 50, drawn, big);
      expect(first.margins).toEqual(detected);
      expect(pointerDragStep(first.drag, 2, detected, 5000, 50, drawn, big).margins).toEqual({ ...detected, right: 0 });
    }
  });

  it("still moves the grabbed edge of a collapsed box after a rebase", () => {
    const thin: CropMargins = { top: 1535, bottom: 1536, left: 0, right: 0 };
    const drag = startDrag(0, "n", ZERO_MARGINS, 100, 10);
    const first = pointerDragStep(drag, 1, thin, 100, 78, drawn, big);
    expect(first.margins).toEqual(thin);
    expect(pointerDragStep(first.drag, 1, thin, 100, 77, drawn, big).margins).toEqual({ ...thin, top: 1515 });
    expect(pointerDragStep(first.drag, 1, thin, 100, 90, drawn, big).margins).toEqual(thin);
  });

  it("behaves like dragMargins while the revision does not change", () => {
    const drag = startDrag(3, "body", detected, 104, 78);
    expect(pointerDragStep(drag, 3, detected, -5000, -5000, drawn, big).margins).toEqual(
      dragMargins(drag.start, -5000, -5000, drawn, big),
    );
  });
});

describe("suppressTextSelection", () => {
  it("turns text selection off and puts the previous value back once", () => {
    const style = { userSelect: "text" };
    const restore = suppressTextSelection(style);
    expect(style.userSelect).toBe("none");
    restore();
    expect(style.userSelect).toBe("text");
    style.userSelect = "auto";
    restore();
    expect(style.userSelect).toBe("auto");
  });

  it("restores the empty inline value used when nothing was set", () => {
    const style = { userSelect: "" };
    const restore = suppressTextSelection(style);
    expect(style.userSelect).toBe("none");
    restore();
    expect(style.userSelect).toBe("");
  });
});

describe("cropOverlayKeys", () => {
  it("draws the first keys and masks only the aligned keys not already drawn", () => {
    const keys = ["a", "b", "c", "d", "e"];
    expect(cropOverlayKeys(keys, MAX_OVERLAY_CHANNELS)).toEqual({ keys: ["a", "b", "c"], maskKeys: ["d", "e"] });
  });

  it("has no mask keys when every aligned key is drawn", () => {
    expect(cropOverlayKeys(["a", "b"], MAX_OVERLAY_CHANNELS)).toEqual({ keys: ["a", "b"], maskKeys: [] });
    expect(cropOverlayKeys(["a", "b", "c"], MAX_OVERLAY_CHANNELS)).toEqual({ keys: ["a", "b", "c"], maskKeys: [] });
  });

  it("does not repeat a key in the mask", () => {
    expect(cropOverlayKeys(["a", "b", "c", "a", "d", "d"], 3)).toEqual({ keys: ["a", "b", "c"], maskKeys: ["d"] });
  });
});

describe("editor box sizing", () => {
  it("uses the measured height between the minimum and the cap", () => {
    expect(editorBoxHeight(300)).toBe(300);
    expect(editorBoxHeight(300.7)).toBe(300);
    expect(editorBoxHeight(100)).toBe(EDITOR_MIN_HEIGHT_PX);
    expect(editorBoxHeight(2000)).toBe(EDITOR_MAX_HEIGHT_PX);
    expect(EDITOR_MIN_HEIGHT_PX).toBe(156);
    expect(EDITOR_MAX_HEIGHT_PX).toBe(520);
  });

  it("falls back to the minimum before the area is measured", () => {
    expect(editorBoxHeight(0)).toBe(EDITOR_MIN_HEIGHT_PX);
    expect(editorBoxHeight(Number.NaN)).toBe(EDITOR_MIN_HEIGHT_PX);
  });

  it("fills the height of a wide area and keeps the preview aspect", () => {
    const fitted = editorBoxSize({ width: 1024, height: 768 }, { width: 1180, height: 400 });
    expect(fitted.height).toBeCloseTo(400, 10);
    expect(fitted.width).toBeCloseTo(400 * (4 / 3), 10);
    expect(editorNaturalWidth({ width: 1024, height: 768 }, 400)).toBeCloseTo(400 * (4 / 3), 10);
  });

  it("is limited by the width of a narrow area", () => {
    expect(editorBoxSize({ width: 1024, height: 256 }, { width: 300, height: 400 })).toEqual({ width: 300, height: 75 });
  });

  it("caps a tall area at the maximum height", () => {
    expect(editorBoxSize({ width: 1000, height: 1000 }, { width: 2000, height: 900 })).toEqual({ width: 520, height: 520 });
  });

  it("uses the natural width at the fallback height before the area is measured", () => {
    expect(editorBoxSize({ width: 1024, height: 768 }, { width: 0, height: 0 })).toEqual({ width: 208, height: 156 });
  });
});

describe("cropHitLayout", () => {
  const sizes = { handlePx: 9, stripPx: 8, bodyPx: 10 };
  const big: GridSize = { width: 4096, height: 3072 };
  const shown = { width: 208, height: 156 };
  const unit = unitsPerScreenPx(big, shown);

  const eps = 1e-6;

  function centre(hit: HitRect) {
    return { x: hit.x + hit.width / 2, y: hit.y + hit.height / 2 };
  }

  function expectCentre(hit: HitRect, x: number, y: number) {
    expect(centre(hit).x).toBeCloseTo(x, 6);
    expect(centre(hit).y).toBeCloseTo(y, 6);
  }

  function overlaps(a: HitRect, b: HitRect): boolean {
    return (
      a.x + eps < b.x + b.width && b.x + eps < a.x + a.width && a.y + eps < b.y + b.height && b.y + eps < a.y + a.height
    );
  }

  function toScreen(hit: HitRect): HitRect {
    return { x: hit.x / unit.x, y: hit.y / unit.y, width: hit.width / unit.x, height: hit.height / unit.y };
  }

  it("centres every handle on its corner or edge midpoint for a box with room", () => {
    const rect = { x0: 400, y0: 300, x1: 3600, y1: 2700 };
    const hits = cropHitLayout(rect, unit, sizes);
    expectCentre(hits.handles.nw, 400, 300);
    expectCentre(hits.handles.n, 2000, 300);
    expectCentre(hits.handles.ne, 3600, 300);
    expectCentre(hits.handles.e, 3600, 1500);
    expectCentre(hits.handles.se, 3600, 2700);
    expectCentre(hits.handles.s, 2000, 2700);
    expectCentre(hits.handles.sw, 400, 2700);
    expectCentre(hits.handles.w, 400, 1500);
    for (const handle of CROP_HANDLES) {
      expect(toScreen(hits.handles[handle]).width).toBeCloseTo(9, 10);
      expect(toScreen(hits.handles[handle]).height).toBeCloseTo(9, 10);
    }
  });

  it("uses the box itself as the body and straddles each edge with its strip for a box with room", () => {
    const rect = { x0: 400, y0: 300, x1: 3600, y1: 2700 };
    const hits = cropHitLayout(rect, unit, sizes);
    expect(hits.body).toEqual({ x: 400, y: 300, width: 3200, height: 2400 });
    expect(centre(hits.strips.n).y).toBeCloseTo(300, 6);
    expect(centre(hits.strips.s).y).toBeCloseTo(2700, 6);
    expect(centre(hits.strips.w).x).toBeCloseTo(400, 6);
    expect(centre(hits.strips.e).x).toBeCloseTo(3600, 6);
    expect(hits.strips.n.x).toBe(400);
    expect(hits.strips.n.width).toBe(3200);
  });

  it("keeps every handle, strip pair and the body apart when the box is collapsed to one row and one column", () => {
    const rect = { x0: 2000, y0: 1500, x1: 2001, y1: 1501 };
    const hits = cropHitLayout(rect, unit, sizes);
    const handles = CROP_HANDLES.map((handle) => hits.handles[handle]);
    for (let i = 0; i < handles.length; i += 1) {
      expect(overlaps(handles[i], hits.body)).toBe(false);
      for (let j = i + 1; j < handles.length; j += 1) {
        expect(overlaps(handles[i], handles[j])).toBe(false);
      }
    }
    expect(overlaps(hits.strips.n, hits.strips.s)).toBe(false);
    expect(overlaps(hits.strips.w, hits.strips.e)).toBe(false);
    expect(overlaps(hits.strips.n, hits.body)).toBe(false);
    expect(overlaps(hits.strips.w, hits.body)).toBe(false);
  });

  it("puts each strip of a collapsed box against its own side of the body, spanning the body", () => {
    const rect = { x0: 2000, y0: 1500, x1: 2001, y1: 1501 };
    const { body, strips } = cropHitLayout(rect, unit, sizes);
    const bodyRight = body.x + body.width;
    const bodyBottom = body.y + body.height;
    expect(strips.n.y + strips.n.height).toBeCloseTo(body.y, 6);
    expect(strips.s.y).toBeCloseTo(bodyBottom, 6);
    expect(strips.w.x + strips.w.width).toBeCloseTo(body.x, 6);
    expect(strips.e.x).toBeCloseTo(bodyRight, 6);
    for (const strip of [strips.n, strips.s]) {
      expect(strip.x).toBeCloseTo(body.x, 6);
      expect(strip.width).toBeCloseTo(body.width, 6);
      expect(toScreen(strip).height).toBeCloseTo(8, 10);
    }
    for (const strip of [strips.w, strips.e]) {
      expect(strip.y).toBeCloseTo(body.y, 6);
      expect(strip.height).toBeCloseTo(body.height, 6);
      expect(toScreen(strip).width).toBeCloseTo(8, 10);
    }
    for (const strip of [strips.n, strips.s, strips.w, strips.e]) {
      expect(overlaps(strip, body)).toBe(false);
    }
  });

  it("gives a collapsed body at least the minimum on-screen size, centred on the box", () => {
    const rect = { x0: 2000, y0: 1500, x1: 2001, y1: 1501 };
    const body = toScreen(cropHitLayout(rect, unit, sizes).body);
    expect(body.width).toBeCloseTo(10, 10);
    expect(body.height).toBeCloseTo(10, 10);
    expectCentre(cropHitLayout(rect, unit, sizes).body, 2000.5, 1500.5);
  });

  it("puts the top handles above the bottom ones and the left handles left of the right ones when collapsed", () => {
    const rect = { x0: 0, y0: 39, x1: 4096, y1: 40 };
    const hits = cropHitLayout(rect, unit, sizes);
    expect(hits.handles.n.y + hits.handles.n.height).toBeLessThanOrEqual(hits.body.y + eps);
    expect(hits.handles.s.y).toBeGreaterThanOrEqual(hits.body.y + hits.body.height - eps);
    expect(hits.handles.nw.y + hits.handles.nw.height).toBeLessThanOrEqual(hits.handles.sw.y + eps);
    expect(hits.handles.ne.y + hits.handles.ne.height).toBeLessThanOrEqual(hits.handles.se.y + eps);
    expect(overlaps(hits.handles.n, hits.handles.s)).toBe(false);
    expect(overlaps(hits.strips.n, hits.strips.s)).toBe(false);
    expect(centre(hits.handles.w).x).toBeCloseTo(0, 6);
    expect(centre(hits.handles.e).x).toBeCloseTo(4096, 6);
  });

  it("keeps the body grabbable between the handles of a box a few screen pixels tall", () => {
    const rect = { x0: 0, y0: 1000, x1: 4096, y1: 1000 + Math.round(12 * unit.y) };
    const hits = cropHitLayout(rect, unit, sizes);
    const coreTop = (rect.y0 + rect.y1) / 2 - 5 * unit.y;
    const coreBottom = (rect.y0 + rect.y1) / 2 + 5 * unit.y;
    expect(hits.handles.n.y + hits.handles.n.height).toBeLessThanOrEqual(coreTop + eps);
    expect(hits.handles.s.y).toBeGreaterThanOrEqual(coreBottom - eps);
    expect(hits.body.y).toBe(rect.y0);
    expect(hits.body.height).toBe(rect.y1 - rect.y0);
  });

  it("collapses to the box when the box has no on-screen size yet", () => {
    const rect = { x0: 10, y0: 20, x1: 30, y1: 60 };
    const hits = cropHitLayout(rect, { x: 0, y: 0 }, sizes);
    expect(hits.body).toEqual({ x: 10, y: 20, width: 20, height: 40 });
    expectCentre(hits.handles.se, 30, 60);
  });
});

describe("handle tables", () => {
  it("lists the eight distinct handles, each with a label and a cursor", () => {
    expect(new Set(CROP_HANDLES).size).toBe(8);
    for (const handle of CROP_HANDLES) {
      expect(CROP_TARGET_LABELS[handle]).toBeTruthy();
      expect(CROP_TARGET_CURSORS[handle]).toMatch(/resize$/);
    }
    expect(CROP_TARGET_LABELS.n).toBe("Top edge");
    expect(CROP_TARGET_LABELS.nw).toBe("Top-left corner");
    expect(CROP_TARGET_LABELS.body).toBe("Crop area");
    expect(CROP_TARGET_CURSORS.body).toBe("move");
  });

  it("lists the four edge strips and describes the handle roles", () => {
    expect([...CROP_EDGE_HANDLES].sort()).toEqual(["e", "n", "s", "w"]);
    expect(CROP_HANDLE_ROLE_DESCRIPTION).toBe("resize handle");
    expect(CROP_BODY_ROLE_DESCRIPTION).toBe("move handle");
  });
});

describe("sameMargins", () => {
  it("compares the four margins", () => {
    expect(sameMargins(margins, { ...margins })).toBe(true);
    expect(sameMargins(margins, { ...margins, right: 16 })).toBe(false);
  });
});

describe("cropSizeLabel", () => {
  it("shows the full grid and the kept size", () => {
    expect(cropSizeLabel(margins, grid)).toBe("100×80 → 80×50");
    expect(cropSizeLabel(ZERO_MARGINS, grid)).toBe("100×80 → 100×80");
  });
});

describe("fitPreviewBox", () => {
  it("is limited by height for a wide enough panel", () => {
    expect(fitPreviewBox(1024, 768, 1000, 156)).toEqual({ width: 208, height: 156 });
  });

  it("is limited by width for a narrow panel", () => {
    expect(fitPreviewBox(1024, 256, 300, 156)).toEqual({ width: 300, height: 75 });
  });

  it("keeps the preview aspect exactly", () => {
    const fitted = fitPreviewBox(1024, 683, 500, 156);
    expect(fitted.width / fitted.height).toBeCloseTo(1024 / 683, 10);
  });

  it("scales small previews up", () => {
    expect(fitPreviewBox(64, 64, 1000, 156)).toEqual({ width: 156, height: 156 });
  });

  it("gives an empty box for invalid sizes", () => {
    expect(fitPreviewBox(0, 100, 300, 156)).toEqual({ width: 0, height: 0 });
    expect(fitPreviewBox(100, 100, 0, 156)).toEqual({ width: 0, height: 0 });
  });
});

describe("unitsPerScreenPx", () => {
  it("gives grid pixels per screen pixel on each axis", () => {
    expect(unitsPerScreenPx({ width: 4096, height: 3072 }, { width: 208, height: 156 })).toEqual({
      x: 4096 / 208,
      y: 3072 / 156,
    });
  });

  it("gives 0 for an empty box", () => {
    expect(unitsPerScreenPx(grid, { width: 0, height: 0 })).toEqual({ x: 0, y: 0 });
  });
});
