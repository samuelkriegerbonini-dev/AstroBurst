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

export interface CompositeHistory {
  blend: string | null;
  colorBalance: string[];
  after: string[];
}

export const EMPTY_COMPOSITE_HISTORY: CompositeHistory = { blend: null, colorBalance: [], after: [] };

export type CompositeOp =
  | { kind: "blend"; preset: string }
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
  linkedStf: boolean;
  resultPng: string | null;
  resultFits: string | null;
  completedSteps: Record<string, boolean>;
  subframeResults: Record<string, SubframeAnalysisResult>;
  excludedFiles: Record<string, string[]>;
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
  linkedStf: true,
  resultPng: null,
  resultFits: null,
  completedSteps: {},
  subframeResults: {},
  excludedFiles: {},
};

export interface StepDef {
  id: string;
  label: string;
  shortLabel: string;
  color: string;
  enabled: (state: WizardState) => boolean;
  badge?: (state: WizardState) => string | null;
}

function filledCount(s: WizardState): number {
  return s.bins.filter((b) => b.files.length > 0).length;
}

function totalFilesCount(s: WizardState): number {
  return s.bins.reduce((acc, b) => acc + b.files.length, 0);
}

const NARROWBAND_IDS = new Set(["ha", "sii", "nii", "oiii", "hb"]);

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
    enabled: () => true,
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
    enabled: (s) => s.bins.some((b) => b.files.length > 1),
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
    enabled: (s) => filledCount(s) >= 2,
  },
  {
    id: "crop",
    label: "Crop",
    shortLabel: "Crop",
    color: "cyan",
    enabled: (s) => Object.keys(s.alignedPaths).length > 0,
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
    enabled: (s) =>
      Object.keys(s.alignedPaths).length > 0 ||
      Object.keys(s.croppedPaths).length > 0 ||
      totalFilesCount(s) > 0,
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
    enabled: (s) => filledCount(s) >= 2,
    badge: (s) => s.compositeReady ? "✓" : null,
  },
  {
    id: "colorbalance",
    label: "Color Balance",
    shortLabel: "Color",
    color: "cyan",
    enabled: (s) => s.compositeReady || filledCount(s) >= 2,
  },
  {
    id: "stretch",
    label: "Stretch",
    shortLabel: "Stretch",
    color: "amber",
    enabled: (s) => s.compositeReady || totalFilesCount(s) > 0,
  },
  {
    id: "adjust",
    label: "Adjust",
    shortLabel: "Adjust",
    color: "purple",
    enabled: (s) => s.compositeReady,
  },
  {
    id: "export",
    label: "Export",
    shortLabel: "Export",
    color: "teal",
    enabled: () => true,
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

  if (clear("align")) partial.alignedPaths = {};
  if (clear("crop")) partial.croppedPaths = {};
  if (clear("background")) partial.backgroundPaths = {};
  if (clear("blend")) {
    partial.compositeReady = false;
    partial.compositeHistory = EMPTY_COMPOSITE_HISTORY;
  }
  if (clear("stretch")) partial.channelResults = {};

  return partial;
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
  if (bin && bin.files.length > 0) return bin.files[0];
  return null;
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

export function resolveRgbPaths(
  state: WizardState,
  useChannelOutputs = false,
): { r: string | null; g: string | null; b: string | null } {
  const activeBins = state.bins.filter((b) => b.files.length > 0);
  const rCandidates = ["r", "sii", "ha"];
  const gCandidates = ["g", "ha", "oiii"];
  const bCandidates = ["b", "oiii", "sii"];
  const usedIds = new Set<string>();
  const pathOf = (binId: string) =>
    useChannelOutputs ? resolveOutputChannelPath(state, binId) : resolveChannelPath(state, binId);
  const anyPath = (): string | null => {
    for (const bin of state.bins) {
      const p = pathOf(bin.id);
      if (p) return p;
    }
    return null;
  };

  const findBest = (candidates: string[], allowReuse = false): string | null => {
    for (const cid of candidates) {
      if (!allowReuse && usedIds.has(cid)) continue;
      const bin = activeBins.find((b) => b.id === cid);
      if (bin) {
        usedIds.add(cid);
        return pathOf(cid);
      }
    }
    return null;
  };

  const r = findBest(rCandidates);
  const g = findBest(gCandidates);
  let b = findBest(bCandidates);
  if (!b) b = findBest(bCandidates, true);

  const fillFromUnused = (): string | null => {
    const bin = activeBins.find((bn) => !usedIds.has(bn.id));
    if (!bin) return null;
    usedIds.add(bin.id);
    return pathOf(bin.id);
  };

  return {
    r: r ?? fillFromUnused() ?? anyPath(),
    g: g ?? fillFromUnused() ?? anyPath(),
    b: b ?? fillFromUnused() ?? anyPath(),
  };
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
    ? active[0]
    : monoBinId
      ? active.find((b) => b.id === monoBinId)
      : active.length === 1
        ? active[0]
        : undefined;
  if (!bin) return null;
  return state.stackedPaths[bin.id] ?? bin.files[0] ?? null;
}

export function wizardZipChannels(state: WizardState): { name: string; path: string }[] {
  const channels = state.compositeReady ? { ...resolveRgbPaths(state), monoBinId: null } : resolveExportRgbPaths(state);
  const entries = channels.monoBinId
    ? [{ name: `channel_${channels.monoBinId}`, path: channels.r }]
    : (["r", "g", "b"] as const).map((key) => ({ name: `channel_${key}`, path: channels[key] }));
  return entries.filter((e): e is { name: string; path: string } => e.path !== null);
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
      return { blend: `Blend: ${op.preset}`, colorBalance: [], after: [] };
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
  return [...(history.blend ? [history.blend] : []), ...history.colorBalance, ...history.after];
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
