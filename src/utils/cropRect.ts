export interface CropMargins {
  top: number;
  bottom: number;
  left: number;
  right: number;
}

export interface CropRect {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

export interface GridSize {
  width: number;
  height: number;
}

export interface EdgePoint {
  x: number;
  y: number;
}

export interface ScreenSize {
  width: number;
  height: number;
}

export interface HitRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface CropBoundsLike {
  crop_top: number;
  crop_bottom: number;
  crop_left: number;
  crop_right: number;
}

export type MarginEdge = keyof CropMargins;
export type CropHandle = "n" | "s" | "e" | "w" | "ne" | "nw" | "se" | "sw";
export type CropEdgeHandle = "n" | "s" | "e" | "w";
export type CropTarget = CropHandle | "body";
export type MarginSource = "auto" | "manual";

export interface MarginDraft {
  margins: CropMargins;
  source: MarginSource;
  revision: number;
  typedEdges: readonly MarginEdge[];
}

export interface PointerDragStart {
  target: CropTarget;
  margins: CropMargins;
  clientX: number;
  clientY: number;
}

export interface PointerDrag {
  revision: number;
  start: PointerDragStart;
}

export interface CropHitSizes {
  handlePx: number;
  stripPx: number;
  bodyPx: number;
}

export interface CropHitLayout {
  body: HitRect;
  strips: Record<CropEdgeHandle, HitRect>;
  handles: Record<CropHandle, HitRect>;
}

export interface TextSelectionStyle {
  userSelect: string;
}

export const ZERO_MARGINS: CropMargins = { top: 0, bottom: 0, left: 0, right: 0 };

export const INITIAL_MARGIN_DRAFT: MarginDraft = {
  margins: ZERO_MARGINS,
  source: "auto",
  revision: 0,
  typedEdges: [],
};

export const MARGIN_EDGES: readonly MarginEdge[] = ["top", "bottom", "left", "right"];

export const CROP_HANDLES: readonly CropHandle[] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];

export const CROP_EDGE_HANDLES: readonly CropEdgeHandle[] = ["n", "s", "w", "e"];

export const CROP_TARGET_LABELS: Record<CropTarget, string> = {
  n: "Top edge",
  s: "Bottom edge",
  e: "Right edge",
  w: "Left edge",
  nw: "Top-left corner",
  ne: "Top-right corner",
  sw: "Bottom-left corner",
  se: "Bottom-right corner",
  body: "Crop area",
};

export const CROP_TARGET_CURSORS: Record<CropTarget, string> = {
  n: "ns-resize",
  s: "ns-resize",
  e: "ew-resize",
  w: "ew-resize",
  nw: "nwse-resize",
  se: "nwse-resize",
  ne: "nesw-resize",
  sw: "nesw-resize",
  body: "move",
};

export const CROP_HANDLE_ROLE_DESCRIPTION = "resize handle";
export const CROP_BODY_ROLE_DESCRIPTION = "move handle";

export const KEY_STEP_PX = 1;
export const KEY_STEP_FAST_PX = 10;

export const EDITOR_MIN_HEIGHT_PX = 156;
export const EDITOR_MAX_HEIGHT_PX = 520;

const HANDLE_EDGES: Record<CropHandle, { vertical: "top" | "bottom" | null; horizontal: "left" | "right" | null }> = {
  n: { vertical: "top", horizontal: null },
  s: { vertical: "bottom", horizontal: null },
  e: { vertical: null, horizontal: "right" },
  w: { vertical: null, horizontal: "left" },
  nw: { vertical: "top", horizontal: "left" },
  ne: { vertical: "top", horizontal: "right" },
  sw: { vertical: "bottom", horizontal: "left" },
  se: { vertical: "bottom", horizontal: "right" },
};

const OPPOSITE_EDGE: Record<MarginEdge, MarginEdge> = {
  top: "bottom",
  bottom: "top",
  left: "right",
  right: "left",
};

function clampNumber(value: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, value));
}

function wholeNonNegative(value: number): number {
  return Number.isFinite(value) ? Math.max(0, Math.round(value)) : 0;
}

function roundedOr(value: number, fallback: number): number {
  return Number.isFinite(value) ? Math.round(value) : fallback;
}

function edgeSpan(edge: MarginEdge, grid: GridSize): number {
  return edge === "top" || edge === "bottom" ? grid.height : grid.width;
}

function lastTypedIsNear(typedEdges: readonly MarginEdge[], near: MarginEdge, far: MarginEdge): boolean {
  for (let i = typedEdges.length - 1; i >= 0; i -= 1) {
    if (typedEdges[i] === near) return true;
    if (typedEdges[i] === far) return false;
  }
  return false;
}

function clampEdgePair(near: number, far: number, span: number, nearYields: boolean): [number, number] {
  if (nearYields) {
    const keptFar = Math.min(far, span - 1);
    return [Math.min(near, span - keptFar - 1), keptFar];
  }
  const keptNear = Math.min(near, span - 1);
  return [keptNear, Math.min(far, span - keptNear - 1)];
}

export function gridSize(dimensions: readonly [number, number] | null | undefined): GridSize | null {
  if (!dimensions) return null;
  const [width, height] = dimensions;
  if (!Number.isInteger(width) || !Number.isInteger(height) || width < 1 || height < 1) return null;
  return { width, height };
}

export function clientDeltaToGrid(delta: number, length: number, size: number): number {
  if (!(length > 0) || !Number.isFinite(delta)) return 0;
  return Math.round((delta / length) * size);
}

export function rectFromMargins(margins: CropMargins, grid: GridSize): CropRect {
  return {
    x0: margins.left,
    y0: margins.top,
    x1: grid.width - margins.right,
    y1: grid.height - margins.bottom,
  };
}

export function marginsFromRect(rect: CropRect, grid: GridSize): CropMargins {
  return {
    top: rect.y0,
    bottom: grid.height - rect.y1,
    left: rect.x0,
    right: grid.width - rect.x1,
  };
}

export function marginsFromBounds(bounds: CropBoundsLike): CropMargins {
  return {
    top: bounds.crop_top,
    bottom: bounds.crop_bottom,
    left: bounds.crop_left,
    right: bounds.crop_right,
  };
}

export function clampMargins(
  margins: CropMargins,
  grid: GridSize | null,
  typedEdges: readonly MarginEdge[] = [],
): CropMargins {
  const top = wholeNonNegative(margins.top);
  const bottom = wholeNonNegative(margins.bottom);
  const left = wholeNonNegative(margins.left);
  const right = wholeNonNegative(margins.right);
  if (!grid) return { top, bottom, left, right };
  if (grid.width < 1 || grid.height < 1) return { ...ZERO_MARGINS };
  const [clampedTop, clampedBottom] = clampEdgePair(top, bottom, grid.height, lastTypedIsNear(typedEdges, "top", "bottom"));
  const [clampedLeft, clampedRight] = clampEdgePair(left, right, grid.width, lastTypedIsNear(typedEdges, "left", "right"));
  return { top: clampedTop, bottom: clampedBottom, left: clampedLeft, right: clampedRight };
}

export function marginEdgeMax(margins: CropMargins, edge: MarginEdge, grid: GridSize): number {
  return Math.max(0, edgeSpan(edge, grid) - margins[OPPOSITE_EDGE[edge]] - 1);
}

export function setMarginEdge(
  margins: CropMargins,
  edge: MarginEdge,
  value: number,
  grid: GridSize | null,
): CropMargins {
  const base = clampMargins(margins, grid);
  const wanted = wholeNonNegative(value);
  if (!grid) return { ...base, [edge]: wanted };
  return { ...base, [edge]: Math.min(wanted, marginEdgeMax(base, edge, grid)) };
}

export function dragHandle(start: CropMargins, handle: CropHandle, point: EdgePoint, grid: GridSize): CropMargins {
  const rect = rectFromMargins(clampMargins(start, grid), grid);
  const edges = HANDLE_EDGES[handle];
  const next = { ...rect };
  if (edges.vertical === "top") next.y0 = clampNumber(roundedOr(point.y, rect.y0), 0, rect.y1 - 1);
  if (edges.vertical === "bottom") next.y1 = clampNumber(roundedOr(point.y, rect.y1), rect.y0 + 1, grid.height);
  if (edges.horizontal === "left") next.x0 = clampNumber(roundedOr(point.x, rect.x0), 0, rect.x1 - 1);
  if (edges.horizontal === "right") next.x1 = clampNumber(roundedOr(point.x, rect.x1), rect.x0 + 1, grid.width);
  return marginsFromRect(next, grid);
}

export function moveBody(start: CropMargins, dx: number, dy: number, grid: GridSize): CropMargins {
  const rect = rectFromMargins(clampMargins(start, grid), grid);
  const width = rect.x1 - rect.x0;
  const height = rect.y1 - rect.y0;
  const x0 = clampNumber(rect.x0 + roundedOr(dx, 0), 0, grid.width - width);
  const y0 = clampNumber(rect.y0 + roundedOr(dy, 0), 0, grid.height - height);
  return marginsFromRect({ x0, y0, x1: x0 + width, y1: y0 + height }, grid);
}

export function arrowDelta(key: string, fast: boolean): { dx: number; dy: number } | null {
  const step = fast ? KEY_STEP_FAST_PX : KEY_STEP_PX;
  switch (key) {
    case "ArrowUp":
      return { dx: 0, dy: -step };
    case "ArrowDown":
      return { dx: 0, dy: step };
    case "ArrowLeft":
      return { dx: -step, dy: 0 };
    case "ArrowRight":
      return { dx: step, dy: 0 };
    default:
      return null;
  }
}

export function nudgeTarget(margins: CropMargins, target: CropTarget, dx: number, dy: number, grid: GridSize): CropMargins {
  if (target === "body") return moveBody(margins, dx, dy, grid);
  const rect = rectFromMargins(clampMargins(margins, grid), grid);
  const edges = HANDLE_EDGES[target];
  const x = edges.horizontal === "left" ? rect.x0 : rect.x1;
  const y = edges.vertical === "top" ? rect.y0 : rect.y1;
  return dragHandle(margins, target, { x: x + dx, y: y + dy }, grid);
}

export function dragMargins(
  start: PointerDragStart,
  clientX: number,
  clientY: number,
  box: ScreenSize,
  grid: GridSize,
): CropMargins {
  const dx = clientDeltaToGrid(clientX - start.clientX, box.width, grid.width);
  const dy = clientDeltaToGrid(clientY - start.clientY, box.height, grid.height);
  return nudgeTarget(start.margins, start.target, dx, dy, grid);
}

export function rebaseDrag(
  drag: PointerDrag,
  revision: number,
  margins: CropMargins,
  clientX: number,
  clientY: number,
): PointerDrag {
  if (drag.revision === revision) return drag;
  return { revision, start: { target: drag.start.target, margins, clientX, clientY } };
}

export function pointerDragStep(
  drag: PointerDrag,
  revision: number,
  margins: CropMargins,
  clientX: number,
  clientY: number,
  box: ScreenSize,
  grid: GridSize,
): { drag: PointerDrag; margins: CropMargins } {
  const current = rebaseDrag(drag, revision, margins, clientX, clientY);
  return { drag: current, margins: dragMargins(current.start, clientX, clientY, box, grid) };
}

export function suppressTextSelection(style: TextSelectionStyle): () => void {
  const previous = style.userSelect;
  style.userSelect = "none";
  let restored = false;
  return () => {
    if (restored) return;
    restored = true;
    style.userSelect = previous;
  };
}

export function replaceDraft(
  previous: MarginDraft,
  margins: CropMargins,
  source: MarginSource,
  typedEdges: readonly MarginEdge[] = [],
): MarginDraft {
  return { margins, source, revision: previous.revision + 1, typedEdges };
}

export function prefillDraft(previous: MarginDraft, margins: CropMargins): MarginDraft {
  return previous.source === "auto" ? replaceDraft(previous, margins, "auto") : previous;
}

export function editDraft(previous: MarginDraft, margins: CropMargins, basedOnRevision: number): MarginDraft {
  if (previous.revision !== basedOnRevision) return previous;
  return { margins, source: "manual", revision: previous.revision, typedEdges: [] };
}

export function draftMargins(draft: MarginDraft, grid: GridSize | null): CropMargins {
  return clampMargins(draft.margins, grid, draft.typedEdges);
}

export function typeMarginDraft(previous: MarginDraft, margins: CropMargins, edge: MarginEdge): MarginDraft {
  return replaceDraft(previous, margins, "manual", [...previous.typedEdges.filter((typed) => typed !== edge), edge]);
}

export function applyWaitsForDetection(draft: MarginDraft, detecting: boolean): boolean {
  return detecting && draft.source === "auto";
}

export function cropOverlayKeys(
  alignedKeys: readonly string[],
  maxOverlayKeys: number,
): { keys: string[]; maskKeys: string[] } {
  const keys = alignedKeys.slice(0, Math.max(0, maxOverlayKeys));
  const maskKeys = alignedKeys.filter((key, i) => !keys.includes(key) && alignedKeys.indexOf(key) === i);
  return { keys, maskKeys };
}

interface AxisHits {
  mid: number;
  bodyLo: number;
  bodyHi: number;
  nearHandle: number;
  farHandle: number;
  nearStrip: number;
  farStrip: number;
}

function axisHits(lo: number, hi: number, unit: number, sizes: CropHitSizes): AxisHits {
  const mid = (lo + hi) / 2;
  const bodyHalf = (sizes.bodyPx * unit) / 2;
  const handleHalf = (sizes.handlePx * unit) / 2;
  const stripHalf = (sizes.stripPx * unit) / 2;
  return {
    mid,
    bodyLo: Math.min(lo, mid - bodyHalf),
    bodyHi: Math.max(hi, mid + bodyHalf),
    nearHandle: Math.min(lo, mid - bodyHalf - handleHalf),
    farHandle: Math.max(hi, mid + bodyHalf + handleHalf),
    nearStrip: Math.min(lo, mid - bodyHalf - stripHalf),
    farStrip: Math.max(hi, mid + bodyHalf + stripHalf),
  };
}

function axisHandleCentre(hits: AxisHits, side: "near" | "far" | null): number {
  if (side === "near") return hits.nearHandle;
  if (side === "far") return hits.farHandle;
  return hits.mid;
}

export function cropHitLayout(rect: CropRect, unit: EdgePoint, sizes: CropHitSizes): CropHitLayout {
  const hx = axisHits(rect.x0, rect.x1, unit.x, sizes);
  const hy = axisHits(rect.y0, rect.y1, unit.y, sizes);
  const handleWidth = sizes.handlePx * unit.x;
  const handleHeight = sizes.handlePx * unit.y;
  const stripWidth = sizes.stripPx * unit.x;
  const stripHeight = sizes.stripPx * unit.y;
  const bodyWidth = hx.bodyHi - hx.bodyLo;
  const bodyHeight = hy.bodyHi - hy.bodyLo;
  const handles = {} as Record<CropHandle, HitRect>;
  for (const handle of CROP_HANDLES) {
    const edges = HANDLE_EDGES[handle];
    const cx = axisHandleCentre(hx, edges.horizontal === "left" ? "near" : edges.horizontal === "right" ? "far" : null);
    const cy = axisHandleCentre(hy, edges.vertical === "top" ? "near" : edges.vertical === "bottom" ? "far" : null);
    handles[handle] = { x: cx - handleWidth / 2, y: cy - handleHeight / 2, width: handleWidth, height: handleHeight };
  }
  return {
    body: { x: hx.bodyLo, y: hy.bodyLo, width: bodyWidth, height: bodyHeight },
    strips: {
      n: { x: hx.bodyLo, y: hy.nearStrip - stripHeight / 2, width: bodyWidth, height: stripHeight },
      s: { x: hx.bodyLo, y: hy.farStrip - stripHeight / 2, width: bodyWidth, height: stripHeight },
      w: { x: hx.nearStrip - stripWidth / 2, y: hy.bodyLo, width: stripWidth, height: bodyHeight },
      e: { x: hx.farStrip - stripWidth / 2, y: hy.bodyLo, width: stripWidth, height: bodyHeight },
    },
    handles,
  };
}

export function sameMargins(a: CropMargins, b: CropMargins): boolean {
  return a.top === b.top && a.bottom === b.bottom && a.left === b.left && a.right === b.right;
}

export function croppedSize(margins: CropMargins, grid: GridSize): GridSize {
  return {
    width: grid.width - margins.left - margins.right,
    height: grid.height - margins.top - margins.bottom,
  };
}

export function cropSizeLabel(margins: CropMargins, grid: GridSize): string {
  const kept = croppedSize(margins, grid);
  return `${grid.width}×${grid.height} → ${kept.width}×${kept.height}`;
}

export function fitPreviewBox(
  previewWidth: number,
  previewHeight: number,
  maxWidth: number,
  maxHeight: number,
): { width: number; height: number } {
  if (!(previewWidth > 0) || !(previewHeight > 0) || !(maxWidth > 0) || !(maxHeight > 0)) {
    return { width: 0, height: 0 };
  }
  const scale = Math.min(maxWidth / previewWidth, maxHeight / previewHeight);
  return { width: previewWidth * scale, height: previewHeight * scale };
}

export function editorBoxHeight(availableHeight: number): number {
  if (!Number.isFinite(availableHeight) || availableHeight <= 0) return EDITOR_MIN_HEIGHT_PX;
  return clampNumber(Math.floor(availableHeight), EDITOR_MIN_HEIGHT_PX, EDITOR_MAX_HEIGHT_PX);
}

export function editorNaturalWidth(aspect: GridSize, availableHeight: number): number {
  return (editorBoxHeight(availableHeight) * aspect.width) / aspect.height;
}

export function editorBoxSize(aspect: GridSize, area: ScreenSize): { width: number; height: number } {
  const maxWidth = area.width > 0 ? area.width : editorNaturalWidth(aspect, area.height);
  return fitPreviewBox(aspect.width, aspect.height, maxWidth, editorBoxHeight(area.height));
}

export function unitsPerScreenPx(grid: GridSize, box: ScreenSize): EdgePoint {
  return {
    x: box.width > 0 ? grid.width / box.width : 0,
    y: box.height > 0 ? grid.height / box.height : 0,
  };
}
