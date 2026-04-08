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

export interface WizardState {
  bins: FrequencyBin[];
  stackedPaths: Record<string, string>;
  alignedPaths: Record<string, string>;
  croppedPaths: Record<string, string>;
  backgroundPaths: Record<string, string>;
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
  stretchMode: "masked" | "arcsinh" | "auto_stf";
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
  { id: "oiii", label: "OIII (502nm)", shortLabel: "OIII", wavelength: 502, color: "#3b82f6", files: [] },
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

const NB_FILTERS = new Set(["Hα (656nm)", "[OIII] (502nm)", "[SII] (673nm)"]);

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
    id: "mask",
    label: "Star Mask",
    shortLabel: "Mask",
    color: "rose",
    enabled: (s) => totalFilesCount(s) > 0,
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
  if (clear("blend")) partial.compositeReady = false;

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

export function resolveChannelPath(state: WizardState, binId: string): string | null {
  if (state.backgroundPaths[binId]) return state.backgroundPaths[binId];
  if (state.croppedPaths[binId]) return state.croppedPaths[binId];
  if (state.alignedPaths[binId]) return state.alignedPaths[binId];
  if (state.stackedPaths[binId]) return state.stackedPaths[binId];
  const bin = state.bins.find((b) => b.id === binId);
  if (bin && bin.files.length > 0) return bin.files[0];
  return null;
}

export function resolveAnyChannelPath(state: WizardState): string | null {
  for (const bin of state.bins) {
    if (state.backgroundPaths[bin.id]) return state.backgroundPaths[bin.id];
    if (state.croppedPaths[bin.id]) return state.croppedPaths[bin.id];
    if (state.alignedPaths[bin.id]) return state.alignedPaths[bin.id];
    if (state.stackedPaths[bin.id]) return state.stackedPaths[bin.id];
    if (bin.files.length > 0) return bin.files[0];
  }
  return null;
}

export function resolveRgbPaths(state: WizardState): { r: string | null; g: string | null; b: string | null } {
  const activeBins = state.bins.filter((b) => b.files.length > 0);
  const rCandidates = ["r", "sii", "ha"];
  const gCandidates = ["g", "ha", "oiii"];
  const bCandidates = ["b", "oiii", "sii"];
  const usedIds = new Set<string>();

  const findBest = (candidates: string[], allowReuse = false): string | null => {
    for (const cid of candidates) {
      if (!allowReuse && usedIds.has(cid)) continue;
      const bin = activeBins.find((b) => b.id === cid);
      if (bin) {
        usedIds.add(cid);
        return resolveChannelPath(state, cid);
      }
    }
    return null;
  };

  const r = findBest(rCandidates);
  const g = findBest(gCandidates);
  let b = findBest(bCandidates);
  if (!b) b = findBest(bCandidates, true);

  return { r, g, b };
}
