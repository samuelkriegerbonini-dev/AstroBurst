import { DOCK_TOOL_IDS, type DockToolId } from "./dockLayout";
import { DOCK_LAYOUT_STORAGE_KEY } from "./dockPersistence";

export const TOOL_CATEGORIES = [
  { id: "inspect", label: "Inspect", color: "#67e8f9", tools: ["image", "headers", "info"] },
  { id: "analyze", label: "Measure", color: "#c4b5fd", tools: ["astrometry", "photometry", "cube", "sagui", "log"] },
  { id: "process", label: "Enhance", color: "#fcd34d", tools: ["processing", "masks", "history"] },
  { id: "create", label: "Combine", color: "#f9a8d4", tools: ["stacking", "compose", "cubeprep", "synth"] },
  { id: "workspace", label: "Files and output", color: "#5eead4", tools: ["files", "export", "config"] },
] as const satisfies readonly { id: string; label: string; color: string; tools: readonly DockToolId[] }[];

export const TOOL_DESCRIPTIONS: Record<DockToolId, string> = {
  files: "Browse imported images, filter the list and select a file to work on.",
  info: "Inspect the selected file's instrument, exposure and metadata.",
  compose: "Assign channels and combine images into a color composite.",
  headers: "Read FITS headers, select an HDU and inspect WCS coordinates.",
  image: "Explore histograms, statistics, pixels, regions and image profiles.",
  astrometry: "Detect stars, solve sky coordinates and compare catalogs.",
  photometry: "Measure source flux with apertures and build light curves.",
  cube: "Explore cube slices, extract spectra and measure spectral lines.",
  cubeprep: "Prepare multiband images with unit conversion, alignment and PSF matching.",
  sagui: "Segment multiband images into regions and explore their spectral energy distributions.",
  processing: "Remove background, stretch contrast and enhance image detail.",
  history: "Review processing steps, undo changes or rerun a previous operation.",
  masks: "Build masks to limit processing to selected parts of an image.",
  stacking: "Calibrate, align and combine exposures into a cleaner image.",
  synth: "Generate synthetic astronomical images with controlled source and noise settings.",
  export: "Save preview images, FITS products and associated data.",
  config: "Configure display defaults, catalogs, downloads and output preferences.",
  log: "Review recorded measurements and export them as a table.",
};

export const DEFAULT_TOOL_SHORTCUTS: readonly DockToolId[] = ["files", "info", "compose", "headers", "image", "astrometry", "photometry", "cube", "processing", "history", "export", "config", "log"];
export const TOOL_SHORTCUTS_KEY = "ab.tools.shortcuts.v1";

export type ToolShortcutsReader = Pick<Storage, "getItem">;

export function parseToolShortcuts(raw: string | null): DockToolId[] {
  if (raw === null) return [...DEFAULT_TOOL_SHORTCUTS];
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [...DEFAULT_TOOL_SHORTCUTS];
    return DOCK_TOOL_IDS.filter((tool) => parsed.includes(tool));
  } catch {
    return [...DEFAULT_TOOL_SHORTCUTS];
  }
}

export function initialToolShortcuts(storage: ToolShortcutsReader | null): DockToolId[] {
  if (storage === null) return [...DEFAULT_TOOL_SHORTCUTS];
  try {
    const raw = storage.getItem(TOOL_SHORTCUTS_KEY);
    if (raw === null && storage.getItem(DOCK_LAYOUT_STORAGE_KEY) !== null) return [...DOCK_TOOL_IDS];
    return parseToolShortcuts(raw);
  } catch {
    return [...DEFAULT_TOOL_SHORTCUTS];
  }
}

export function toolCategoryColor(tool: DockToolId): string {
  return TOOL_CATEGORIES.find((category) => (category.tools as readonly DockToolId[]).includes(tool))?.color ?? "var(--ab-text-2)";
}

export function toggleToolShortcut(enabled: readonly DockToolId[], tool: DockToolId, visible: boolean): DockToolId[] {
  return visible ? enabled.filter((id) => id !== tool) : DOCK_TOOL_IDS.filter((id) => id === tool || enabled.includes(id));
}
