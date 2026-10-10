import { detectChannel, headerFilterValues, resolveFileFilter, resolveFilterFromName, shortName, type ChannelSource } from "./channelMapping";

export interface FrequencyBin {
  id: string;
  label: string;
  shortLabel: string;
  wavelength?: number;
  color: string;
  files: string[];
}

export interface BlendWeight {
  channelId: string;
  r: number;
  g: number;
  b: number;
}

export interface SubframeMetrics {
  file_path: string;
  file_name: string;
  star_count: number;
  median_fwhm: number;
  median_eccentricity: number;
  median_snr: number;
  background_median: number;
  background_sigma: number;
  noise_ratio: number;
  noise_sigma: number;
  noise_fraction: number;
  weight: number;
  accepted: boolean;
}

export interface SubframeAnalysisResult {
  subframes: SubframeMetrics[];
  total: number;
  accepted: number;
  rejected: number;
  elapsed_ms: number;
}

export interface ChannelStage {
  path: string;
  note: string;
}

export interface ChannelResult {
  starless: ChannelStage | null;
  stretched: ChannelStage | null;
}

export interface LevelMatchEntry {
  channel: string;
  scale: number;
  z: number;
}

export interface LevelScale {
  k: number;
  offset: number;
  z: number;
}

export interface CompositeHistory {
  blend: string | null;
  levels: string[];
  colorBalance: string[];
  after: string[];
}

export const EMPTY_COMPOSITE_HISTORY: CompositeHistory = { blend: null, levels: [], colorBalance: [], after: [] };

export type CompositeOp =
  | { kind: "blend"; preset: string; levels?: readonly LevelMatchEntry[] | null }
  | { kind: "colorBalance"; mode: string; r: number; g: number; b: number; scnr: { method: string; amount: number } | null }
  | { kind: "resetColorBalance" }
  | { kind: "lrgb"; lightness: number; chrominance: number }
  | { kind: "starRemoval"; sigma: number; growth: number };

export interface WizardState {
  bins: FrequencyBin[];
  stackedPaths: Record<string, string>;
  alignedPaths: Record<string, string>;
  croppedPaths: Record<string, string>;
  backgroundPaths: Record<string, string>;
  channelResults: Record<string, ChannelResult>;
  compositeHistory: CompositeHistory;
  blendWeights: BlendWeight[];
  blendPreset: string;
  compositeReady: boolean;
  wbMode: "auto" | "spcc" | "manual" | "none";
  wbR: number;
  wbG: number;
  wbB: number;
  starMaskPath: string | null;
  segmPath: string | null;
  maskGrowth: number;
  maskProtection: number;
  stretchMode: "masked" | "arcsinh" | "ghs" | "auto_stf";
  stretchFactor: number;
  targetBackground: number;
  scnrEnabled: boolean;
  scnrAmount: number;
  scnrMethod: "average" | "maximum";
  scnrPreserveLuminance: boolean;
  resultPng: string | null;
  resultFits: string | null;
  completedSteps: Record<string, boolean>;
  subframeResults: Record<string, SubframeAnalysisResult>;
  excludedFiles: Record<string, string[]>;
  levelMatch: boolean | null;
  blendLevelScales: Record<string, LevelScale> | null;
  spccFactors: { r: number; g: number; b: number } | null;
  alignRefChoice: string | null;
  alignRefBinId: string | null;
  alignRunToken: string | null;
}

export const DEFAULT_BINS: FrequencyBin[] = [
  { id: "ha", label: "Hα (656nm)", shortLabel: "Hα", wavelength: 656, color: "#ef4444", files: [] },
  { id: "oiii", label: "OIII (501nm)", shortLabel: "OIII", wavelength: 501, color: "#3b82f6", files: [] },
  { id: "sii", label: "SII (673nm)", shortLabel: "SII", wavelength: 673, color: "#f97316", files: [] },
  { id: "r", label: "Red", shortLabel: "R", color: "#dc2626", files: [] },
  { id: "g", label: "Green", shortLabel: "G", color: "#16a34a", files: [] },
  { id: "b", label: "Blue", shortLabel: "B", color: "#2563eb", files: [] },
  { id: "l", label: "Luminance", shortLabel: "L", color: "#a1a1aa", files: [] },
];

export const BLEND_PRESETS: Record<string, { label: string; desc: string; weights: BlendWeight[] }> = {
  rgb: {
    label: "RGB",
    desc: "Direct R→R G→G B→B",
    weights: [
      { channelId: "r", r: 1.0, g: 0.0, b: 0.0 },
      { channelId: "g", r: 0.0, g: 1.0, b: 0.0 },
      { channelId: "b", r: 0.0, g: 0.0, b: 1.0 },
    ],
  },
  sho: {
    label: "SHO (Hubble)",
    desc: "SII→R Hα→G OIII→B",
    weights: [
      { channelId: "sii", r: 1.0, g: 0.0, b: 0.0 },
      { channelId: "ha", r: 0.0, g: 1.0, b: 0.0 },
      { channelId: "oiii", r: 0.0, g: 0.0, b: 1.0 },
    ],
  },
  hubble_legacy: {
    label: "Hubble Legacy",
    desc: "Blended SHO with teal/yellow tones",
    weights: [
      { channelId: "sii", r: 0.7, g: 0.3, b: 0.0 },
      { channelId: "ha", r: 0.3, g: 0.8, b: 0.2 },
      { channelId: "oiii", r: 0.0, g: 0.15, b: 0.85 },
    ],
  },
  hoo: {
    label: "HOO",
    desc: "Hα→R OIII→G+B",
    weights: [
      { channelId: "ha", r: 1.0, g: 0.0, b: 0.0 },
      { channelId: "oiii", r: 0.0, g: 0.5, b: 0.5 },
    ],
  },
  dynamic_hoo: {
    label: "Dynamic HOO",
    desc: "Blended Hα/OIII with warm tones",
    weights: [
      { channelId: "ha", r: 0.9, g: 0.4, b: 0.0 },
      { channelId: "oiii", r: 0.1, g: 0.6, b: 1.0 },
    ],
  },
  foraxx: {
    label: "Foraxx",
    desc: "Popular narrowband blend",
    weights: [
      { channelId: "sii", r: 0.8, g: 0.2, b: 0.0 },
      { channelId: "ha", r: 0.2, g: 0.7, b: 0.1 },
      { channelId: "oiii", r: 0.0, g: 0.1, b: 0.9 },
    ],
  },
};

export const INITIAL_STATE: WizardState = {
  bins: DEFAULT_BINS.map((b) => ({ ...b, files: [] })),
  stackedPaths: {},
  alignedPaths: {},
  croppedPaths: {},
  backgroundPaths: {},
  channelResults: {},
  compositeHistory: EMPTY_COMPOSITE_HISTORY,
  blendWeights: BLEND_PRESETS.sho.weights,
  blendPreset: "sho",
  compositeReady: false,
  wbMode: "auto",
  wbR: 1.0,
  wbG: 1.0,
  wbB: 1.0,
  starMaskPath: null,
  segmPath: null,
  maskGrowth: 2.5,
  maskProtection: 0.85,
  stretchMode: "masked",
  stretchFactor: 50,
  targetBackground: 0.25,
  scnrEnabled: false,
  scnrAmount: 0.5,
  scnrMethod: "average",
  scnrPreserveLuminance: false,
  resultPng: null,
  resultFits: null,
  completedSteps: {},
  subframeResults: {},
  excludedFiles: {},
  levelMatch: null,
  blendLevelScales: null,
  spccFactors: null,
  alignRefChoice: null,
  alignRefBinId: null,
  alignRunToken: null,
};

export interface StepDef {
  id: string;
  label: string;
  shortLabel: string;
  color: string;
  enabled: (state: WizardState) => boolean;
  blockedReason: (state: WizardState) => string | null;
  badge?: (state: WizardState) => string | null;
}

function filledCount(s: WizardState): number {
  return s.bins.filter((b) => b.files.length > 0).length;
}

export function singleFilledBin(state: WizardState): string | null {
  const filled = state.bins.filter((b) => b.files.length > 0);
  return filled.length === 1 ? filled[0].id : null;
}

function totalFilesCount(s: WizardState): number {
  return s.bins.reduce((acc, b) => acc + b.files.length, 0);
}

const NEEDS_FRAMES = "assign frames in step 1";
const NEEDS_TWO_CHANNELS = "assign at least 2 channels";

function joinNames(names: readonly string[]): string {
  if (names.length <= 1) return names.join("");
  return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}

export function effectiveBinFiles(state: WizardState, bin: FrequencyBin): string[] {
  const excluded = state.excludedFiles[bin.id];
  if (!excluded || excluded.length === 0) return bin.files;
  const skip = new Set(excluded);
  return bin.files.filter((f) => !skip.has(f));
}

export function unstackedBins(state: WizardState): FrequencyBin[] {
  return state.bins.filter((b) => effectiveBinFiles(state, b).length >= 2 && !state.stackedPaths[b.id]);
}

export function unstackedReason(state: WizardState): string | null {
  const bins = unstackedBins(state);
  if (bins.length === 0) return null;
  const frames = bins.length === 1 ? "2+ frames" : "2+ frames each";
  return `stack ${joinNames(bins.map((b) => b.shortLabel))} first (${frames}), or keep one file per channel`;
}

export function fullyExcludedBins(state: WizardState): FrequencyBin[] {
  return state.bins.filter((b) => b.files.length > 0 && resolveChannelPath(state, b.id, "stacked") === null);
}

export function fullyExcludedReason(state: WizardState): string | null {
  const bins = fullyExcludedBins(state);
  if (bins.length === 0) return null;
  const each = bins.length === 1 ? "one" : "one per channel";
  return `all ${joinNames(bins.map((b) => b.shortLabel))} frames are excluded; re-include ${each} in Stack`;
}

function channelInputsReason(state: WizardState): string | null {
  return fullyExcludedReason(state) ?? unstackedReason(state);
}

function gate(blockedReason: (s: WizardState) => string | null): Pick<StepDef, "enabled" | "blockedReason"> {
  return { enabled: (s) => blockedReason(s) === null, blockedReason };
}

export function wizardHasProgress(state: WizardState): boolean {
  return totalFilesCount(state) > 0 || Object.values(state.completedSteps).some(Boolean);
}

export const NARROWBAND_IDS = new Set(["ha", "sii", "nii", "oiii", "hb"]);

const NB_PRESETS = new Set(["sho", "hoo", "dynamic_hoo", "foraxx", "hubble_legacy"]);

const NB_FILTERS = new Set(["Hα (656nm)", "[OIII] (501nm)", "[SII] (673nm)"]);

export interface FilterDetectionRef {
  path: string;
  filter: string | null;
}

export function isNarrowbandWorkflow(
  bins: FrequencyBin[],
  blendPreset?: string,
  filterDetections?: FilterDetectionRef[],
): boolean {
  const filled = bins.filter((b) => b.files.length > 0);
  if (filled.some((b) => NARROWBAND_IDS.has(b.id))) return true;
  if (blendPreset && NB_PRESETS.has(blendPreset)) return true;

  if (filterDetections && filterDetections.length > 0) {
    const assignedFiles = new Set(filled.flatMap((b) => b.files));
    for (const det of filterDetections) {
      if (det.filter && NB_FILTERS.has(det.filter) && assignedFiles.has(det.path)) {
        return true;
      }
    }
  }

  return false;
}

export const STEPS: StepDef[] = [
  {
    id: "channels",
    label: "Channel Assignment",
    shortLabel: "Channels",
    color: "violet",
    ...gate(() => null),
    badge: (s) => {
      const n = totalFilesCount(s);
      return n > 0 ? `${n}` : null;
    },
  },
  {
    id: "stack",
    label: "Stacking",
    shortLabel: "Stack",
    color: "blue",
    ...gate((s) => {
      if (totalFilesCount(s) === 0) return NEEDS_FRAMES;
      return s.bins.some((b) => b.files.length > 1) ? null : "needs a channel with 2+ frames";
    }),
    badge: (s) => {
      const n = Object.keys(s.stackedPaths).length;
      return n > 0 ? `${n}` : null;
    },
  },
  {
    id: "align",
    label: "Channel Alignment",
    shortLabel: "Align",
    color: "sky",
    ...gate((s) => (filledCount(s) >= 2 ? channelInputsReason(s) : NEEDS_TWO_CHANNELS)),
  },
  {
    id: "crop",
    label: "Crop",
    shortLabel: "Crop",
    color: "cyan",
    ...gate((s) => {
      if (Object.keys(s.alignedPaths).length > 0 || singleFilledBin(s) !== null) return channelInputsReason(s);
      return filledCount(s) >= 2 ? "run Align first" : NEEDS_FRAMES;
    }),
    badge: (s) => {
      const n = Object.keys(s.croppedPaths).length;
      return n > 0 ? `${n}` : null;
    },
  },
  {
    id: "background",
    label: "Background Extraction",
    shortLabel: "BG",
    color: "emerald",
    ...gate((s) =>
      Object.keys(s.alignedPaths).length > 0 ||
      Object.keys(s.croppedPaths).length > 0 ||
      totalFilesCount(s) > 0
        ? channelInputsReason(s)
        : NEEDS_FRAMES),
    badge: (s) => {
      const n = Object.keys(s.backgroundPaths).length;
      return n > 0 ? `${n}` : null;
    },
  },
  {
    id: "blend",
    label: "Channel Blending",
    shortLabel: "Blend",
    color: "amber",
    ...gate((s) => (filledCount(s) >= 2 ? channelInputsReason(s) : NEEDS_TWO_CHANNELS)),
    badge: (s) => s.compositeReady ? "✓" : null,
  },
  {
    id: "colorbalance",
    label: "Color Balance",
    shortLabel: "Color",
    color: "cyan",
    ...gate((s) => {
      if (s.compositeReady) return null;
      return filledCount(s) >= 2 ? channelInputsReason(s) : NEEDS_TWO_CHANNELS;
    }),
  },
  {
    id: "stretch",
    label: "Stretch",
    shortLabel: "Stretch",
    color: "amber",
    ...gate((s) => {
      if (s.compositeReady) return null;
      return totalFilesCount(s) > 0 ? channelInputsReason(s) : NEEDS_FRAMES;
    }),
  },
  {
    id: "adjust",
    label: "Adjust",
    shortLabel: "Adjust",
    color: "purple",
    ...gate((s) => {
      if (s.compositeReady) return null;
      return filledCount(s) >= 2 ? "run Blend first" : `${NEEDS_TWO_CHANNELS}, then run Blend`;
    }),
  },
  {
    id: "export",
    label: "Export",
    shortLabel: "Export",
    color: "teal",
    ...gate((s) => {
      if (s.compositeReady) return null;
      return totalFilesCount(s) > 0 ? channelInputsReason(s) : NEEDS_FRAMES;
    }),
  },
];

export const STEP_ORDER = STEPS.map((s) => s.id);

export function invalidateFromStep(
  completed: Record<string, boolean>,
  fromStepId: string,
): Record<string, boolean> {
  const idx = STEP_ORDER.indexOf(fromStepId);
  if (idx === -1) return completed;
  const next = { ...completed };
  for (let i = idx; i < STEP_ORDER.length; i++) {
    delete next[STEP_ORDER[i]];
  }
  return next;
}

export function invalidateDownstream(
  state: WizardState,
  fromStepId: string,
): Partial<WizardState> {
  const idx = STEP_ORDER.indexOf(fromStepId);
  if (idx === -1) return {};
  const partial: Partial<WizardState> = {
    completedSteps: invalidateFromStep(state.completedSteps, fromStepId),
  };

  const clear = (stepId: string) => STEP_ORDER.indexOf(stepId) > idx;

  if (clear("align")) {
    partial.alignedPaths = {};
    partial.alignRefBinId = null;
    partial.alignRunToken = null;
  }
  if (clear("crop")) partial.croppedPaths = {};
  if (clear("background")) partial.backgroundPaths = {};
  if (clear("blend")) {
    partial.compositeReady = false;
    partial.compositeHistory = EMPTY_COMPOSITE_HISTORY;
    partial.blendLevelScales = null;
  }
  if (clear("stretch")) partial.channelResults = {};

  return partial;
}

const STEP_RESULTS: Partial<Record<string, (s: WizardState) => boolean>> = {
  align: (s) => Object.keys(s.alignedPaths).length > 0,
  crop: (s) => Object.keys(s.croppedPaths).length > 0,
  background: (s) => Object.keys(s.backgroundPaths).length > 0,
  blend: (s) => s.compositeReady,
  stretch: (s) => Object.keys(s.channelResults).length > 0,
};

export function rerunDiscards(state: WizardState, fromStepId: string): string[] {
  const from = STEP_ORDER.indexOf(fromStepId);
  if (from === -1) return [];
  const after: WizardState = { ...state, ...invalidateDownstream(state, fromStepId) };
  return STEPS.filter((step, i) => {
    if (i <= from) return false;
    const has = STEP_RESULTS[step.id];
    const lostResult = has ? has(state) && !has(after) : false;
    const lostDone = !!state.completedSteps[step.id] && !after.completedSteps[step.id];
    return lostResult || lostDone;
  }).map((step) => step.shortLabel);
}

export function discardsNotice(action: string, labels: readonly string[]): string | null {
  return labels.length > 0 ? `${action} discards: ${labels.join(", ")}` : null;
}

export function unalignedBins(state: WizardState): FrequencyBin[] {
  const filled = state.bins.filter((b) => b.files.length > 0);
  if (filled.length < 2) return [];
  return filled.filter((b) => !state.alignedPaths[b.id]);
}

export const BACKGROUND_UNALIGNED_NOTICE =
  "The channels are not aligned. Running Align later discards these BG results; align first unless the channels are already registered.";

export function backgroundAlignNotice(state: WizardState): string | null {
  if (filledCount(state) < 2 || Object.keys(state.alignedPaths).length > 0) return null;
  return BACKGROUND_UNALIGNED_NOTICE;
}

export function nextEnabledStep(
  currentId: string,
  state: WizardState,
): string | null {
  const idx = STEP_ORDER.indexOf(currentId);
  for (let i = idx + 1; i < STEP_ORDER.length; i++) {
    const step = STEPS.find((s) => s.id === STEP_ORDER[i]);
    if (step && step.enabled(state)) return step.id;
  }
  return null;
}

export type PipelineStage = "background" | "cropped" | "aligned" | "stacked";

const STAGE_ORDER: PipelineStage[] = ["background", "cropped", "aligned", "stacked"];

function stagePath(state: WizardState, binId: string, stage: PipelineStage): string | undefined {
  switch (stage) {
    case "background": return state.backgroundPaths[binId];
    case "cropped": return state.croppedPaths[binId];
    case "aligned": return state.alignedPaths[binId];
    case "stacked": return state.stackedPaths[binId];
  }
}

export function resolveChannelPath(
  state: WizardState,
  binId: string,
  upTo: PipelineStage = "background",
): string | null {
  for (let i = STAGE_ORDER.indexOf(upTo); i < STAGE_ORDER.length; i++) {
    const p = stagePath(state, binId, STAGE_ORDER[i]);
    if (p) return p;
  }
  const bin = state.bins.find((b) => b.id === binId);
  return bin ? effectiveBinFiles(state, bin)[0] ?? null : null;
}

export function withExcludedFiles(state: WizardState, binId: string, files: string[]): WizardState {
  const next: WizardState = { ...state, excludedFiles: { ...state.excludedFiles, [binId]: files } };
  const needsStack = (s: WizardState) => unstackedBins(s).some((b) => b.id === binId);
  const sameInput = resolveChannelPath(next, binId, "stacked") === resolveChannelPath(state, binId, "stacked");
  if (sameInput && needsStack(next) === needsStack(state)) return next;
  return { ...next, ...invalidateDownstream(state, "align"), alignedPaths: {}, alignRefBinId: null, alignRunToken: null };
}

export function resolveAnyChannelPath(
  state: WizardState,
  upTo: PipelineStage = "background",
): string | null {
  for (const bin of state.bins) {
    const p = resolveChannelPath(state, bin.id, upTo);
    if (p) return p;
  }
  return null;
}

interface RgbSlot {
  binId: string;
  path: string | null;
}

const RGB_CANDIDATES: Record<RgbKey, readonly string[]> = {
  r: ["r", "sii", "ha"],
  g: ["g", "ha", "oiii"],
  b: ["b", "oiii", "sii"],
};

function rgbSlots(state: WizardState, pathOf: (binId: string) => string | null): Record<RgbKey, RgbSlot | null> {
  const activeBins = state.bins.filter((b) => b.files.length > 0);
  const usedIds = new Set<string>();
  const slotOf = (binId: string): RgbSlot => ({ binId, path: pathOf(binId) });
  const resolved = (slot: RgbSlot | null): RgbSlot | null => (slot && slot.path !== null ? slot : null);
  const anyPath = (): RgbSlot | null => {
    for (const bin of state.bins) {
      const slot = slotOf(bin.id);
      if (slot.path) return slot;
    }
    return null;
  };

  const findBest = (candidates: readonly string[], allowReuse = false): RgbSlot | null => {
    for (const cid of candidates) {
      if (!allowReuse && usedIds.has(cid)) continue;
      if (activeBins.some((b) => b.id === cid)) {
        usedIds.add(cid);
        return slotOf(cid);
      }
    }
    return null;
  };

  const r = findBest(RGB_CANDIDATES.r);
  const g = findBest(RGB_CANDIDATES.g);
  let b = findBest(RGB_CANDIDATES.b);
  if (!b?.path) b = findBest(RGB_CANDIDATES.b, true);

  const fillFromUnused = (): RgbSlot | null => {
    const bin = activeBins.find((bn) => !usedIds.has(bn.id));
    if (!bin) return null;
    usedIds.add(bin.id);
    return slotOf(bin.id);
  };

  return {
    r: resolved(r) ?? resolved(fillFromUnused()) ?? anyPath(),
    g: resolved(g) ?? resolved(fillFromUnused()) ?? anyPath(),
    b: resolved(b) ?? resolved(fillFromUnused()) ?? anyPath(),
  };
}

export function resolveRgbPaths(
  state: WizardState,
  useChannelOutputs = false,
): { r: string | null; g: string | null; b: string | null } {
  const slots = rgbSlots(state, (binId) =>
    useChannelOutputs ? resolveOutputChannelPath(state, binId) : resolveChannelPath(state, binId));
  return { r: slots.r?.path ?? null, g: slots.g?.path ?? null, b: slots.b?.path ?? null };
}

export function resolveRgbBins(state: WizardState): { r: string | null; g: string | null; b: string | null } {
  const slots = rgbSlots(state, (binId) => resolveChannelPath(state, binId));
  return { r: slots.r?.binId ?? null, g: slots.g?.binId ?? null, b: slots.b?.binId ?? null };
}

function fixedExportBins(state: WizardState): Record<RgbKey, string> | null {
  if (state.compositeReady || filledCount(state) < 2 || resolveExportRgbPaths(state).monoBinId !== null) return null;
  const { r, g, b } = resolveRgbBins(state);
  return r && g && b ? { r, g, b } : null;
}

export function exportMappingBanner(state: WizardState): string | null {
  const bins = fixedExportBins(state);
  if (!bins) return null;
  const label = (binId: string) => binShortLabel(state, binId);
  return `No composite yet: exporting the fixed mapping R=${label(bins.r)} G=${label(bins.g)} B=${label(bins.b)}. Your Blend weights are not applied; run Blend to export the composite.`;
}

export function singleChannelBinId(state: WizardState): string | null {
  return state.bins.find((b) => resolveChannelPath(state, b.id) !== null)?.id ?? null;
}

export function channelStretchInput(state: WizardState, binId: string): string | null {
  return state.channelResults[binId]?.starless?.path ?? resolveChannelPath(state, binId);
}

export function resolveOutputChannelPath(state: WizardState, binId: string): string | null {
  const result = state.channelResults[binId];
  return result?.stretched?.path ?? result?.starless?.path ?? resolveChannelPath(state, binId);
}

export function resolveExportRgbPaths(
  state: WizardState,
): { r: string | null; g: string | null; b: string | null; monoBinId: string | null } {
  const paths = resolveRgbPaths(state, true);
  const binId = singleChannelBinId(state);
  const result = binId ? state.channelResults[binId] : undefined;
  const out = binId ? resolveOutputChannelPath(state, binId) : null;
  if (!binId || !result || !out) return { ...paths, monoBinId: null };
  const slots = [paths.r, paths.g, paths.b];
  if (slots.every((p) => p === out)) return { ...paths, monoBinId: null };
  if (!result.stretched && slots.includes(out)) return { ...paths, monoBinId: null };
  return { r: out, g: out, b: out, monoBinId: binId };
}

export function wizardHeaderSourcePath(
  state: WizardState,
  exported: readonly (string | null)[],
  monoBinId: string | null = null,
): string | null {
  if (Object.keys(state.croppedPaths).length > 0) return null;
  const stretched = new Set(Object.values(state.channelResults).map((c) => c.stretched?.path));
  if (exported.some((p) => p !== null && stretched.has(p))) return null;
  const active = state.bins.filter((b) => b.files.length > 0);
  const aligned = Object.keys(state.alignedPaths).length > 0;
  const bin = aligned
    ? active.find((b) => b.id === state.alignRefBinId) ?? active[0]
    : monoBinId
      ? active.find((b) => b.id === monoBinId)
      : active.length === 1
        ? active[0]
        : undefined;
  if (!bin) return null;
  return state.stackedPaths[bin.id] ?? effectiveBinFiles(state, bin)[0] ?? null;
}

export function wizardChannelExport(state: WizardState): { r: string; g: string; b: string; monoBinId: string | null } | null {
  const { r, g, b, monoBinId } = resolveExportRgbPaths(state);
  return r && g && b ? { r, g, b, monoBinId } : null;
}

export function wizardZipChannels(state: WizardState): { name: string; path: string }[] {
  const channels = state.compositeReady ? { ...resolveRgbPaths(state), monoBinId: null } : resolveExportRgbPaths(state);
  const entries = channels.monoBinId
    ? [{ name: `channel_${channels.monoBinId}`, path: channels.r }]
    : (["r", "g", "b"] as const).map((key) => ({ name: `channel_${key}`, path: channels[key] }));
  return entries.filter((e): e is { name: string; path: string } => e.path !== null);
}

export function exportBlockedReason(state: WizardState): string | null {
  if (state.compositeReady) return null;
  const { r, g, b } = resolveExportRgbPaths(state);
  return r || g || b ? null : "Assign at least one channel in step 1";
}

export const WCS_MISSING_WARNING =
  "This FITS was written without a celestial WCS (no CTYPE, CRVAL or CD cards), so viewers cannot place it on the sky.";

export function exportWcsWarning(result: object, headerWarning: string | null): string | null {
  if (headerWarning) return headerWarning;
  return "wcs_written" in result && result.wcs_written === false ? WCS_MISSING_WARNING : null;
}

export function autoStfBlockedReason(state: WizardState): string | null {
  if (state.compositeReady) return null;
  return filledCount(state) >= 2
    ? "Auto STF needs a blended composite: run Blend first."
    : "Auto STF needs a blended composite, and Blend needs at least 2 channels.";
}

type RgbKey = "r" | "g" | "b";

const RGB_KEYS: readonly RgbKey[] = ["r", "g", "b"];

const SPCC_SOURCES: Record<RgbKey, readonly string[]> = {
  r: ["r", "ha"],
  g: ["g", "oiii"],
  b: ["b", "sii"],
};

export interface SpccInput {
  binId: string;
  path: string;
}

export function spccInputs(state: WizardState): Record<RgbKey, SpccInput | null> {
  const pick = (ids: readonly string[]): SpccInput | null => {
    for (const binId of ids) {
      const path = resolveChannelPath(state, binId);
      if (path) return { binId, path };
    }
    return null;
  };
  return { r: pick(SPCC_SOURCES.r), g: pick(SPCC_SOURCES.g), b: pick(SPCC_SOURCES.b) };
}

const SPCC_BROADBAND_ONLY = "SPCC models broadband R/G/B filters only";

const NARROWBAND_FILTER_CODE = /^F\d{3,4}N$/;

const MEDIUM_BAND_FILTER_CODE = /^F\d{3,4}M$/;

const NARROWBAND_LINE_CODES = new Set(["HA", "HALPHA", "H_ALPHA", "OIII", "O3", "SII", "S2", "NII", "HB", "HBETA"]);

const NARROWBAND_BIN_LABELS: Record<string, string> = { ha: "Hα", oiii: "OIII", sii: "SII", nii: "NII", hb: "Hβ" };

export function narrowbandFilterLabel(
  file: ChannelSource | undefined,
  detection: FilterDetectionRef | undefined,
): string | null {
  if (detection?.filter && NB_FILTERS.has(detection.filter)) return detection.filter;
  if (!file) return null;
  const code = resolveFileFilter(file)?.code;
  if (code && (NARROWBAND_FILTER_CODE.test(code) || NARROWBAND_LINE_CODES.has(code))) return code;
  const bin = detectChannel(file);
  return bin ? NARROWBAND_BIN_LABELS[bin] ?? null : null;
}

export const SPCC_MIN_NM = 380;

export const SPCC_MAX_NM = 830;

export const AUTO_WB_SPCC_HINT = "Use SPCC or manual factors.";

export const AUTO_WB_MANUAL_HINT = "Use manual factors; SPCC does not model these filters.";

export function fileFilterBand(file: ChannelSource | undefined, path: string): { code: string; nm: number } | null {
  return (file ? resolveFileFilter(file) : null) ?? resolveFilterFromName(file?.name || shortName(path));
}

export function filterCodeTokens(file: ChannelSource | undefined, path: string): string[] {
  const sources = [...(file ? headerFilterValues(file) : []), shortName(file?.name || path)];
  return sources.flatMap((value) => value.toUpperCase().split(/[^A-Z0-9]+/)).filter((token) => token !== "");
}

export function narrowbandFileLabel(
  file: ChannelSource | undefined,
  detection: FilterDetectionRef | undefined,
  path: string,
): string | null {
  return narrowbandFilterLabel(file, detection) ?? filterCodeTokens(file, path).find((t) => NARROWBAND_FILTER_CODE.test(t)) ?? null;
}

export function scnrAutoEnable(narrowband: boolean, spccBlocked: string | null): boolean {
  return !narrowband && spccBlocked === null;
}

export function autoWbErrorText(message: string, spccAllowed: boolean): string {
  return spccAllowed ? message : message.replace(AUTO_WB_SPCC_HINT, AUTO_WB_MANUAL_HINT);
}

function binShortLabel(state: WizardState, binId: string): string {
  return state.bins.find((b) => b.id === binId)?.shortLabel ?? binId;
}

export function spccBlockReason(
  state: WizardState,
  files: readonly (ChannelSource & { path: string })[],
  detections: readonly FilterDetectionRef[] = [],
): string | null {
  const inputs = spccInputs(state);
  const fallbacks = RGB_KEYS.flatMap((key) => {
    const input = inputs[key];
    return input && input.binId !== key ? [{ key, binId: input.binId }] : [];
  });
  if (fallbacks.length > 0) {
    const names = fallbacks.map((f) => binShortLabel(state, f.binId)).join(", ");
    const letters = fallbacks.map((f) => f.key.toUpperCase()).join(", ");
    return `${SPCC_BROADBAND_ONLY}; ${names} would be measured as ${letters}.`;
  }
  const inputFiles = RGB_KEYS.flatMap((key) =>
    (state.bins.find((b) => b.id === key)?.files ?? []).map((path) => ({
      path,
      file: files.find((f) => f.path === path),
      where: `${shortName(path)} in ${binShortLabel(state, key)}`,
    })),
  );
  for (const { path, file, where } of inputFiles) {
    const label = narrowbandFileLabel(file, detections.find((d) => d.path === path), path);
    if (label) return `${SPCC_BROADBAND_ONLY}; ${where} is a narrowband frame (${label}).`;
  }
  for (const { path, file, where } of inputFiles) {
    const medium = filterCodeTokens(file, path).find((t) => MEDIUM_BAND_FILTER_CODE.test(t));
    if (medium) return `${SPCC_BROADBAND_ONLY}; ${where} is a medium-band frame (${medium}).`;
  }
  for (const { path, file, where } of inputFiles) {
    const band = fileFilterBand(file, path);
    if (band && (band.nm < SPCC_MIN_NM || band.nm > SPCC_MAX_NM)) {
      return `SPCC models visible light (${SPCC_MIN_NM}-${SPCC_MAX_NM} nm) only; ${where} is ${band.code} at ${band.nm} nm.`;
    }
  }
  return null;
}

const ALIGN_METHOD_LABELS: Record<string, string> = {
  phase_correlation: "phase correlation",
  affine: "affine",
  rigid: "rigid",
  wcs: "WCS",
};

const IDENTITY_ALIGN_METHODS = new Set(["identity", "phase_correlation_identity"]);

export interface AlignChannelOutcome {
  unregistered: string | null;
  usedMethod: string | null;
}

export function alignChannelOutcome(
  channel: { registered?: boolean; method_used?: string } | undefined,
  requestedMethod: string,
  isReference: boolean,
): AlignChannelOutcome {
  if (!channel || isReference) return { unregistered: null, usedMethod: null };
  const used = channel.method_used;
  const registered = channel.registered ?? !(used && IDENTITY_ALIGN_METHODS.has(used));
  if (!registered) {
    const other = requestedMethod === "affine" ? "Phase Correlation" : "Star-based Affine";
    return { unregistered: `not registered — low-confidence match, channel left unshifted; try ${other}`, usedMethod: null };
  }
  const usedMethod = used && used !== requestedMethod ? ALIGN_METHOD_LABELS[used] ?? used : null;
  return { unregistered: null, usedMethod };
}

function alignMethodLabel(method: string): string {
  if (IDENTITY_ALIGN_METHODS.has(method)) return "identity";
  return ALIGN_METHOD_LABELS[method] ?? method;
}

export function alignRunMethodLabel(result: {
  align_method: string;
  channels?: readonly { offset?: readonly [number, number]; registered?: boolean; method_used?: string }[];
  reference_index?: number;
}): string {
  const requested = alignMethodLabel(result.align_method);
  const referenceIndex = result.reference_index ?? 0;
  const used = (result.channels ?? [])
    .filter((_, i) => i !== referenceIndex)
    .map((ch) => ch.method_used)
    .filter((m): m is string => !!m);
  const fallbacks = used.filter((m) => m !== result.align_method);
  if (fallbacks.length === 0) return requested;
  const counts = new Map<string, number>();
  for (const m of fallbacks) {
    const label = alignMethodLabel(m);
    counts.set(label, (counts.get(label) ?? 0) + 1);
  }
  if (fallbacks.length === used.length && counts.size === 1 && !counts.has("identity")) return [...counts.keys()][0];
  const detail = [...counts].map(([label, n]) => `${label} on ${n} of ${used.length}`).join(", ");
  return `${requested} requested; ${detail}`;
}

export const ALIGN_OFFSET_TITLE = "Offset of this channel relative to the reference, in array pixels (y down)";

function signedPixels(value: number): string {
  if (!Number.isFinite(value)) return "n/a";
  const rounded = Math.round(value * 10) / 10;
  if (rounded === 0) return "0.0 px";
  return `${rounded > 0 ? "+" : "−"}${Math.abs(rounded).toFixed(1)} px`;
}

export function formatAlignOffset(offset: readonly [number, number]): string {
  const [dy, dx] = offset;
  return `Δx ${signedPixels(dx)}  Δy ${signedPixels(dy)}`;
}

export function alignMatchSummary(
  channel: { confidence?: number; matched_stars?: number; inliers?: number; residual_px?: number } | undefined,
): string | null {
  if (!channel) return null;
  const matched = channel.matched_stars ?? 0;
  if (matched > 0) {
    const stars = `${channel.inliers ?? 0}/${matched} stars`;
    return channel.residual_px != null && Number.isFinite(channel.residual_px)
      ? `${stars}, ${channel.residual_px.toFixed(2)} px`
      : stars;
  }
  const snr = channel.confidence ?? 0;
  return Number.isFinite(snr) && snr > 0 ? `SNR ${snr.toFixed(1)}` : null;
}

export const MAX_OVERLAY_CHANNELS = 3;

export function alignOverlayBinIds(
  binIds: readonly string[],
  chosen: readonly string[] = [],
  referenceBinId: string | null = null,
): string[] {
  if (binIds.length === 0) return [];
  const reference = referenceBinId !== null && binIds.includes(referenceBinId) ? referenceBinId : binIds[0];
  const others = binIds.filter((id) => id !== reference);
  if (binIds.length <= MAX_OVERLAY_CHANNELS) return [reference, ...others];
  const picked: string[] = [];
  const slots = MAX_OVERLAY_CHANNELS - 1;
  for (const id of [...chosen, ...others]) {
    if (picked.length === slots) break;
    if (others.includes(id) && !picked.includes(id)) picked.push(id);
  }
  return [reference, ...picked];
}

export interface AlignedRun {
  binIds: string[];
  aligned: Record<string, string>;
  inputs: Record<string, string>;
  referenceBinId?: string | null;
}

export function alignedRunFromChannels(
  channels: readonly { binId: string; path: string }[],
  alignedPaths: Readonly<Record<string, string>>,
  referenceBinId: string | null = null,
): AlignedRun | null {
  if (channels.length < 2) return null;
  const aligned: Record<string, string> = {};
  const inputs: Record<string, string> = {};
  for (const { binId, path } of channels) {
    const key = alignedPaths[binId];
    if (!key) return null;
    aligned[binId] = key;
    inputs[binId] = path;
  }
  const run: AlignedRun = { binIds: channels.map((c) => c.binId), aligned, inputs };
  return referenceBinId === null ? run : { ...run, referenceBinId };
}

export interface AlignOverlayRequest {
  binIds: string[];
  afterKeys: string[];
  beforePaths: string[];
}

export function alignOverlayRequest(run: AlignedRun, chosen: readonly string[] = []): AlignOverlayRequest | null {
  const binIds = alignOverlayBinIds(run.binIds, chosen, run.referenceBinId ?? null);
  const afterKeys = binIds.map((id) => run.aligned[id]);
  const beforePaths = binIds.map((id) => run.inputs[id]);
  if (binIds.length < 2 || afterKeys.some((k) => !k) || beforePaths.some((p) => !p)) return null;
  return { binIds, afterKeys, beforePaths };
}

export function alignOverlayChoices(run: AlignedRun, request: AlignOverlayRequest): string[] | null {
  return run.binIds.length > MAX_OVERLAY_CHANNELS ? run.binIds.filter((id) => id !== request.binIds[0]) : null;
}

export type AlignPreviewView = "after" | "before" | "blink";

const NO_DATA = "checkerboard = no data";

export function alignOverlayColours(labels: readonly string[]): string {
  const [reference, ...others] = labels;
  const ref = `${reference} (ref)`;
  if (others.length === 0) return `grey = ${ref}`;
  if (others.length === 1) return `R = ${ref} · G+B = ${others[0]}`;
  return `R = ${ref} · G = ${others[0]} · B = ${others[1]}`;
}

export function alignPreviewLegend(labels: readonly string[], view: AlignPreviewView, blinkIndex = 1): string {
  if (labels.length < 2) return `${alignOverlayColours(labels)} · ${NO_DATA}`;
  if (view === "blink") {
    const other = labels[Math.min(Math.max(blinkIndex, 1), labels.length - 1)];
    return `blinking ${labels[0]} (ref) and ${other} after Align · stars that jump = residual offset · ${NO_DATA}`;
  }
  const meaning = view === "before"
    ? "inputs before Align: coloured fringes = original offset"
    : "white = aligned, coloured fringes = residual offset";
  return `${alignOverlayColours(labels)} · ${meaning}, ${NO_DATA}`;
}

export function sameAlignedRun(a: AlignedRun | null, b: AlignedRun | null): boolean {
  if (a === b) return true;
  if (!a || !b || a.binIds.length !== b.binIds.length) return false;
  if ((a.referenceBinId ?? null) !== (b.referenceBinId ?? null)) return false;
  return a.binIds.every(
    (id, i) => id === b.binIds[i] && a.aligned[id] === b.aligned[id] && a.inputs[id] === b.inputs[id],
  );
}

export interface AlignedRunState {
  source: Readonly<Record<string, string>>;
  loading: boolean;
  run: AlignedRun | null;
}

export function nextAlignedRunState(
  previous: AlignedRunState | null,
  channels: readonly { binId: string; path: string }[],
  alignedPaths: Readonly<Record<string, string>>,
  loading: boolean,
  referenceBinId: string | null = null,
): AlignedRunState {
  const run = loading ? null : alignedRunFromChannels(channels, alignedPaths, referenceBinId);
  if (
    previous &&
    previous.source === alignedPaths &&
    previous.loading === loading &&
    sameAlignedRun(previous.run, run)
  ) {
    return previous;
  }
  return { source: alignedPaths, loading, run };
}

export interface AlignOverlayShown {
  previewUrl: string;
  frameUrls: readonly string[];
  binIds: readonly string[];
}

export interface AlignBlinkFrame {
  src: string;
  binId: string;
  index: number;
}

export function alignBlinkPairs(frameUrls: readonly string[], binIds: readonly string[]): number {
  return Math.min(frameUrls.length, binIds.length);
}

export function alignBlinkIndex(index: number, pairs: number): number {
  return Math.min(Math.max(index, 1), Math.max(pairs - 1, 1));
}

export function alignBlinkFrame(
  frameUrls: readonly string[],
  binIds: readonly string[],
  index: number,
  showOther: boolean,
): AlignBlinkFrame | null {
  const pairs = alignBlinkPairs(frameUrls, binIds);
  if (pairs < 2) return null;
  const shownIndex = showOther ? alignBlinkIndex(index, pairs) : 0;
  return { src: frameUrls[shownIndex], binId: binIds[shownIndex], index: shownIndex };
}

export interface AlignPreviewFrame {
  view: AlignPreviewView;
  src: string;
  label: string;
  labels: readonly string[];
  blinkIndex: number;
  canBlink: boolean;
}

export function alignPreviewFrame(
  view: "after" | "before",
  blink: { on: boolean; showOther: boolean; index: number },
  after: AlignOverlayShown | null,
  before: AlignOverlayShown | null,
  binLabels: Readonly<Record<string, string>>,
): AlignPreviewFrame {
  const labelOf = (binId: string) => binLabels[binId] ?? binId;
  const frames = after?.frameUrls ?? [];
  const frameBins = after?.binIds ?? [];
  const pairs = alignBlinkPairs(frames, frameBins);
  const canBlink = pairs >= 2;
  const blinkIndex = alignBlinkIndex(blink.index, pairs);
  const blinkFrame = blink.on ? alignBlinkFrame(frames, frameBins, blink.index, blink.showOther) : null;
  if (blinkFrame) {
    const name = labelOf(blinkFrame.binId);
    return {
      view: "blink",
      src: blinkFrame.src,
      label: blinkFrame.index === 0 ? `${name} (ref)` : name,
      labels: frameBins.slice(0, pairs).map(labelOf),
      blinkIndex,
      canBlink,
    };
  }
  const shown = view === "before" ? before : after;
  return {
    view,
    src: shown?.previewUrl ?? "",
    label: view === "before" ? "Before Align" : "After Align",
    labels: (shown?.binIds ?? []).map(labelOf),
    blinkIndex,
    canBlink,
  };
}

export const ALIGN_IMAGE_LOAD_ERROR = "the overlay image could not be loaded; run Align again";

export interface AlignViewerStatus {
  hasRequest: boolean;
  loading: boolean;
  error: string;
  brokenSrc: string;
}

export interface AlignViewerState {
  src: string;
  pending: boolean;
  shown: AlignPreviewFrame | null;
  error: string;
}

export function alignViewerState(
  target: AlignPreviewFrame,
  displayed: AlignPreviewFrame | null,
  status: AlignViewerStatus,
): AlignViewerState {
  if (!status.hasRequest) return { src: "", pending: false, shown: null, error: "" };
  const broken = target.src !== "" && target.src === status.brokenSrc;
  if (target.src && !broken) {
    const shown = displayed && displayed.src !== target.src ? displayed : target;
    return { src: target.src, pending: status.loading, shown, error: "" };
  }
  const error = broken ? ALIGN_IMAGE_LOAD_ERROR : status.error;
  if (error || !displayed) return { src: "", pending: false, shown: null, error };
  return { src: displayed.src, pending: true, shown: displayed, error: "" };
}

export function alignDisplayedOnLoad(
  loadedSrc: string,
  target: AlignPreviewFrame,
  displayed: AlignPreviewFrame | null,
): AlignPreviewFrame | null {
  return loadedSrc !== "" && loadedSrc === target.src ? target : displayed;
}

export interface AlignRunRecord<Result> {
  running: boolean;
  inputs: readonly string[];
  result: Result | null;
  error: string;
}

export function alignRunOutcome<Result>(
  record: AlignRunRecord<Result> | null,
  inputs: readonly string[],
  keysStored: boolean,
): { result: Result | null; error: string } {
  if (!record || record.running) return { result: null, error: "" };
  const sameInputs = record.inputs.length === inputs.length && record.inputs.every((p, i) => p === inputs[i]);
  if (!sameInputs) return { result: null, error: "" };
  return { result: keysStored ? record.result : null, error: record.error };
}

export interface AlignInput {
  binId: string;
  path: string;
}

export function alignInputs(state: WizardState): AlignInput[] {
  return state.bins.flatMap((bin) => {
    if (bin.files.length === 0) return [];
    const path = resolveChannelPath(state, bin.id, "stacked");
    return path ? [{ binId: bin.id, path }] : [];
  });
}

export function sameAlignInputs(a: readonly AlignInput[], b: readonly AlignInput[]): boolean {
  return a.length === b.length && a.every((input, i) => input.binId === b[i].binId && input.path === b[i].path);
}

export const ALIGN_INPUTS_CHANGED = "Inputs changed while aligning; run Align again";

export function alignRunFinish<Result>(
  started: readonly AlignInput[],
  current: readonly AlignInput[],
  result: Result | null,
  error: string,
): { record: AlignRunRecord<Result>; store: boolean } {
  if (!sameAlignInputs(started, current)) {
    return {
      record: { running: false, inputs: current.map((c) => c.path), result: null, error: ALIGN_INPUTS_CHANGED },
      store: false,
    };
  }
  return {
    record: { running: false, inputs: started.map((c) => c.path), result, error },
    store: result !== null && error === "",
  };
}

export type AlignRunEvent<Result> =
  | { type: "start"; run: AlignRunRecord<Result> }
  | { type: "finish"; started: AlignRunRecord<Result>; run: AlignRunRecord<Result> }
  | { type: "reset" };

export function nextAlignRun<Result>(
  current: AlignRunRecord<Result> | null,
  event: AlignRunEvent<Result>,
): AlignRunRecord<Result> | null {
  switch (event.type) {
    case "start":
      return event.run;
    case "finish":
      return current === event.started ? event.run : current;
    case "reset":
      return null;
  }
}

export const BIN_MENU_WIDTH = 200;

export const BIN_MENU_MAX_HEIGHT = 180;

const MENU_GAP = 4;

const VIEWPORT_MARGIN = 8;

export interface MenuAnchor {
  left: number;
  top: number;
  bottom: number;
}

export interface MenuPlacement {
  left: number;
  top: number | null;
  bottom: number | null;
}

export function binMenuPlacement(anchor: MenuAnchor, viewport: { width: number; height: number }): MenuPlacement {
  const left = Math.max(VIEWPORT_MARGIN, Math.min(anchor.left, viewport.width - BIN_MENU_WIDTH - VIEWPORT_MARGIN));
  const spaceBelow = viewport.height - anchor.bottom - MENU_GAP - VIEWPORT_MARGIN;
  const spaceAbove = anchor.top - MENU_GAP - VIEWPORT_MARGIN;
  if (spaceBelow >= BIN_MENU_MAX_HEIGHT || spaceBelow >= spaceAbove) {
    return { left, top: anchor.bottom + MENU_GAP, bottom: null };
  }
  return { left, top: null, bottom: viewport.height - anchor.top + MENU_GAP };
}

export type MenuKeyAction =
  | { type: "close"; preventDefault: boolean }
  | { type: "focus"; index: number };

export function binMenuKeyAction(key: string, current: number, count: number): MenuKeyAction | null {
  if (key === "Escape") return { type: "close", preventDefault: true };
  if (key === "Tab") return { type: "close", preventDefault: false };
  if (count <= 0) return null;
  switch (key) {
    case "ArrowDown":
      return { type: "focus", index: current < 0 ? 0 : (current + 1) % count };
    case "ArrowUp":
      return { type: "focus", index: current < 0 ? count - 1 : (current - 1 + count) % count };
    case "Home":
      return { type: "focus", index: 0 };
    case "End":
      return { type: "focus", index: count - 1 };
    default:
      return null;
  }
}

export function withChannelStage(
  results: Record<string, ChannelResult>,
  binId: string,
  stage: "starless" | "stretched",
  value: ChannelStage,
): Record<string, ChannelResult> {
  const next: ChannelResult = stage === "starless"
    ? { starless: value, stretched: null }
    : { starless: results[binId]?.starless ?? null, stretched: value };
  return { ...results, [binId]: next };
}

export function channelOutputPaths(state: WizardState): string[] {
  const paths: string[] = [];
  for (const result of Object.values(state.channelResults)) {
    if (result.starless) paths.push(result.starless.path);
    if (result.stretched) paths.push(result.stretched.path);
  }
  return paths;
}

export function droppedChannelOutputs(before: WizardState, after: WizardState): string[] {
  const kept = new Set(channelOutputPaths(after));
  return channelOutputPaths(before).filter((p) => !kept.has(p));
}

export function channelExportHistory(state: WizardState, exported: (string | null)[]): string[] {
  const lines: string[] = [];
  const mapping = fixedExportBins(state);
  if (mapping) {
    const id = (binId: string) => binId.toUpperCase();
    lines.push(`Channel mapping: R=${id(mapping.r)} G=${id(mapping.g)} B=${id(mapping.b)} (fixed, Blend weights not applied)`);
  }
  for (const bin of state.bins) {
    const result = state.channelResults[bin.id];
    const out = resolveOutputChannelPath(state, bin.id);
    if (!result || !out || !exported.includes(out)) continue;
    if (result.starless) lines.push(`Channel ${bin.id}: ${result.starless.note}`);
    if (result.stretched) lines.push(`Channel ${bin.id}: ${result.stretched.note}`);
  }
  return lines;
}

function starRemovalParams(sigma: number, growth: number): string {
  return `${sigma.toFixed(1)} sigma, growth ${growth.toFixed(2)}x FWHM`;
}

export function starRemovalNote(sigma: number, growth: number): string {
  return `star removal ${starRemovalParams(sigma, growth)}`;
}

const percent = (v: number) => `${Math.round(v * 100)}%`;

export function applyCompositeOp(history: CompositeHistory, op: CompositeOp): CompositeHistory {
  switch (op.kind) {
    case "blend":
      return { blend: `Blend: ${op.preset}`, levels: (op.levels ?? []).map(levelMatchLine), colorBalance: [], after: [] };
    case "colorBalance": {
      const lines: string[] = [];
      if (op.r !== 1 || op.g !== 1 || op.b !== 1) {
        lines.push(`White balance: ${op.mode} R=${op.r.toFixed(3)} G=${op.g.toFixed(3)} B=${op.b.toFixed(3)}`);
      }
      if (op.scnr) lines.push(`SCNR: ${op.scnr.method} ${percent(op.scnr.amount)}`);
      return { ...history, colorBalance: lines };
    }
    case "resetColorBalance":
      return { ...history, colorBalance: [] };
    case "lrgb":
      return { ...history, after: [...history.after, `LRGB: lightness ${percent(op.lightness)}, chrominance ${percent(op.chrominance)}`] };
    case "starRemoval":
      return { ...history, after: [...history.after, `Star removal: ${starRemovalParams(op.sigma, op.growth)}`] };
  }
}

export function compositeHistoryLines(history: CompositeHistory): string[] {
  return [...(history.blend ? [history.blend] : []), ...history.levels, ...history.colorBalance, ...history.after];
}

export function levelMatchFactors(entry: LevelMatchEntry): string {
  return `x${entry.scale.toPrecision(4)} ${entry.z >= 0 ? "+" : "-"}${Math.abs(entry.z).toPrecision(4)}`;
}

export function levelMatchLine(entry: LevelMatchEntry): string {
  return `Level match ${entry.channel}: ${levelMatchFactors(entry)}`;
}

export function wizardStackName(binId: string, drizzle: boolean, runId: number): string {
  return `${drizzle ? "drizzle" : "stacked"}_${binId}_${runId}`;
}

export interface MaskedChannelSummary {
  iterations_run?: number;
  converged?: boolean;
}

export interface StretchRunSummaryInput {
  elapsed_ms?: number;
  iterations_run?: number;
  converged?: boolean;
  stretch_factor?: number;
  channels?: Partial<Record<"r" | "g" | "b", MaskedChannelSummary>>;
}

const SUMMARY_CHANNELS = ["r", "g", "b"] as const;

export function stretchRunSummary(result: StretchRunSummaryInput): string {
  const parts = [`${result.elapsed_ms ?? 0}ms`];
  const perChannel = SUMMARY_CHANNELS.flatMap((k) => {
    const stats = result.channels?.[k];
    return stats ? [{ label: k.toUpperCase(), stats }] : [];
  });
  if (perChannel.length > 0) {
    const iterations = perChannel.filter((c) => typeof c.stats.iterations_run === "number");
    if (iterations.length > 0) {
      parts.push(`iterations ${iterations.map((c) => `${c.label} ${c.stats.iterations_run}`).join(" / ")}`);
    }
    const flagged = perChannel.filter((c) => typeof c.stats.converged === "boolean");
    if (flagged.length > 0) {
      const failed = flagged.filter((c) => !c.stats.converged).map((c) => c.label);
      parts.push(failed.length === 0 ? "converged" : `not converged (${failed.join(", ")})`);
    }
  } else {
    if (result.iterations_run) parts.push(`${result.iterations_run} iterations`);
    if (result.converged !== undefined) parts.push(result.converged ? "converged" : "not converged");
  }
  if (result.stretch_factor) parts.push(`factor=${result.stretch_factor}`);
  return parts.join(", ");
}

export interface BackgroundRunSummaryInput {
  sample_count?: number | null;
  rms_residual?: number | null;
  elapsed_ms?: number;
}

export interface ToneRunSummaryInput {
  elapsed_ms: number;
  curves_applied: boolean;
  contrast_reapplied?: string[];
}

const CONTRAST_LABELS: Record<string, string> = { lhe: "LHE", hdr: "HDRMT" };

export function toneRunSummary(result: ToneRunSummaryInput): string {
  const parts = [`${result.elapsed_ms}ms`];
  if (result.curves_applied) parts.push("curves applied");
  const reapplied = (result.contrast_reapplied ?? []).map((op) => CONTRAST_LABELS[op] ?? op);
  if (reapplied.length > 0) parts.push(`${reapplied.join(" + ")} re-applied on the new curves`);
  return parts.join(" | ");
}

export function backgroundRunSummary(result: BackgroundRunSummaryInput): string {
  const parts: string[] = [];
  if (result.sample_count != null) parts.push(`${result.sample_count} samples`);
  if (result.rms_residual != null) parts.push(`RMS ${result.rms_residual.toFixed(4)}`);
  parts.push(`${result.elapsed_ms ?? 0}ms`);
  return parts.join(", ");
}
