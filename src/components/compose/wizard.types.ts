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

export interface WizardState {
  bins: FrequencyBin[];
  stackedPaths: Record<string, string>;
  backgroundPaths: Record<string, string>;
  alignedPaths: Record<string, string>;
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
  linkedStf: boolean;
  resultPng: string | null;
  resultFits: string | null;
  completedSteps: Record<string, boolean>;
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
  backgroundPaths: {},
  alignedPaths: {},
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
  linkedStf: true,
  resultPng: null,
  resultFits: null,
  completedSteps: {},
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

function totalFiles(s: WizardState): number {
  return s.bins.reduce((acc, b) => acc + b.files.length, 0);
}

export const STEPS: StepDef[] = [
  {
    id: "channels",
    label: "Channel Assignment",
    shortLabel: "Channels",
    color: "violet",
    enabled: () => true,
    badge: (s) => {
      const n = totalFiles(s);
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
    id: "background",
    label: "Background Extraction",
    shortLabel: "BG",
    color: "emerald",
    enabled: (s) => totalFiles(s) > 0,
    badge: (s) => {
      const n = Object.keys(s.backgroundPaths).length;
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
    id: "blend",
    label: "Channel Blending",
    shortLabel: "Blend",
    color: "amber",
    enabled: (s) => filledCount(s) >= 2,
    badge: (s) => s.compositeReady ? "✓" : null,
  },
  {
    id: "calibrate",
    label: "Color Calibration",
    shortLabel: "Color",
    color: "cyan",
    enabled: (s) => s.compositeReady || filledCount(s) >= 2,
  },
  {
    id: "mask",
    label: "Star Mask",
    shortLabel: "Mask",
    color: "rose",
    enabled: (s) => totalFiles(s) > 0,
  },
  {
    id: "stretch",
    label: "Stretch",
    shortLabel: "Stretch",
    color: "amber",
    enabled: (s) => s.compositeReady || totalFiles(s) > 0,
  },
  {
    id: "color",
    label: "Color Adjust",
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
