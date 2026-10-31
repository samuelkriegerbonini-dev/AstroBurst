import { useState, useCallback, useId, useRef, useEffect, useMemo } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { X } from "lucide-react";
import { Slider, Toggle, RunButton, ErrorAlert, SectionHeader, WarningList } from "../ui";
import { runCalibrationPipeline } from "../../services/stacking";
import { cancelProgress } from "../../services/progress";
import { useProgress } from "../../hooks/useProgress";
import { useTimer } from "../../hooks/useTimer";
import { getOutputDir } from "../../infrastructure/tauri";
import { PIPELINE_PROGRESS_EVENT, type CombineMethod, type NormalizationMethod, type PipelineDarkGroup, type PipelineMasterOutput, type PipelineResult, type PipelineStats } from "../../shared/types/stacking";
import { isCancelledRun } from "../../utils/drizzleRgbResult";
import { pipelineProgressText } from "../../utils/pipelineProgress";
import { formatTime } from "../../utils/format";
import type { CosmeticConfig } from "../../shared/types/cosmetic";
import {
  boundRejectionSettings,
  COMBINE_OPTIONS,
  DEFAULT_STACK_SETTINGS,
  NORMALIZATION_OPTIONS,
  pipelineRejectionFields,
  type RejectionSettings,
} from "../../utils/stackingRejection";
import { parseDefectList, formatDefectError } from "../../utils/defectList";
import { detectChannel, displayFilterValue, filterCardValue } from "../../utils/channelMapping";
import { laneForBin, MONO_LANE, PIPELINE_LANES, type PipelineLane } from "../../utils/pipelineToCompose";
import { openInFiles } from "./resultHandoff";
import RejectionControls from "./RejectionControls";
import { calibrationPaths, isStackedProduct, sortByFrameKind, stackedLeftOutLine, type CalibrationFrameKind, type CalibrationFrames } from "../../utils/frameKind";
import { formatCount } from "../../utils/formatCount";
import { PIPELINE_RGB_CHOICE, pipelineInitialChoice, pipelineOutputName, stackedLights, toDims } from "../../utils/stackingOutputs";
import { cullChannelGroups, pipelineChannelInputs, subframeExclusionNotice } from "../../utils/subframeCull";
import { CONFIRM_CLEAR_LABEL, useArmedConfirm } from "../../hooks/useArmedConfirm";
import type { ProcessedFile } from "../../shared/types";
import type { CalibrationState, RunTarget, StackConfig } from "./StackingTab";

interface FileGroup {
  label: string;
  paths: string[];
}

const NO_PATHS: readonly string[] = [];
const NO_FILTER_KEY = "no filter";
const LANE_COLORS: Record<PipelineLane, string> = {
  L: "#a1a1aa",
  R: "#ef4444",
  G: "#22c55e",
  B: "#3b82f6",
  Ha: "#fb7185",
  OIII: "#2dd4bf",
  SII: "#f97316",
  Mono: "#d4d4d8",
};
const LANE_BUTTON_CLASS = "text-[10px] text-zinc-500 hover:text-zinc-300 bg-zinc-800 px-2 py-0.5 rounded";
const HANDOFF_BUTTON_CLASS = "rounded-md bg-sky-600/20 px-2 py-1 text-[11px] text-sky-200 transition-colors hover:bg-sky-600/30";
const ARMED_HANDOFF_BUTTON_CLASS = "rounded-md bg-amber-500/15 px-2 py-1 text-[11px] text-amber-300 ring-1 ring-amber-500/30";
const COMPOSE_CONFIRM_KEY = "compose";
const MAX_DEFECT_ERRORS = 3;
const DEFECT_PLACEHOLDER = "Point x y\nCol x [y0 y1]\nRow y [x0 x1]\n# 0-based, one entry per line";
const PIPELINE_REJECTION_DEFAULTS: RejectionSettings = { ...boundRejectionSettings(DEFAULT_STACK_SETTINGS), sigmaLow: 2.5, sigmaHigh: 3.0 };

function normalizeNoteText(stackNormalization: NormalizationMethod | undefined, normalize: boolean): string | null {
  if (stackNormalization === undefined || (!normalize && stackNormalization === "none")) return null;
  const method = NORMALIZATION_OPTIONS.find((option) => option.value === stackNormalization)?.label ?? stackNormalization;
  const pipeline = normalize ? "this divides each light by its own sky background instead." : "the Pipeline stacks these lights without normalization.";
  return `The Stack tab normalization is ${method}; ${pipeline}`;
}

const ICON = (
  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-blue-400">
    <path d="M4 4h16v16H4z" />
    <path d="M4 12h16M12 4v16" opacity="0.3" />
  </svg>
);

interface KeyedPath {
  path: string;
  key: string;
}

interface SeededLight extends KeyedPath {
  home: PipelineLane | null;
}

interface FilterGroup {
  key: string;
  paths: string[];
}

interface LaneSeed {
  lanes: FileGroup[];
  lights: SeededLight[];
  stacked: string[];
}

interface LaneAddition {
  added: string[];
  refused: KeyedPath[];
}

type FilterKeys = ReadonlyMap<string, string>;

function inLaneOrder(groups: FileGroup[]): FileGroup[] {
  const rank = (label: string) => PIPELINE_LANES.indexOf(label as PipelineLane);
  return [...groups].sort((a, b) => rank(a.label) - rank(b.label));
}

function placeLight(file: ProcessedFile): Omit<SeededLight, "path"> {
  const lane = laneForBin(detectChannel(file));
  if (lane) return { key: lane, home: lane };
  const card = filterCardValue(file);
  if (card === null) return { key: NO_FILTER_KEY, home: MONO_LANE };
  return { key: displayFilterValue(file) ?? `wheel position ${card}`, home: null };
}

function laneTakes(lane: FileGroup, key: string, keys: FilterKeys): boolean {
  return lane.paths.every((path) => {
    const held = keys.get(path);
    return held === undefined || held === key;
  });
}

function isLeftOut(light: SeededLight, lanes: readonly FileGroup[], keys: FilterKeys): boolean {
  if (light.home === null) return true;
  const home = lanes.find((lane) => lane.label === light.home);
  return home !== undefined && !laneTakes(home, light.key, keys);
}

function byFilter(lights: readonly KeyedPath[]): FilterGroup[] {
  const groups = new Map<string, string[]>();
  for (const light of lights) {
    const paths = groups.get(light.key);
    if (paths) paths.push(light.path);
    else groups.set(light.key, [light.path]);
  }
  return [...groups].map(([key, paths]) => ({ key, paths }));
}

function lightsCounted(groups: readonly FilterGroup[]): string {
  const total = groups.reduce((sum, group) => sum + group.paths.length, 0);
  return `${total} light${total === 1 ? "" : "s"}`;
}

function filterCounts(groups: readonly FilterGroup[]): string {
  return groups.map((group) => `${group.key} (${group.paths.length})`).join(", ");
}

function filterLeftOutLine(groups: readonly FilterGroup[]): string | null {
  if (groups.length === 0) return null;
  return `${lightsCounted(groups)} left out by filter: ${filterCounts(groups)}`;
}

function laneRefusalNote(label: string, refused: readonly KeyedPath[]): string | null {
  if (refused.length === 0) return null;
  const groups = byFilter(refused);
  return `${lightsCounted(groups)} not added to the ${label} lane: ${filterCounts(groups)}; a lane holds one filter`;
}

function seedLanes(files: ProcessedFile[]): LaneSeed {
  const lanePaths = new Map<string, string[]>();
  const lights: SeededLight[] = [];
  const stacked: string[] = [];
  for (const file of sortByFrameKind(files).lights) {
    if (!file.path) continue;
    if (isStackedProduct(file.result?.header)) {
      stacked.push(file.path);
      continue;
    }
    const light: SeededLight = { path: file.path, ...placeLight(file) };
    lights.push(light);
    if (light.home === null) continue;
    const paths = lanePaths.get(light.home);
    if (paths) paths.push(light.path);
    else lanePaths.set(light.home, [light.path]);
  }
  const lanes = PIPELINE_LANES.flatMap((label) => {
    const paths = lanePaths.get(label);
    return paths ? [{ label, paths }] : [];
  });
  return { lanes, lights, stacked };
}

function laneAddition(current: readonly FileGroup[], label: string, picked: readonly string[], keys: FilterKeys): LaneAddition {
  const held = new Set(current.flatMap((group) => group.paths));
  let laneKey = current.find((group) => group.label === label)?.paths.map((path) => keys.get(path)).find((key) => key !== undefined);
  const added: string[] = [];
  const refused: KeyedPath[] = [];
  for (const path of picked) {
    if (held.has(path)) continue;
    held.add(path);
    const key = keys.get(path);
    if (key !== undefined && laneKey !== undefined && key !== laneKey) {
      refused.push({ path, key });
      continue;
    }
    if (laneKey === undefined) laneKey = key;
    added.push(path);
  }
  return { added, refused };
}

function calibrationFromFiles(files: ProcessedFile[]): CalibrationFrames<string> {
  return calibrationPaths(sortByFrameKind(files.filter((file) => file.path)).calibration);
}

function withMissingPaths(current: string[], incoming: readonly string[]): string[] {
  const present = new Set(current);
  const missing = incoming.filter((p) => !present.has(p));
  return missing.length === 0 ? current : [...current, ...missing];
}

function mergeChannelSeed(current: FileGroup[], seed: FileGroup[], keys: FilterKeys): FileGroup[] {
  let changed = false;
  const merged = current.map((group) => {
    const incoming = seed.find((s) => s.label === group.label);
    const incomingKey = incoming && keys.get(incoming.paths[0]);
    if (!incoming || incomingKey === undefined || !laneTakes(group, incomingKey, keys)) return group;
    const paths = withMissingPaths(group.paths, incoming.paths);
    if (paths === group.paths) return group;
    changed = true;
    return { ...group, paths };
  });
  const added = seed.filter((s) => !current.some((group) => group.label === s.label));
  if (!changed && added.length === 0) return current;
  return inLaneOrder([...merged, ...added]);
}

const CLEAR_BUTTON_CLASS = "text-[10px] text-red-400 hover:text-red-300 bg-zinc-800 px-2 py-0.5 rounded";

function ClearButton({ armed, onPress, onBlur }: { armed: boolean; onPress: () => void; onBlur: () => void }) {
  return (
    <button type="button" aria-live="polite" onClick={onPress} onBlur={onBlur} className={CLEAR_BUTTON_CLASS}>
      {armed ? CONFIRM_CLEAR_LABEL : "Clear"}
    </button>
  );
}

const CalibRow = ({ label, addLabel, count, onAdd, clearArmed, onClear, onClearBlur, testId }: { label: string; addLabel: string; count: number; onAdd: () => void; clearArmed: boolean; onClear: () => void; onClearBlur: () => void; testId?: string }) => (
  <div className="flex items-center justify-between" data-testid={testId}>
    <span className="text-xs text-zinc-400">{label}: {count}</span>
    <div className="flex gap-1">
      <button aria-label={addLabel} onClick={onAdd} className="text-[10px] text-zinc-500 hover:text-zinc-300 bg-zinc-800 px-2 py-0.5 rounded">+ Add</button>
      {count > 0 && <ClearButton armed={clearArmed} onPress={onClear} onBlur={onClearBlur} />}
    </div>
  </div>
);

const CALIBRATION_FRAMES: Record<CalibrationFrameKind, string> = {
  dark: "dark frames",
  flat: "flat frames",
  flatdark: "flat-dark frames",
  bias: "bias frames",
};

function darkGroupsLine(groups: readonly PipelineDarkGroup[] | undefined): string | null {
  if (!groups || !groups.some((g) => typeof g.temp_c === "number")) return null;
  const parts = groups.map((g) => (typeof g.temp_c === "number" ? `${g.temp_c.toFixed(1)} °C (${g.frames})` : `unknown (${g.frames})`));
  return `dark groups: ${parts.join(", ")}`;
}

export const COSMETIC_AUTO_NOTE =
  "Auto detect replaces pixels in every light without a preview; check one light first with Cosmetic > Dry run.";

export function CosmeticAutoNote({ enabled }: { enabled: boolean }) {
  if (!enabled) return null;
  return <p className="text-[10px] text-amber-400/80 leading-snug">{COSMETIC_AUTO_NOTE}</p>;
}

export function PipelineMasterLines({ stats }: { stats: PipelineStats }) {
  const flatDarks = stats.flat_darks_combined ?? 0;
  const groups = darkGroupsLine(stats.dark_groups);
  return (
    <>
      {stats.darks_combined > 0 && <div>{`Master dark: ${stats.darks_combined} frames`}</div>}
      {stats.flats_combined > 0 && <div>{`Master flat: ${stats.flats_combined} frames`}</div>}
      {flatDarks > 0 && <div>{`Master flat-dark: ${flatDarks} frames`}</div>}
      {groups && <div>{groups}</div>}
    </>
  );
}

interface PipelinePanelProps {
  files?: ProcessedFile[];
  calibration?: CalibrationState;
  stackConfig?: StackConfig;
  runTarget?: RunTarget | null;
  onShow?: (result: PipelineResult, choice: string, target: RunTarget | null) => void;
  rejectedPaths?: readonly string[];
  onUseAllFrames?: () => void;
  composeHasProgress?: boolean;
  onComposeMasters?: (masters: PipelineMasterOutput[]) => void;
}

export default function PipelinePanel({ files = [], calibration, stackConfig, runTarget = null, onShow, rejectedPaths = NO_PATHS, onUseAllFrames, composeHasProgress = false, onComposeMasters }: PipelinePanelProps) {
  const [result, setResult] = useState<PipelineResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [cancelled, setCancelled] = useState(false);
  const progress = useProgress(PIPELINE_PROGRESS_EVENT);
  const timer = useTimer();

  const seed = useMemo(() => seedLanes(files), [files]);
  const keyOfPath = useMemo<FilterKeys>(() => new Map(seed.lights.map((light) => [light.path, light.key])), [seed]);
  const [channels, setChannels] = useState<FileGroup[]>(() => seed.lanes);
  const channelsRef = useRef(channels);
  channelsRef.current = channels;
  const keyOfPathRef = useRef(keyOfPath);
  keyOfPathRef.current = keyOfPath;
  const { leftOutGroups, stackedLine } = useMemo(() => {
    const inLanes = new Set(channels.flatMap((group) => group.paths));
    const leftOut = seed.lights.filter((light) => !inLanes.has(light.path) && isLeftOut(light, channels, keyOfPath));
    return {
      leftOutGroups: byFilter(leftOut),
      stackedLine: stackedLeftOutLine(seed.stacked.filter((path) => !inLanes.has(path)).length, "the lanes"),
    };
  }, [seed, channels, keyOfPath]);
  const leftOutLine = filterLeftOutLine(leftOutGroups);
  const [putChoices, setPutChoices] = useState<Record<string, string>>({});
  const [laneNote, setLaneNote] = useState<string | null>(null);
  const [laneChoice, setLaneChoice] = useState("");
  const [loadedCalibration] = useState(() => calibrationFromFiles(files));
  const [darks, setDarks] = useState<string[]>(() => withMissingPaths(calibration?.darkPaths ?? [], loadedCalibration.dark));
  const [flats, setFlats] = useState<string[]>(() => withMissingPaths(calibration?.flatPaths ?? [], loadedCalibration.flat));
  const [flatDarks, setFlatDarks] = useState<string[]>(() => withMissingPaths(calibration?.flatDarkPaths ?? [], loadedCalibration.flatdark));
  const [bias, setBias] = useState<string[]>(() => withMissingPaths(calibration?.biasPaths ?? [], loadedCalibration.bias));
  const [rejectionSettings, setRejectionSettings] = useState<RejectionSettings>(() => (stackConfig ? boundRejectionSettings(stackConfig) : PIPELINE_REJECTION_DEFAULTS));
  const [combine, setCombine] = useState<CombineMethod>(() => stackConfig?.combine ?? "mean");
  const [normalize, setNormalize] = useState(true);
  const [align, setAlign] = useState(true);
  const [darkOptimize, setDarkOptimize] = useState(false);
  const [activePreview, setActivePreview] = useState<string | null>(null);

  const [cosmeticEnabled, setCosmeticEnabled] = useState(false);
  const [cosmeticUseDark, setCosmeticUseDark] = useState(true);
  const [cosmeticHotSigma, setCosmeticHotSigma] = useState(3.0);
  const [cosmeticColdEnabled, setCosmeticColdEnabled] = useState(false);
  const [cosmeticColdSigma, setCosmeticColdSigma] = useState(3.0);
  const [cosmeticAutoEnabled, setCosmeticAutoEnabled] = useState(false);
  const [cosmeticAutoSigma, setCosmeticAutoSigma] = useState(3.0);
  const [defectText, setDefectText] = useState("");

  const clearConfirm = useArmedConfirm();
  const composeConfirm = useArmedConfirm();
  const defectListId = useId();
  const combineId = useId();
  const normalizeNoteId = useId();

  const culled = useMemo(() => cullChannelGroups(channels, rejectedPaths), [channels, rejectedPaths]);
  const exclusionNotice = subframeExclusionNotice(culled.excluded);
  const parsedDefects = useMemo(() => parseDefectList(defectText), [defectText]);
  const defectErrors = cosmeticEnabled ? parsedDefects.errors : [];

  const rgbCanvasRef = useRef<HTMLCanvasElement>(null);
  const chCanvasRef = useRef<HTMLCanvasElement>(null);
  const decodedPreviewsRef = useRef(new Map<string, ImageData>());

  const seededFilesRef = useRef(files);
  useEffect(() => {
    const seeded = seededFilesRef.current;
    if (seeded === files) return;
    seededFilesRef.current = files;
    const known = new Set(seeded.map((file) => file.path));
    const fresh = files.filter((file) => !known.has(file.path));
    if (fresh.length === 0) return;
    const lanes = seedLanes(fresh).lanes;
    setChannels((prev) => mergeChannelSeed(prev, lanes, keyOfPath));
    const loaded = calibrationFromFiles(fresh);
    setDarks((prev) => withMissingPaths(prev, loaded.dark));
    setFlats((prev) => withMissingPaths(prev, loaded.flat));
    setFlatDarks((prev) => withMissingPaths(prev, loaded.flatdark));
    setBias((prev) => withMissingPaths(prev, loaded.bias));
  }, [files, keyOfPath]);

  const seededCalibrationRef = useRef(calibration);
  useEffect(() => {
    if (seededCalibrationRef.current === calibration) return;
    seededCalibrationRef.current = calibration;
    if (!calibration) return;
    setDarks((prev) => withMissingPaths(prev, calibration.darkPaths));
    setFlats((prev) => withMissingPaths(prev, calibration.flatPaths));
    setBias((prev) => withMissingPaths(prev, calibration.biasPaths));
    setFlatDarks((prev) => withMissingPaths(prev, calibration.flatDarkPaths));
  }, [calibration]);

  const seededConfigRef = useRef(stackConfig);
  const configTouchedRef = useRef(false);
  useEffect(() => {
    if (seededConfigRef.current === stackConfig) return;
    seededConfigRef.current = stackConfig;
    if (!stackConfig || configTouchedRef.current) return;
    setRejectionSettings(boundRejectionSettings(stackConfig));
    setCombine(stackConfig.combine);
  }, [stackConfig]);

  const changeRejection = useCallback((change: Partial<RejectionSettings>) => {
    configTouchedRef.current = true;
    setRejectionSettings((prev) => ({ ...prev, ...change }));
  }, []);

  const buildCosmetic = useCallback((): CosmeticConfig | null => {
    if (!cosmeticEnabled) return null;
    return {
      use_master_dark: cosmeticUseDark,
      dark_hot_sigma: cosmeticUseDark ? cosmeticHotSigma : null,
      dark_cold_sigma: cosmeticUseDark && cosmeticColdEnabled ? cosmeticColdSigma : null,
      auto_hot_sigma: cosmeticAutoEnabled ? cosmeticAutoSigma : null,
      auto_cold_sigma: null,
      defects: parsedDefects.defects,
      cfa: null,
      amount: 1,
      replacement: "median",
    };
  }, [cosmeticEnabled, cosmeticUseDark, cosmeticHotSigma, cosmeticColdEnabled, cosmeticColdSigma, cosmeticAutoEnabled, cosmeticAutoSigma, parsedDefects]);

  const pickFiles = useCallback(async (title: string): Promise<string[]> => {
    const selected = await open({
      multiple: true,
      filters: [{ name: "FITS", extensions: ["fits", "fit", "fts"] }],
      title,
    });
    if (!selected) return [];
    if (Array.isArray(selected)) return selected;
    return [selected];
  }, []);

  const addToChannel = useCallback(
    async (label: string) => {
      const paths = await pickFiles(`Select ${label} lights`);
      if (paths.length === 0) return;
      const { added, refused } = laneAddition(channelsRef.current, label, paths, keyOfPathRef.current);
      setLaneNote(laneRefusalNote(label, refused));
      if (added.length === 0) return;
      setChannels((prev) => prev.map((group) => (group.label === label ? { ...group, paths: withMissingPaths(group.paths, added) } : group)));
    },
    [pickFiles]
  );

  const missingLanes = PIPELINE_LANES.filter((label) => !channels.some((group) => group.label === label));
  const laneToAdd = missingLanes.find((label) => label === laneChoice) ?? missingLanes[0];

  const laneCandidates = (key: string) =>
    PIPELINE_LANES.filter((label) => {
      const lane = channels.find((group) => group.label === label);
      return lane === undefined || laneTakes(lane, key, keyOfPath);
    });

  const putLane = (key: string, candidates: readonly PipelineLane[]): PipelineLane =>
    candidates.find((label) => label === putChoices[key]) ?? (candidates.includes(MONO_LANE) ? MONO_LANE : candidates[0]);

  const putInLane = (group: FilterGroup, label: PipelineLane) => {
    setChannels((prev) => {
      const lane = prev.find((g) => g.label === label);
      if (!lane) return inLaneOrder([...prev, { label, paths: [...group.paths] }]);
      if (!laneTakes(lane, group.key, keyOfPath)) return prev;
      const paths = withMissingPaths(lane.paths, group.paths);
      return paths === lane.paths ? prev : prev.map((g) => (g === lane ? { ...g, paths } : g));
    });
    setLaneNote(null);
  };

  const addLane = () => {
    if (!laneToAdd) return;
    setChannels((prev) => (prev.some((group) => group.label === laneToAdd) ? prev : inLaneOrder([...prev, { label: laneToAdd, paths: [] }])));
  };

  const removeLane = (label: string) => {
    setChannels((prev) => prev.filter((group) => group.label !== label || group.paths.length > 0));
  };

  const clearLane = (label: string) => {
    setChannels((prev) => prev.map((group) => (group.label === label ? { ...group, paths: [] } : group)));
  };

  const addCalibration = useCallback(
    async (type: CalibrationFrameKind) => {
      const paths = await pickFiles(`Select ${CALIBRATION_FRAMES[type]}`);
      if (paths.length === 0) return;
      switch (type) {
        case "dark": setDarks((p) => withMissingPaths(p, paths)); break;
        case "flat": setFlats((p) => withMissingPaths(p, paths)); break;
        case "flatdark": setFlatDarks((p) => withMissingPaths(p, paths)); break;
        case "bias": setBias((p) => withMissingPaths(p, paths)); break;
      }
    },
    [pickFiles]
  );

  const handleRun = async () => {
    const { inputs: channelInputs } = pipelineChannelInputs(channels, rejectedPaths);

    if (channelInputs.length === 0 || defectErrors.length > 0) return;

    const target = runTarget;
    const lightCount = channelInputs.reduce((sum, c) => sum + c.paths.length, 0);
    const name = pipelineOutputName(channelInputs[0].paths[0], lightCount, new Date());
    setLoading(true);
    setError(null);
    setCancelled(false);
    setResult(null);
    setActivePreview(null);
    progress.reset();
    timer.reset();
    timer.start();
    try {
      const res = await runCalibrationPipeline({
        channels: channelInputs,
        dark_paths: darks,
        flat_paths: flats,
        bias_paths: bias,
        flat_dark_paths: flatDarks,
        ...pipelineRejectionFields(rejectionSettings),
        normalize,
        align,
        combine,
        cosmetic: buildCosmetic(),
        dark_optimize: darkOptimize && darks.length > 0,
      }, await getOutputDir(), name);
      const choice = pipelineInitialChoice(res);
      setResult(res);
      setActivePreview(choice);
      if (choice) onShow?.(res, choice, target);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (isCancelledRun(msg)) setCancelled(true);
      else setError(msg);
    } finally {
      setLoading(false);
      timer.stop();
      progress.reset();
    }
  };

  useEffect(() => {
    decodedPreviewsRef.current.clear();
  }, [result]);

  const showChoice = (choice: string) => {
    setActivePreview(choice);
    if (result) onShow?.(result, choice, runTarget);
  };

  useEffect(() => {
    if (activePreview !== PIPELINE_RGB_CHOICE) return;
    if (!result?.rgb_preview || !rgbCanvasRef.current) return;
    const rgbSize = toDims(result.rgb_dimensions);
    if (!rgbSize) return;

    const [w, h] = rgbSize;
    const canvas = rgbCanvasRef.current;
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    let imgData = decodedPreviewsRef.current.get(PIPELINE_RGB_CHOICE);
    if (!imgData) {
      const raw = atob(result.rgb_preview);
      imgData = ctx.createImageData(w, h);
      const px = new Uint32Array(imgData.data.buffer);
      for (let i = 0, j = 0; i < w * h; i++, j += 3) {
        px[i] = (255 << 24) | (raw.charCodeAt(j + 2) << 16) | (raw.charCodeAt(j + 1) << 8) | raw.charCodeAt(j);
      }
      decodedPreviewsRef.current.set(PIPELINE_RGB_CHOICE, imgData);
    }
    ctx.putImageData(imgData, 0, 0);
  }, [result, activePreview]);

  useEffect(() => {
    if (!activePreview || activePreview === PIPELINE_RGB_CHOICE) return;
    const ch = result?.channel_previews.find((c) => c.label === activePreview);
    const canvas = chCanvasRef.current;
    if (!ch || !canvas) return;

    const { width: w, height: h } = ch;
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    let imgData = decodedPreviewsRef.current.get(ch.label);
    if (!imgData) {
      const raw = atob(ch.pixels_b64);
      imgData = ctx.createImageData(w, h);
      const px = new Uint32Array(imgData.data.buffer);
      for (let i = 0, j = 0; i < w * h; i++, j += 2) {
        const v = (raw.charCodeAt(j) | (raw.charCodeAt(j + 1) << 8)) >> 8;
        px[i] = (255 << 24) | (v << 16) | (v << 8) | v;
      }
      decodedPreviewsRef.current.set(ch.label, imgData);
    }
    ctx.putImageData(imgData, 0, 0);
  }, [result, activePreview]);

  const totalLights = culled.groups.reduce((s, c) => s + c.paths.length, 0);
  const smallestChannel = culled.groups
    .filter((c) => c.paths.length > 0)
    .reduce((min, c) => Math.min(min, c.paths.length), Number.POSITIVE_INFINITY);
  const normalizeNote = normalizeNoteText(stackConfig?.normalization, normalize);

  const masters = (result?.masters ?? []).filter((master) => master.fits_path);
  const composeArmed = composeConfirm.armedKey === COMPOSE_CONFIRM_KEY;
  const composeTheseMasters = () => {
    if (!onComposeMasters) return;
    const handOff = () => onComposeMasters(masters);
    if (composeHasProgress) {
      composeConfirm.press(COMPOSE_CONFIRM_KEY, handOff);
      return;
    }
    composeConfirm.blur(COMPOSE_CONFIRM_KEY);
    handOff();
  };

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <SectionHeader icon={ICON} title="Calibration Pipeline" />

      <div className="flex flex-col gap-2">
        {channels.map((ch, i) => {
          const kept = culled.groups[i].paths.length;
          const lost = ch.paths.length - kept;
          const clearKey = `channel:${ch.label}`;
          return (
            <div key={ch.label} data-testid="pipeline-lane" className="flex items-center justify-between rounded border border-zinc-800/50 px-2 py-1.5">
              <div className="flex items-center gap-2">
                <span className="inline-block h-3 w-3 rounded-full" style={{ backgroundColor: LANE_COLORS[ch.label as PipelineLane] }} />
                <span className="text-xs text-zinc-300">{ch.label}</span>
                <span className="text-[10px] text-zinc-500">{kept} files{lost > 0 && ` (${lost} excluded)`}</span>
              </div>
              <div className="flex gap-1">
                <button aria-label={`Add ${ch.label} lights`} onClick={() => addToChannel(ch.label)} className={LANE_BUTTON_CLASS}>+ Add</button>
                {ch.paths.length > 0 ? (
                  <ClearButton
                    armed={clearConfirm.armedKey === clearKey}
                    onPress={() => clearConfirm.press(clearKey, () => clearLane(ch.label))}
                    onBlur={() => clearConfirm.blur(clearKey)}
                  />
                ) : (
                  <button type="button" onClick={() => removeLane(ch.label)} className={LANE_BUTTON_CLASS}>Remove</button>
                )}
              </div>
            </div>
          );
        })}
        {channels.length === 0 && (
          <p className="text-[10px] text-zinc-500 leading-snug">No lanes yet. Add a lane and pick its lights.</p>
        )}
        <p data-testid="pipeline-lane-note" role="status" className="text-[10px] text-amber-400/80 leading-snug empty:hidden">{laneNote ?? ""}</p>
        {leftOutLine && (
          <p data-testid="pipeline-filter-left-out" className="text-[10px] text-zinc-500 leading-snug">{leftOutLine}</p>
        )}
        {leftOutGroups.map((group) => {
          const candidates = laneCandidates(group.key);
          if (candidates.length === 0) return null;
          const lane = putLane(group.key, candidates);
          return (
            <div key={group.key} data-testid="pipeline-put-in-lane" className="flex items-center justify-end gap-1">
              <span className="min-w-0 truncate text-[10px] text-zinc-500">{group.key}</span>
              <select aria-label={`Lane for the ${group.key} lights`} value={lane} onChange={(e) => setPutChoices((prev) => ({ ...prev, [group.key]: e.target.value }))} className="ab-select">
                {candidates.map((label) => <option key={label} value={label}>{label}</option>)}
              </select>
              <button type="button" aria-label={`Put the ${group.key} lights in the ${lane} lane`} onClick={() => putInLane(group, lane)} className={LANE_BUTTON_CLASS}>Put in lane</button>
            </div>
          );
        })}
        {stackedLine && (
          <p data-testid="pipeline-stacked-left-out" className="text-[10px] text-zinc-500 leading-snug">{stackedLine}</p>
        )}
        {laneToAdd && (
          <div className="flex items-center justify-end gap-1">
            <select aria-label="Lane to add" value={laneToAdd} onChange={(e) => setLaneChoice(e.target.value)} className="ab-select">
              {missingLanes.map((label) => <option key={label} value={label}>{label}</option>)}
            </select>
            <button type="button" onClick={addLane} className={LANE_BUTTON_CLASS}>+ Lane</button>
          </div>
        )}
      </div>

      <div className="flex flex-col gap-2 border-t border-zinc-800/50 pt-3">
        <span className="text-xs text-zinc-500 uppercase tracking-wider">Calibration (optional)</span>
        <CalibRow label="Darks" addLabel={`Add ${CALIBRATION_FRAMES.dark}`} count={darks.length} onAdd={() => addCalibration("dark")} clearArmed={clearConfirm.armedKey === "calib:dark"} onClear={() => clearConfirm.press("calib:dark", () => setDarks([]))} onClearBlur={() => clearConfirm.blur("calib:dark")} />
        <CalibRow label="Flats" addLabel={`Add ${CALIBRATION_FRAMES.flat}`} count={flats.length} onAdd={() => addCalibration("flat")} clearArmed={clearConfirm.armedKey === "calib:flat"} onClear={() => clearConfirm.press("calib:flat", () => setFlats([]))} onClearBlur={() => clearConfirm.blur("calib:flat")} />
        <CalibRow label="Flat darks" addLabel={`Add ${CALIBRATION_FRAMES.flatdark}`} testId="pipeline-flatdark-row" count={flatDarks.length} onAdd={() => addCalibration("flatdark")} clearArmed={clearConfirm.armedKey === "calib:flatdark"} onClear={() => clearConfirm.press("calib:flatdark", () => setFlatDarks([]))} onClearBlur={() => clearConfirm.blur("calib:flatdark")} />
        <CalibRow label="Bias" addLabel={`Add ${CALIBRATION_FRAMES.bias}`} count={bias.length} onAdd={() => addCalibration("bias")} clearArmed={clearConfirm.armedKey === "calib:bias"} onClear={() => clearConfirm.press("calib:bias", () => setBias([]))} onClearBlur={() => clearConfirm.blur("calib:bias")} />
        {darks.length > 0 && (
          <>
            <Toggle label="Optimize dark scale per light (minimise noise)" checked={darkOptimize} accent="sky" onChange={setDarkOptimize} />
            {darkOptimize && (
              <p className="text-[10px] text-zinc-500 leading-snug">Each light gets the dark scale that minimises its background noise instead of the exposure ratio.</p>
            )}
          </>
        )}
      </div>

      <div className="flex flex-col gap-2 border-t border-zinc-800/50 pt-3">
        <span className="text-xs text-zinc-500 uppercase tracking-wider">Cosmetic correction</span>
        <Toggle label="Repair hot / cold pixels after dark subtraction" checked={cosmeticEnabled} accent="sky" onChange={setCosmeticEnabled} />
        {cosmeticEnabled && (
          <div className="flex flex-col gap-2 pl-2 border-l border-zinc-800">
            <Toggle label="Use master dark" checked={cosmeticUseDark} accent="sky" onChange={setCosmeticUseDark} />
            {cosmeticUseDark && (
              <>
                <Slider label="Hot sigma" value={cosmeticHotSigma} min={0.5} max={20} step={0.1} accent="sky" format={(v) => v.toFixed(1)} onChange={setCosmeticHotSigma} />
                <Toggle label="Cold pixels" checked={cosmeticColdEnabled} accent="sky" onChange={setCosmeticColdEnabled} />
                {cosmeticColdEnabled && (
                  <Slider label="Cold sigma" value={cosmeticColdSigma} min={0.5} max={20} step={0.1} accent="sky" format={(v) => v.toFixed(1)} onChange={setCosmeticColdSigma} />
                )}
                {darks.length === 0 && (
                  <p className="text-[10px] text-amber-400/80 leading-snug">Add dark frames above: without a master dark this method does nothing.</p>
                )}
              </>
            )}
            <Toggle label="Auto detect hot pixels per light" checked={cosmeticAutoEnabled} accent="sky" onChange={setCosmeticAutoEnabled} />
            {cosmeticAutoEnabled && (
              <Slider label="Auto sigma" value={cosmeticAutoSigma} min={0.5} max={20} step={0.1} accent="sky" format={(v) => v.toFixed(1)} onChange={setCosmeticAutoSigma} />
            )}
            <CosmeticAutoNote enabled={cosmeticAutoEnabled} />
            <label htmlFor={defectListId} className="text-xs text-zinc-400">Defect list</label>
            <textarea
              id={defectListId}
              value={defectText}
              onChange={(e) => setDefectText(e.target.value)}
              placeholder={DEFECT_PLACEHOLDER}
              spellCheck={false}
              disabled={loading}
              className="w-full h-20 resize-y bg-zinc-900/80 border border-zinc-700/50 rounded px-2 py-1 text-[11px] font-mono text-zinc-200 placeholder:text-zinc-600 focus:border-sky-400"
            />
            <div className="text-[10px] text-zinc-500">
              {parsedDefects.defects.length} entr{parsedDefects.defects.length === 1 ? "y" : "ies"}
              {defectErrors.length > 0 && <span className="text-amber-400"> - {defectErrors.length} invalid line{defectErrors.length === 1 ? "" : "s"}</span>}
            </div>
            {defectErrors.slice(0, MAX_DEFECT_ERRORS).map((err) => (
              <div key={err.line} className="text-[10px] text-amber-400/90 font-mono">{formatDefectError(err)}</div>
            ))}
          </div>
        )}
      </div>

      <div className="flex flex-col gap-3 border-t border-zinc-800/50 pt-3">
        <span className="text-xs text-zinc-500 uppercase tracking-wider">Stacking</span>
        <RejectionControls settings={rejectionSettings} onChange={changeRejection} frameCount={totalLights > 0 ? smallestChannel : null} accent="sky" />
        <div className="flex items-center justify-between">
          <label htmlFor={combineId} className="text-xs text-zinc-400">Combine</label>
          <select id={combineId} value={combine} onChange={(e) => { configTouchedRef.current = true; setCombine(e.target.value as CombineMethod); }} className="ab-select">
            {COMBINE_OPTIONS.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
          </select>
        </div>
        <Toggle label="Normalize to sky background (divide)" checked={normalize} accent="sky" onChange={setNormalize} describedBy={normalizeNote ? normalizeNoteId : undefined} />
        {normalizeNote && (
          <p id={normalizeNoteId} data-testid="pipeline-normalize-note" className="text-[10px] text-zinc-500 leading-snug">{normalizeNote}</p>
        )}
        <Toggle label="Align frames before stack" checked={align} accent="sky" onChange={setAlign} />
      </div>

      {exclusionNotice && (
        <div className="flex items-start justify-between gap-2 text-[10px] text-amber-400/90 bg-amber-900/20 border border-amber-800/30 rounded px-2.5 py-1.5">
          <span data-subframe-notice className="break-words">{exclusionNotice}</span>
          <button
            type="button"
            onClick={() => onUseAllFrames?.()}
            className="shrink-0 text-[10px] text-amber-300 hover:text-amber-200 bg-zinc-800 px-2 py-0.5 rounded"
          >Use all frames</button>
        </div>
      )}

      <RunButton
        label={`Run Pipeline (${totalLights} lights)`}
        runningLabel="Running pipeline..."
        running={loading}
        disabled={totalLights === 0 || defectErrors.length > 0}
        accent="sky"
        onClick={handleRun}
      />
      {loading && (
        <div className="flex flex-col gap-1.5 animate-fade-in">
          <div className="w-full h-1.5 bg-zinc-800 rounded-full overflow-hidden">
            <div data-testid="pipeline-progress-bar" className="h-full rounded-full transition-all duration-300" style={{ width: `${progress.percent}%`, background: "linear-gradient(90deg, var(--ab-sky), #7dd3fc)" }} />
          </div>
          <div className="flex justify-between items-center gap-2 text-[10px] text-zinc-500">
            <span data-testid="pipeline-progress-label" className="truncate">{pipelineProgressText(progress.stage)}</span>
            <span className="flex shrink-0 items-center gap-2">
              <span data-testid="pipeline-elapsed" className="font-mono">{timer.formatted}</span>
              <button
                type="button"
                data-testid="pipeline-cancel"
                onClick={() => { cancelProgress(PIPELINE_PROGRESS_EVENT).catch(() => {}); }}
                title="Cancel pipeline"
                aria-label="Cancel pipeline"
                className="text-zinc-500 hover:text-red-400 transition-colors"
              >
                <X size={11} />
              </button>
            </span>
          </div>
        </div>
      )}
      {cancelled && <div className="text-[10px] text-zinc-500">Pipeline cancelled</div>}
      <ErrorAlert message={error} />

      {result && (
        <div className="flex flex-col gap-3 animate-fade-in border-t border-zinc-800/50 pt-3">
          <span className="text-xs font-semibold text-zinc-400">Results</span>
          <div data-testid="pipeline-warnings" className="flex flex-col gap-3 empty:hidden">
            <WarningList warnings={result.warnings} />
          </div>

          <div className="flex gap-1">
            {result.channel_previews.map((ch) => (
              <button
                key={ch.label}
                onClick={() => showChoice(ch.label)}
                className={`ab-pill ${activePreview === ch.label ? "data-active" : ""}`}
                data-active={activePreview === ch.label || undefined}
              >
                {ch.label}
              </button>
            ))}
            {result.rgb_preview && (
              <button
                onClick={() => showChoice(PIPELINE_RGB_CHOICE)}
                className={`ab-pill ${activePreview === PIPELINE_RGB_CHOICE ? "data-active" : ""}`}
                data-active={activePreview === PIPELINE_RGB_CHOICE || undefined}
              >
                {result.rgb_palette ?? PIPELINE_RGB_CHOICE}
              </button>
            )}
          </div>

          {activePreview === PIPELINE_RGB_CHOICE && result.rgb_preview && (
            <canvas ref={rgbCanvasRef} className="w-full rounded border border-zinc-700" style={{ imageRendering: "auto" }} />
          )}
          {activePreview && activePreview !== PIPELINE_RGB_CHOICE && (
            <canvas ref={chCanvasRef} className="w-full rounded border border-zinc-700" style={{ imageRendering: "auto" }} />
          )}

          <div className="flex flex-col gap-1 text-[10px] text-zinc-500">
            {result.stats.channels.map((ch) => (
              <div key={ch.label}>
                {ch.label}: {stackedLights(ch)} lights, mean={ch.mean.toFixed(1)} std={ch.stddev.toFixed(1)}
                {ch.cosmetic_replaced != null && `, ${formatCount(ch.cosmetic_replaced)} px repaired`}
                {ch.dark_scale_mean != null && `, dark scale ${ch.dark_scale_mean.toFixed(3)}`}
                {ch.dark_scale_min != null && ch.dark_scale_max != null && ch.dark_scale_max - ch.dark_scale_min > 1e-3 && ` (${ch.dark_scale_min.toFixed(3)}..${ch.dark_scale_max.toFixed(3)})`}
              </div>
            ))}
            <PipelineMasterLines stats={result.stats} />
            {typeof result.elapsed_ms === "number" && <div>{`Finished in ${formatTime(result.elapsed_ms)}`}</div>}
          </div>

          {masters.length > 0 && (
            <div className="flex flex-wrap gap-2">
              <button type="button" onClick={() => openInFiles(masters.map((master) => master.fits_path))} className={HANDOFF_BUTTON_CLASS}>Open masters</button>
              {onComposeMasters && (
                <button
                  type="button"
                  data-testid="pipeline-compose-masters"
                  aria-live="polite"
                  aria-label={composeArmed ? "Confirm replacing the Compose channels with these masters" : undefined}
                  title={composeArmed ? "Click again to replace the Compose channels with these masters; the Compose step results are discarded" : "Open the masters and put each in its filter's Compose channel"}
                  onClick={composeTheseMasters}
                  onBlur={() => composeConfirm.blur(COMPOSE_CONFIRM_KEY)}
                  className={composeArmed ? ARMED_HANDOFF_BUTTON_CLASS : HANDOFF_BUTTON_CLASS}
                >
                  {composeArmed ? "Replace Compose channels?" : "Compose these masters"}
                </button>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
