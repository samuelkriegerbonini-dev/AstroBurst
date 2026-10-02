export const DOCK_TOOL_IDS = ["files", "info", "compose", "headers", "analysis", "processing", "stacking", "synth", "export", "config"] as const;
export type DockToolId = (typeof DOCK_TOOL_IDS)[number];
export const DOCK_ANCHORS = ["left-top", "left-bottom", "right-top", "right-bottom"] as const;
export type DockAnchor = (typeof DOCK_ANCHORS)[number];
export type DockSide = "left" | "right";
export type DockOrientation = "vertical" | "horizontal";
export type SideAnchor = "left-top" | "right-top";
export type BottomAnchor = "left-bottom" | "right-bottom";

export const ANCHOR_LABELS: Record<DockAnchor, string> = {
  "left-top": "Left Top",
  "left-bottom": "Bottom Left",
  "right-top": "Right Top",
  "right-bottom": "Bottom Right",
};

export function isDockToolId(value: unknown): value is DockToolId {
  return typeof value === "string" && (DOCK_TOOL_IDS as readonly string[]).includes(value);
}

export function anchorSide(anchor: DockAnchor): DockSide {
  return anchor === "left-top" || anchor === "left-bottom" ? "left" : "right";
}

export function anchorOrientation(anchor: DockAnchor): DockOrientation {
  return anchor === "left-bottom" || anchor === "right-bottom" ? "horizontal" : "vertical";
}

export function otherBottom(anchor: BottomAnchor): BottomAnchor {
  return anchor === "left-bottom" ? "right-bottom" : "left-bottom";
}

export function moveLabel(anchor: DockAnchor): string {
  return `Move to ${ANCHOR_LABELS[anchor]}`;
}

export type DockMountPolicy = "always" | "whileOpen" | "perFile";

export interface DockToolMeta {
  id: DockToolId;
  label: string;
  shortLabel: string;
  accent: string;
  keywords?: string[];
  needsFile: boolean;
  mount: DockMountPolicy;
  minWidth: number;
}

export const DOCK_TOOL_META: Record<DockToolId, DockToolMeta> = {
  files: { id: "files", label: "Files", shortLabel: "Files", accent: "var(--ab-accent)", needsFile: false, mount: "always", minWidth: 180 },
  info: { id: "info", label: "Info", shortLabel: "Info", accent: "var(--ab-teal)", needsFile: false, mount: "whileOpen", minWidth: 180 },
  compose: { id: "compose", label: "Compose", shortLabel: "Comp", accent: "var(--ab-teal)", needsFile: true, mount: "whileOpen", minWidth: 280 },
  headers: { id: "headers", label: "Headers", shortLabel: "Headers", accent: "var(--ab-teal)", needsFile: true, mount: "whileOpen", minWidth: 280 },
  analysis: { id: "analysis", label: "Analysis", shortLabel: "Analysis", accent: "var(--ab-blue)", needsFile: true, mount: "perFile", minWidth: 280 },
  processing: { id: "processing", label: "Processing", shortLabel: "Proc", accent: "var(--ab-amber)", needsFile: true, mount: "whileOpen", minWidth: 280 },
  stacking: { id: "stacking", label: "Stacking", shortLabel: "Stack", accent: "var(--ab-blue)", needsFile: true, mount: "whileOpen", minWidth: 280 },
  synth: { id: "synth", label: "Synth", shortLabel: "Synth", accent: "var(--ab-rose)", needsFile: true, mount: "whileOpen", minWidth: 280 },
  export: { id: "export", label: "Export", shortLabel: "Export", accent: "var(--ab-amber)", needsFile: true, mount: "whileOpen", minWidth: 280 },
  config: { id: "config", label: "Settings", shortLabel: "Config", accent: "#a1a1aa", keywords: ["Config"], needsFile: true, mount: "whileOpen", minWidth: 280 },
};

export interface DockSizes {
  leftW: number;
  rightW: number;
  bottomH: number;
  bottomSplit: number;
}

export interface SizeLimit {
  min: number;
  max: number;
  default: number;
}

export const DOCK_SIZE_LIMITS: Record<keyof DockSizes, SizeLimit> = {
  leftW: { min: 180, max: 640, default: 300 },
  rightW: { min: 280, max: 640, default: 380 },
  bottomH: { min: 140, max: 600, default: 280 },
  bottomSplit: { min: 0.25, max: 0.75, default: 0.5 },
};

const SIZE_KEYS = ["leftW", "rightW", "bottomH", "bottomSplit"] as const;

export const MIN_PREVIEW_W = 320;
export const MIN_PREVIEW_H = 200;
export const COLUMN_VW_CAP_SINGLE = 60;
export const COLUMN_VW_CAP_DOUBLE = 40;

export interface DockLayout {
  anchors: Record<DockAnchor, DockToolId[]>;
  active: Record<DockAnchor, DockToolId | null>;
  sizes: DockSizes;
}

function freezeLayout(layout: DockLayout): DockLayout {
  for (const anchor of DOCK_ANCHORS) Object.freeze(layout.anchors[anchor]);
  Object.freeze(layout.anchors);
  Object.freeze(layout.active);
  Object.freeze(layout.sizes);
  return Object.freeze(layout);
}

export const DEFAULT_DOCK_LAYOUT: DockLayout = freezeLayout({
  anchors: {
    "left-top": ["files", "info"],
    "left-bottom": ["compose"],
    "right-top": ["headers", "analysis", "processing", "stacking"],
    "right-bottom": ["synth", "export", "config"],
  },
  active: { "left-top": "files", "left-bottom": "compose", "right-top": null, "right-bottom": null },
  sizes: { leftW: 300, rightW: 380, bottomH: 280, bottomSplit: 0.5 },
});

export type DockAction =
  | { type: "move"; tool: DockToolId; anchor: DockAnchor; index?: number }
  | { type: "reorder"; anchor: DockAnchor; from: number; to: number }
  | { type: "toggle"; tool: DockToolId }
  | { type: "open"; tool: DockToolId }
  | { type: "hide"; tool: DockToolId }
  | { type: "closeAnchor"; anchor: DockAnchor }
  | { type: "resize"; key: keyof DockSizes; value: number; persist?: boolean }
  | { type: "reset" };

function defaultAnchorOf(tool: DockToolId): DockAnchor {
  return DOCK_ANCHORS.find((a) => DEFAULT_DOCK_LAYOUT.anchors[a].includes(tool)) ?? "left-top";
}

export function anchorOf(layout: DockLayout, tool: DockToolId): DockAnchor {
  return DOCK_ANCHORS.find((a) => layout.anchors[a].includes(tool)) ?? defaultAnchorOf(tool);
}

export function indexOf(layout: DockLayout, tool: DockToolId): number {
  return layout.anchors[anchorOf(layout, tool)].indexOf(tool);
}

export function isToolOpen(layout: DockLayout, tool: DockToolId): boolean {
  return layout.active[anchorOf(layout, tool)] === tool;
}

export function clampSize(key: keyof DockSizes, value: number): number {
  const limit = DOCK_SIZE_LIMITS[key];
  if (Number.isNaN(value)) return limit.default;
  const clamped = Math.min(limit.max, Math.max(limit.min, value));
  return key === "bottomSplit" ? Math.round(clamped * 1000) / 1000 : Math.round(clamped);
}

function sideSizeKey(anchor: SideAnchor): "leftW" | "rightW" {
  return anchor === "left-top" ? "leftW" : "rightW";
}

export function columnMin(layout: DockLayout, anchor: SideAnchor): number {
  const limit = DOCK_SIZE_LIMITS[sideSizeKey(anchor)].min;
  const tool = layout.active[anchor];
  return tool === null ? limit : Math.max(limit, DOCK_TOOL_META[tool].minWidth);
}

export function columnRenderWidth({ size, min, max, dockW, bothOpen }: { size: number; min: number; max: number; dockW: number; bothOpen: boolean }): number {
  const cap = bothOpen ? COLUMN_VW_CAP_DOUBLE : COLUMN_VW_CAP_SINGLE;
  return Math.min(Math.max(size, min), max, (dockW * cap) / 100);
}

function withAnchors(layout: DockLayout, patch: Partial<Record<DockAnchor, DockToolId[]>>): DockLayout["anchors"] {
  return { ...layout.anchors, ...patch };
}

function reorder(layout: DockLayout, anchor: DockAnchor, from: number, to: number): DockLayout {
  const list = layout.anchors[anchor];
  const inBounds = (i: number) => Number.isInteger(i) && i >= 0 && i < list.length;
  if (!inBounds(from) || !inBounds(to) || from === to) return layout;
  const next = list.slice();
  const [tool] = next.splice(from, 1);
  next.splice(to, 0, tool);
  return { ...layout, anchors: withAnchors(layout, { [anchor]: next }) };
}

function clampIndex(index: number, max: number): number {
  return Math.min(Math.max(Math.trunc(index), 0), max);
}

function move(layout: DockLayout, tool: DockToolId, anchor: DockAnchor, index: number | undefined): DockLayout {
  const from = anchorOf(layout, tool);
  const fromIndex = layout.anchors[from].indexOf(tool);
  if (fromIndex < 0) return layout;
  const hasIndex = index !== undefined && Number.isFinite(index);
  if (from === anchor) {
    if (!hasIndex) return layout;
    return reorder(layout, anchor, fromIndex, clampIndex(index, layout.anchors[anchor].length - 1));
  }
  const target = layout.anchors[anchor].slice();
  target.splice(hasIndex ? clampIndex(index, target.length) : target.length, 0, tool);
  const source = layout.anchors[from].filter((t) => t !== tool);
  const active = { ...layout.active, [anchor]: tool };
  if (layout.active[from] === tool) active[from] = null;
  return { ...layout, anchors: withAnchors(layout, { [from]: source, [anchor]: target }), active };
}

function setActive(layout: DockLayout, anchor: DockAnchor, tool: DockToolId | null): DockLayout {
  if (layout.active[anchor] === tool) return layout;
  return { ...layout, active: { ...layout.active, [anchor]: tool } };
}

export function dockReducer(layout: DockLayout, action: DockAction): DockLayout {
  switch (action.type) {
    case "move":
      return move(layout, action.tool, action.anchor, action.index);
    case "reorder":
      return reorder(layout, action.anchor, action.from, action.to);
    case "toggle":
    case "open":
    case "hide": {
      if (indexOf(layout, action.tool) < 0) return layout;
      const anchor = anchorOf(layout, action.tool);
      const current = layout.active[anchor];
      if (action.type === "open") return setActive(layout, anchor, action.tool);
      if (action.type === "hide") return current === action.tool ? setActive(layout, anchor, null) : layout;
      return setActive(layout, anchor, current === action.tool ? null : action.tool);
    }
    case "closeAnchor":
      return setActive(layout, action.anchor, null);
    case "resize": {
      const value = clampSize(action.key, action.value);
      if (layout.sizes[action.key] === value) return layout;
      return { ...layout, sizes: { ...layout.sizes, [action.key]: value } };
    }
    case "reset":
      return DEFAULT_DOCK_LAYOUT;
    default:
      return layout;
  }
}

export function dockLayoutErrors(layout: DockLayout): string[] {
  const errors: string[] = [];
  const counts = new Map<DockToolId, number>();
  for (const anchor of DOCK_ANCHORS) {
    const list: unknown = layout.anchors?.[anchor];
    if (!Array.isArray(list)) {
      errors.push(`anchor ${anchor} has no tool list`);
      continue;
    }
    for (const tool of list) {
      if (isDockToolId(tool)) counts.set(tool, (counts.get(tool) ?? 0) + 1);
      else errors.push(`unknown tool ${String(tool)} at ${anchor}`);
    }
    const active: unknown = layout.active?.[anchor];
    if (active !== null && !list.includes(active)) errors.push(`active tool ${String(active)} at ${anchor} is not docked there`);
  }
  for (const tool of DOCK_TOOL_IDS) {
    const n = counts.get(tool) ?? 0;
    if (n !== 1) errors.push(`tool ${tool} appears ${n} times`);
  }
  for (const key of SIZE_KEYS) {
    const value: unknown = layout.sizes?.[key];
    const { min, max } = DOCK_SIZE_LIMITS[key];
    if (typeof value !== "number" || !Number.isFinite(value) || value < min || value > max) {
      errors.push(`size ${key} ${String(value)} outside ${min}..${max}`);
    }
  }
  return errors;
}

export interface ToolMountInput {
  tool: DockToolId;
  activeTool: DockToolId | null;
  shownTool: DockToolId | null;
  anchorMounted: boolean;
  hasFile: boolean;
  fileKey: string | null;
  keptFileKey: string | null;
  mount: DockMountPolicy;
  needsFile: boolean;
}

export interface ToolMountState {
  mounted: boolean;
  visible: boolean;
  active: boolean;
}

export function toolMountState({ tool, activeTool, shownTool, anchorMounted, hasFile, fileKey, keptFileKey, mount, needsFile }: ToolMountInput): ToolMountState {
  const fileOk = !needsFile || hasFile;
  const showing = anchorMounted ? shownTool : null;
  const visible = fileOk && showing === tool;
  const active = fileOk && activeTool === tool;
  const kept = mount === "perFile" && fileKey !== null && keptFileKey === fileKey;
  const mounted = fileOk && (mount === "always" || visible || active || kept);
  return { mounted, visible, active };
}

export function keptToolFileKey(toolActive: boolean, fileKey: string | null, keptFileKey: string | null): string | null {
  if (toolActive) return fileKey;
  return keptFileKey === fileKey ? keptFileKey : null;
}

export function anchorOpenTool(layout: DockLayout, anchor: DockAnchor, hasFile: boolean): DockToolId | null {
  const tool = layout.active[anchor];
  if (tool === null) return null;
  return DOCK_TOOL_META[tool].needsFile && !hasFile ? null : tool;
}
