import { useState, useRef, useCallback, useEffect, useLayoutEffect, useId, useMemo, memo } from "react";
import { Activity, BarChart3, CircleDot, Crosshair, FileText, Layers } from "lucide-react";
import CubeFrameNav from "../CubeFrameNav";
import LineMeasurementSection from "./LineMeasurementSection";
import LineFitSection from "./LineFitSection";
import OutputNote from "./OutputNote";
import StaleResultNote, { ReopenedProductNote } from "./StaleResultNote";
import LineListControls from "./LineListControls";
import SpectralAxisControls from "./SpectralAxisControls";
import SpectrumComparisonSection from "./SpectrumComparisonSection";
import ExportNotice, { SAVED_NOTICE_MS } from "./ExportNotice";
import { useTimedFlag, useTimedNotice } from "../../hooks/useTimedNotice";
import SpectrumPlot from "./SpectrumPlot";
import SpectrumViewControls from "./SpectrumViewControls";
import RangeEdgeField from "./RangeEdgeField";
import DecimalField from "../ui/DecimalField";
import { ErrorAlert, SMALL_BUTTON_CLASS } from "../ui";
import { scienceDqSummary } from "../../utils/analysisLabels";
import {
  collapseCubeRange,
  computeMomentMaps,
  fitCubeLines,
  getCubeSpectrum,
  getCubeSpectrumRegion,
  inspectLineFitSpaxel,
} from "../../services/cube";
import { getSpectralAxis, measureSpectralLine } from "../../services/spectral";
import { lineEntry, lineFitEntry, measurementLog, spectrumExportEntry } from "../../utils/measurementLog";
import {
  DEFAULT_LINE_FIT_SNR,
  LINE_FIT_LABEL_PREFIX,
  displayedLineFitPlane,
  inspectKey,
  lineFitConfig,
  lineFitCubeResult,
  lineFitFrameNote,
  preferredLineFitPlane,
  type LineFitRun,
} from "../../utils/lineFit";
import { AUTO_WINDOWS_RULE, guardedContinuumWindows } from "../../utils/lineWindows";
import { nativeFluxLabel } from "../../utils/spectrumUnits";
import { getOutputDir } from "../../infrastructure/tauri";
import { useCubeContext, useFileContext, useRenderContext } from "../../context/PreviewContext";
import { setIntegration, useRampIntegration } from "../../hooks/useRampStore";
import {
  formatRampFrameLabel,
  frameForIntegration,
  frameToPosition,
  rampFrameFromPlotChannel,
  rampFrameMarker,
  rampKeyHint,
} from "../../utils/rampLabels";
import { useRegionDoc } from "../../hooks/useRegionStore";
import { useRegionKey } from "../../hooks/useRegionKey";
import {
  beginRegionSpectrum,
  clearRegionSpectrum,
  commitRegionSpectrum,
  failRegionSpectrum,
  useSpectrum,
} from "../../hooks/useSpectrumStore";
import {
  cubeViewKey,
  documentViewKey,
  effectiveFrameChoice,
  effectiveMode,
  spectrumViewStore,
  useSpectrumView,
} from "../../hooks/useSpectrumViewStore";
import type {
  CollapseRangeMode,
  CollapseRangeResult,
  CollapseZeroPolicy,
  ContinuumWindows,
  CubeDims,
  LineFitComponents,
  LineFitPlane,
  LineFitSpaxel,
  MomentConfig,
  MomentKind,
  MomentMapsResult,
  ScienceDqPolicy,
} from "../../shared/types/cube";
import { COLLAPSE_RANGE_MODES, MOMENT_KINDS } from "../../shared/types/cube";
import type { DisplayHint } from "../../shared/types/preview";
import type { Region, RegionShape } from "../../shared/types/regions";
import type {
  CorrectionFrame,
  LineMeasurement,
  LineMeasurementSource,
  LineModel,
  MeasuredLineSource,
  RadialVelocityCorrectionResult,
  SpectralAxisInfo,
  SpectralAxisMode,
  SpectrumSource,
  VelocityConvention,
} from "../../shared/types/spectral";
import {
  PLUS_MINUS,
  lineMeasurementAxis,
  momentVelocityFrameNote,
  runForSource,
  spectrumSourceKey,
  spectrumSourceLabel,
} from "../../utils/lineMeasure";
import type { Spectrum1dSource } from "../../shared/types/spectrum1d";
import { SPECTRUM_FILE_CORRECTION_NOTE, documentSummary, type SpectrumDocument } from "../../utils/spectrumDocument";
import { applyCorrectionKms, formatAxis } from "../../utils/spectralAxis";
import { clearedSetup, setupCarriesTo, setupFromView, setupInstrument, setupIsDefault, setupLabel, viewFromSetup } from "../../utils/spectrumSetup";
import { APP_VERSION } from "../../utils/constants";
import { pixelToWorld } from "../../services/astrometry";
import { shapeCentre } from "../../utils/regionGeometry";
import {
  fileBaseName,
  fluxUnitLabel,
  regionForShape,
  regionForSource,
  saveCsvDialog,
  spectrumCsv,
  spectrumCsvFileName,
  type SpectrumExportInput,
} from "../../utils/spectrumExport";
import {
  bandById,
  displayConvention,
  lineById,
  lineClickHint,
  lineClickSetsRest,
  pickedBandLabel,
  pickedLineLabel,
  redshiftFrameLabel,
  type BandMark,
  type LineFamily,
  type LineMark,
} from "../../utils/lineList";
import {
  FRAME_KEY_HINT,
  channelFromPlotPixel,
  channelRequestNeeded,
  clampFrame,
  displayedChannel,
  filterFrameLabel,
  formatFrameLabel,
  frameAxisValue,
  frameFromKey,
  type FramePublishGate,
} from "../../utils/cubeNavigation";
import { decimalFieldState } from "../../utils/decimalText";
import {
  errPlaneLabel,
  errorBandKind,
  errorBandLegend,
  noiseFactorStaleNote,
  regionErrorLine,
  regionNotesShown,
  regionViewErrors,
} from "../../utils/regionErrors";
import {
  channelWidthAt,
  commitRangeEdge,
  commitWindowEdge,
  continuumOverlapNote,
  formatRangeLabel,
  fullCubeCollapse,
  noSpectralAxisText,
  spectralSectionsVisible,
  windowsAreValid,
  type ChannelRange,
  type FullCollapseMode,
  type PlotMapping,
  type WindowEdge,
  type WindowSide,
} from "../../utils/spectrumRange";
import { SPECTRUM_PLOT_HEIGHT, bandChannelRange, documentPlot, rangeBand, spectrumPlotMapping, spectrumPlotParts } from "../../utils/spectrumPlotModel";

export interface CubeResult {
  label: string;
  previewUrl: string;
  fitsPath: string | null;
  dimensions: [number, number] | null;
  frameIndex?: number;
  displayHint?: DisplayHint;
}

interface SpectroscopyPanelProps {
  spectrum?: number[];
  wavelengths?: number[] | null;
  pixelCoord?: { x: number; y: number } | null;
  isLoading?: boolean;
  cubeDims?: CubeDims | null;
  elapsed?: number;
  error?: string | null;
  filePath?: string;
  onFramePreview?: (previewUrl: string, frameIndex: number, fitsPath?: string) => void;
  onCubeResult?: (result: CubeResult) => void;
  onInvalidateCubeResult?: (fitsPaths: string[]) => void;
  publishGate: FramePublishGate;
  documentMode?: boolean;
  spectrumDocument?: SpectrumDocument | null;
  viewKey?: string;
}

type BrushTarget = "range" | "left" | "right";
type RegionView = "sum" | "mean" | "jy";
type RangeFieldUnit = "channel" | "axis";

const RANGE_FIELD_UNITS: readonly RangeFieldUnit[] = ["channel", "axis"];

interface LineRun {
  key: string;
  measurementAxis: number[];
  result: LineMeasurement;
}

function hasArea(shape: RegionShape): boolean {
  return shape.shape === "circle" || shape.shape === "box" || shape.shape === "ellipse" || shape.shape === "polygon";
}

function pickRegion(regions: Region[], selectedId: string | null): { target: Region | null; background: Region | null } {
  const selected = regions.find((r) => r.id === selectedId) ?? null;
  const target = selected && hasArea(selected.shape) ? selected : (regions.find((r) => r.props.include && hasArea(r.shape)) ?? null);
  if (!target) return { target: null, background: null };
  const linked = target.backgroundId ? (regions.find((r) => r.id === target.backgroundId) ?? null) : null;
  const background = linked && linked.shape.shape === "annulus" ? linked : null;
  return { target, background };
}

function documentErrorOrigin(source: Spectrum1dSource): string {
  if (source.kind === "wcs1d") return "ERR extension";
  if (source.kind === "csv") return "err column";
  return "FLUX_ERROR column";
}

const DOCUMENT_CURSOR_KEY_HINT =
  "Left and Right arrows move the channel cursor, Shift moves ten channels, Home and End go to the ends, Escape clears it.";
const DEFAULT_SNR = 3;
const SNR_FIELD = { min: 0 };
const LABEL_MAPPING_WIDTH = 400;
const MEASURED_SERIES_VIEW: RegionView = "sum";
const LINE_FIT_READY_HINT = "brush a line range and set a rest wavelength";
const LINE_PICK_DOUBLE_CLICK_GUARD_MS = 400;
const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-1.5 py-0.5 text-[10px] font-mono text-zinc-200 focus:border-violet-500/50 w-14 disabled:opacity-40";
const SELECT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-1.5 py-0.5 text-[10px] text-zinc-200 focus:border-violet-500/50 disabled:opacity-40";
const LABEL_CLASS = "text-[9px] text-zinc-500 uppercase";
const SPECTROSCOPY_PRODUCT_PREFIXES: readonly string[] = ["Collapse ", "Moment ", LINE_FIT_LABEL_PREFIX];

interface RangeRequest {
  z0: number;
  z1: number;
  mode: CollapseRangeMode;
}

interface RangeRun {
  result: CollapseRangeResult;
  request: RangeRequest;
}

interface MomentRun {
  result: MomentMapsResult;
  config: MomentConfig;
}

interface MomentConfigInput {
  range: ChannelRange;
  snrText: string;
  windows: ContinuumWindows | null;
  channelCount: number;
  restUm: number | null;
  convention: VelocityConvention;
  maskBelow: boolean;
  dqPolicy: ScienceDqPolicy;
}

function momentConfig(input: MomentConfigInput): MomentConfig | { error: string } {
  const snr = decimalFieldState(input.snrText, SNR_FIELD);
  if (snr.value === null) return { error: `SNR threshold: ${snr.error ?? "Enter a number"}` };
  if (input.windows && !windowsAreValid(input.windows, input.channelCount)) {
    return { error: `continuum windows must be channel ranges inside 0–${Math.max(input.channelCount - 1, 0)}` };
  }
  return {
    z0: input.range.z0,
    z1: input.range.z1,
    rest_um: input.restUm,
    convention: input.convention,
    continuum: input.windows,
    snr_threshold: snr.value,
    mask_below_threshold: input.maskBelow,
    dq_policy: input.dqPolicy,
  };
}

function sameRequest(stored: object, current: object | null): boolean {
  return current !== null && !("error" in current) && JSON.stringify(stored) === JSON.stringify(current);
}

function SpectroscopyPanel({
  spectrum = [],
  wavelengths = null,
  pixelCoord: pixelCoordProp = null,
  isLoading = false,
  cubeDims: cubeDimsProp = null,
  elapsed = 0,
  error = null,
  filePath,
  onFramePreview,
  onCubeResult,
  onInvalidateCubeResult,
  publishGate: framePublishGate,
  documentMode = false,
  spectrumDocument = null,
  viewKey: viewKeyProp,
}: SpectroscopyPanelProps) {
  const pixelCoord = documentMode ? null : pixelCoordProp;
  const cubeDims = documentMode ? null : cubeDimsProp;
  const docSpectrum = documentMode ? (spectrumDocument?.spectrum ?? null) : null;
  const containerRef = useRef<HTMLDivElement>(null);
  const [collapseLoading, setCollapseLoading] = useState(false);
  const [collapseResult, setCollapseResult] = useState<CollapseRangeResult | null>(null);
  const [collapseError, setCollapseError] = useState<string | null>(null);
  const [collapseMode, setCollapseMode] = useState<FullCollapseMode>("mean");
  const [collapseZeroPolicy, setCollapseZeroPolicy] = useState<CollapseZeroPolicy>("include_physical_zeros");
  const regionSeqRef = useRef(0);
  const collapseSeqRef = useRef(0);
  const rangeSeqRef = useRef(0);
  const momentSeqRef = useRef(0);
  const lineSeqRef = useRef(0);
  const lineFitSeqRef = useRef(0);
  const inspectSeqRef = useRef(0);
  const inspectKeyRef = useRef<string | null>(null);
  const previousFileRef = useRef<string | undefined>(filePath);
  const invalidateCubeResultRef = useRef(onInvalidateCubeResult);
  const collapseProductsRef = useRef<string[]>([]);
  const rangeProductsRef = useRef<string[]>([]);
  const momentProductsRef = useRef<string[]>([]);
  const fitProductsRef = useRef<string[]>([]);
  const regionPendingRef = useRef(false);
  const showCubeFocusRef = useRef<HTMLElement | null>(null);
  const holdShowCubeFocus = useCallback((element: HTMLElement | null) => {
    showCubeFocusRef.current = element;
  }, []);

  const spectrumStore = useSpectrum();
  const region = documentMode ? null : spectrumStore.region;
  const regionSource = documentMode ? null : spectrumStore.regionSource;
  const regionContinuum = documentMode ? null : spectrumStore.regionContinuum;
  const regionLoading = !documentMode && spectrumStore.regionLoading;
  const regionError = documentMode ? null : spectrumStore.regionError;
  const { processed } = useRenderContext();
  const { file } = useFileContext();
  const inputKey = JSON.stringify([filePath, documentMode, file?.id, file?.imageRef, file?.reloads, file?.finishedAt]);
  const { ramp: cubeRamp } = useCubeContext();
  const ramp = documentMode ? null : cubeRamp;
  const spectral = spectralSectionsVisible(ramp);
  const rampIntegration = useRampIntegration();
  const lastRampIntegrationRef = useRef(rampIntegration);
  const bunit = documentMode ? (docSpectrum?.flux_unit ?? null) : (file?.result?.header?.BUNIT ?? null);
  const shownFrame = displayedChannel(processed, filePath);
  const regionKey = useRegionKey();
  const regionDoc = useRegionDoc(regionKey);
  const picked = useMemo(() => pickRegion(regionDoc.regions, regionDoc.selectedId), [regionDoc]);
  const [regionView, setRegionView] = useState<RegionView>("sum");
  const [exportBusy, setExportBusy] = useState(false);
  const [savedPath, showSaved, clearSaved] = useTimedNotice<string>(SAVED_NOTICE_MS);
  const [copied, flagCopied, clearCopied] = useTimedFlag(1500);
  const [exportError, setExportError] = useState<string | null>(null);
  const exportSeqRef = useRef(0);

  const viewKey =
    viewKeyProp ??
    (documentMode ? documentViewKey(filePath ?? "", String(spectrumDocument?.source.hdu ?? "")) : cubeViewKey(filePath ?? ""));
  const [view, patchView] = useSpectrumView(viewKey);
  const docPlot = useMemo(() => (docSpectrum ? documentPlot(docSpectrum, view) : null), [docSpectrum, view]);
  const docAxis = docPlot?.axis ?? null;
  const setupVacuumUm = useMemo(() => {
    if (docSpectrum === null || docAxis === null || (docAxis.kind !== "wave" && docAxis.kind !== "awav" && docAxis.kind !== "freq")) return null;
    const values = formatAxis(docAxis, "wavelength_vac", null, view.convention)?.values;
    return values !== undefined && values.length === docSpectrum.flux.length ? values : null;
  }, [docSpectrum, docAxis, view.convention]);
  const [setupFrom, setSetupFrom] = useState<string | null>(() => spectrumViewStore.seededFrom(viewKey));
  const seedInputsRef = useRef({ view, patchView, setupVacuumUm });
  seedInputsRef.current = { view, patchView, setupVacuumUm };
  useEffect(() => {
    if (!docSpectrum) return;
    const seed = seedInputsRef.current;
    setSetupFrom(spectrumViewStore.seededFrom(viewKey));
    const last = spectrumViewStore.lastSetup();
    if (last === null || last.fromKey === viewKey || !setupCarriesTo(last, docSpectrum)) return;
    if (!setupIsDefault(seed.view) || spectrumViewStore.setupDismissed(viewKey)) return;
    const partial = viewFromSetup(last, seed.setupVacuumUm);
    seed.patchView(partial);
    if (setupIsDefault({ ...seed.view, ...partial })) return;
    spectrumViewStore.markSeeded(viewKey, last.fromLabel);
    setSetupFrom(last.fromLabel);
  }, [viewKey, docSpectrum]);
  useEffect(() => {
    if (docSpectrum && !setupIsDefault(view)) spectrumViewStore.rememberSetup(setupFromView(viewKey, setupLabel(docSpectrum), view, setupVacuumUm, setupInstrument(docSpectrum)));
  }, [view, viewKey, setupVacuumUm, docSpectrum]);
  const frameChoice = effectiveFrameChoice(view, docSpectrum);
  const mode = effectiveMode(view, docSpectrum);
  const { restUm, convention, redshift, lineModel, useDq } = view;
  const headerAxis = documentMode ? (docPlot?.axis ?? null) : (cubeDims?.spectral_axis ?? null);
  const [headerFallbackAxis, setHeaderFallbackAxis] = useState<SpectralAxisInfo | null>(null);
  const [axisError, setAxisError] = useState<string | null>(null);
  const axis = headerAxis ?? headerFallbackAxis;
  const changeMode = useCallback((next: SpectralAxisMode) => patchView({ mode: next }), [patchView]);
  const changeRestUm = useCallback((next: number | null) => patchView({ restUm: next }), [patchView]);
  const changeConvention = useCallback((next: VelocityConvention) => patchView({ convention: next }), [patchView]);
  const [correction, setCorrection] = useState<CorrectionFrame>("none");
  const [correctionResult, setCorrectionResult] = useState<RadialVelocityCorrectionResult | null>(null);
  const changeLineListVisible = useCallback((next: boolean) => patchView({ lineListVisible: next }), [patchView]);
  const changeRedshift = useCallback((next: number) => patchView({ redshift: next }), [patchView]);
  const changeLineFamilies = useCallback((next: LineFamily[]) => patchView({ lineFamilies: next }), [patchView]);
  const changeBandSpecies = useCallback((next: string[]) => patchView({ bandSpecies: next }), [patchView]);
  const lastLinePickRef = useRef(0);

  const brushTargetId = useId();
  const snrId = useId();
  const [brushTarget, setBrushTarget] = useState<BrushTarget>("range");
  const activeBrush: BrushTarget = spectral ? brushTarget : "range";
  const [fieldUnit, setFieldUnit] = useState<RangeFieldUnit>("axis");
  const [rangeMode, setRangeMode] = useState<CollapseRangeMode>("sum");
  const [rangeLoading, setRangeLoading] = useState(false);
  const [rangeError, setRangeError] = useState<string | null>(null);
  const [rangeRun, setRangeRun] = useState<RangeRun | null>(null);
  const rangeResult = rangeRun?.result ?? null;
  const [snrText, setSnrText] = useState(String(DEFAULT_SNR));
  const [maskBelow, setMaskBelow] = useState(true);
  const [momentLoading, setMomentLoading] = useState(false);
  const [momentError, setMomentError] = useState<string | null>(null);
  const [momentRun, setMomentRun] = useState<MomentRun | null>(null);
  const momentResult = momentRun?.result ?? null;
  const [momentKind, setMomentKind] = useState<MomentKind>("m0");
  const changeLineModel = useCallback((next: LineModel) => patchView({ lineModel: next }), [patchView]);
  const [lineResult, setLineResult] = useState<LineRun | null>(null);
  const [lineLoading, setLineLoading] = useState(false);
  const [lineError, setLineError] = useState<string | null>(null);
  const [useErr, setUseErr] = useState(true);
  const changeUseDq = useCallback((next: boolean) => patchView({ useDq: next }), [patchView]);
  const scienceDqPolicy: ScienceDqPolicy = useDq ? "do_not_use" : "ignore";
  const [emissionOnly, setEmissionOnly] = useState(true);
  const [lineFitSnrText, setLineFitSnrText] = useState(String(DEFAULT_LINE_FIT_SNR));
  const [resolvingPowerText, setResolvingPowerText] = useState("");
  const [lineFitComponents, setLineFitComponents] = useState<LineFitComponents>("one");
  const [lineFitLoading, setLineFitLoading] = useState(false);
  const [lineFitError, setLineFitError] = useState<string | null>(null);
  const [lineFitRun, setLineFitRun] = useState<LineFitRun | null>(null);
  const [inspect, setInspect] = useState<LineFitSpaxel | null>(null);
  const [inspectLoading, setInspectLoading] = useState(false);
  const [inspectError, setInspectError] = useState<string | null>(null);
  const [frame, setFrame] = useState(() => shownFrame ?? 0);
  const [frameFile, setFrameFile] = useState(filePath);
  const [seenRecord, setSeenRecord] = useState(processed);
  const [requestSeq, setRequestSeq] = useState(0);
  const [loopPlayback, setLoopPlayback] = useState(false);
  if (frameFile !== filePath) {
    setFrameFile(filePath);
    setFrame(shownFrame ?? 0);
    setSeenRecord(processed);
  } else if (seenRecord !== processed) {
    setSeenRecord(processed);
    if (processed === null) setFrame(0);
  }

  useEffect(() => {
    if (processed === null) framePublishGate.supersede();
  }, [processed, framePublishGate]);

  useEffect(() => {
    regionSeqRef.current++;
    collapseSeqRef.current++;
    rangeSeqRef.current++;
    momentSeqRef.current++;
    lineSeqRef.current++;
    lineFitSeqRef.current++;
    inspectSeqRef.current++;
    inspectKeyRef.current = null;
    setMomentRun(null);
    setRangeError(null);
    setRangeRun(null);
    setMomentError(null);
    setCollapseResult(null);
    setCollapseError(null);
    setCollapseLoading(false);
    setRangeLoading(false);
    setMomentLoading(false);
    setLineResult(null);
    setLineError(null);
    setLineLoading(false);
    setLineFitRun(null);
    setLineFitError(null);
    setLineFitLoading(false);
    setInspect(null);
    setInspectLoading(false);
    setInspectError(null);
    if (previousFileRef.current !== filePath) {
      previousFileRef.current = filePath;
      setRegionView("sum");
      setCorrectionResult(null);
      clearRegionSpectrum();
    }
  }, [filePath]);

  useEffect(() => {
    setHeaderFallbackAxis(null);
    setAxisError(null);
    if (!filePath || headerAxis || documentMode) return;
    let cancelled = false;
    getSpectralAxis(filePath)
      .then((info) => {
        if (!cancelled) setHeaderFallbackAxis(info);
      })
      .catch((e: unknown) => {
        if (!cancelled) setAxisError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [filePath, headerAxis, documentMode]);

  const series = useMemo<number[]>(() => {
    if (documentMode) return docSpectrum?.flux ?? [];
    if (region) {
      if (regionView === "jy" && region.flux_jy) return region.flux_jy;
      return regionView === "mean" ? region.mean : region.sum;
    }
    return spectrum;
  }, [documentMode, docSpectrum, region, regionView, spectrum]);
  const shownRegionView: RegionView = region && regionView === "jy" && !region.flux_jy ? "sum" : regionView;
  const bandErrors = useMemo(() => (region ? regionViewErrors(region, shownRegionView) : null), [region, shownRegionView]);
  const seriesErr = docSpectrum?.err ?? bandErrors?.err ?? null;
  const exportView: RegionView = region ? shownRegionView : "sum";
  const n = series.length;
  const totalFrames = cubeDims ? (cubeDims.frames ?? 0) : 0;
  const channelCount = totalFrames > 0 ? totalFrames : n;
  const range = view.range !== null && view.range.z1 < channelCount ? view.range : null;
  const windows = windowsAreValid(view.windows, channelCount) ? view.windows : null;
  useEffect(() => {
    if (channelCount > 0 && (range !== view.range || windows !== view.windows)) patchView({ range, windows });
  }, [channelCount, range, windows, view.range, view.windows, patchView]);
  const continuumToSend = windowsAreValid(windows, channelCount) ? windows : null;
  const frameLinked = totalFrames > 1;
  const rangeRequest = useMemo<RangeRequest | null>(() => (range ? { z0: range.z0, z1: range.z1, mode: rangeMode } : null), [range, rangeMode]);
  const momentRequest = useMemo(
    () => (range ? momentConfig({ range, snrText, windows, channelCount, restUm, convention, maskBelow, dqPolicy: scienceDqPolicy }) : null),
    [range, snrText, windows, channelCount, restUm, convention, maskBelow, scienceDqPolicy],
  );
  const fitRequest = useMemo(
    () =>
      range
        ? lineFitConfig({
            range,
            windows,
            channelCount,
            restUm,
            convention,
            snrText: lineFitSnrText,
            emissionOnly,
            useErr,
            useDq,
            resolvingPowerText,
            components: lineFitComponents,
            redshift,
          })
        : null,
    [range, windows, channelCount, restUm, convention, lineFitSnrText, emissionOnly, useErr, useDq, resolvingPowerText, lineFitComponents, redshift],
  );
  const rangeStale = rangeRun !== null && !sameRequest(rangeRun.request, rangeRequest);
  const momentStale = momentRun !== null && !sameRequest(momentRun.config, momentRequest);
  const fitStale = lineFitRun !== null && !sameRequest(lineFitRun.config, fitRequest);
  const ownedProducts = [
    collapseResult?.fits_path,
    rangeResult?.fits_path,
    ...(momentResult ? MOMENT_KINDS.map((kind) => momentResult[kind].fits_path) : []),
    ...(lineFitRun ? Object.values(lineFitRun.result.planes).map((plane) => plane?.fits_path) : []),
  ];
  const calculating = collapseLoading || rangeLoading || momentLoading || lineFitLoading;
  const reopened =
    !documentMode &&
    !calculating &&
    processed !== null &&
    processed.kind === "cube" &&
    processed.inputPath === filePath &&
    processed.frameIndex === undefined &&
    processed.fitsPath !== null &&
    !ownedProducts.includes(processed.fitsPath) &&
    SPECTROSCOPY_PRODUCT_PREFIXES.some((prefix) => processed.label.startsWith(prefix))
      ? { label: processed.label, path: processed.fitsPath }
      : null;
  const reopenedNote = reopened && (
    <ReopenedProductNote
      label={reopened.label}
      onShowCube={onInvalidateCubeResult ? () => invalidateCubeResultRef.current?.([reopened.path]) : undefined}
      focusAfterShowCube={showCubeFocusRef}
    />
  );

  const photometricPlanes = cubeDims?.photometric_planes ?? null;
  const navLabelFor = useMemo(() => {
    if (ramp) return (idx: number) => formatRampFrameLabel(idx, ramp);
    if (!photometricPlanes?.length) return undefined;
    return (idx: number) => filterFrameLabel(idx, totalFrames, photometricPlanes) ?? formatFrameLabel(idx, totalFrames, null, "");
  }, [ramp, photometricPlanes, totalFrames]);

  const shownPngOnly = processed !== null && processed.fitsPath === null;
  const requestFrame = useCallback(
    (idx: number) => {
      const target = clampFrame(idx, totalFrames);
      if (!channelRequestNeeded(target, frame, shownFrame, shownPngOnly)) return;
      framePublishGate.supersede();
      setFrame(target);
      setRequestSeq((seq) => seq + 1);
    },
    [totalFrames, frame, shownFrame, shownPngOnly, framePublishGate],
  );

  useEffect(() => {
    if (!ramp || ramp.nints <= 1 || lastRampIntegrationRef.current === rampIntegration) return;
    lastRampIntegrationRef.current = rampIntegration;
    requestFrame(frameForIntegration(frame, rampIntegration, ramp.ngroups));
  }, [ramp, rampIntegration, frame, requestFrame]);

  useEffect(() => {
    if (!ramp || ramp.nints <= 1) return;
    const shownIntegration = frameToPosition(frame, ramp.ngroups).integration;
    lastRampIntegrationRef.current = shownIntegration;
    setIntegration(shownIntegration);
  }, [ramp, frame]);

  const publishCubeResult = useCallback(
    (result: CubeResult) => {
      framePublishGate.supersede();
      onCubeResult?.(result);
    },
    [framePublishGate, onCubeResult],
  );

  const currentSource = useMemo<SpectrumSource | null>(() => {
    if (region) return regionSource;
    return pixelCoord ? { kind: "pixel", x: pixelCoord.x, y: pixelCoord.y } : null;
  }, [region, regionSource, pixelCoord]);
  const measureSource = useMemo<LineMeasurementSource | null>(
    () => (documentMode ? (spectrumDocument?.source ?? null) : currentSource),
    [documentMode, spectrumDocument, currentSource],
  );
  const lineSource = useMemo<MeasuredLineSource | null>(() => {
    const fileSource = docSpectrum?.source;
    if (fileSource?.kind !== "x1dints") return measureSource;
    return { kind: "series", paths: fileSource.paths, detector: fileSource.detector, sporder: fileSource.sporder, pick: fileSource.pick };
  }, [docSpectrum, measureSource]);
  const fluxUnit = fluxUnitLabel(bunit, exportView, measureSource?.kind ?? "pixel");
  const lineShiftKms = useMemo(
    () => (correction !== "none" && correctionResult ? applyCorrectionKms([0], correctionResult, correction)[0] : null),
    [correction, correctionResult],
  );
  const plotParts = useMemo(
    () =>
      docPlot ??
      spectrumPlotParts({
        axis,
        mode,
        view,
        correction,
        correctionResult,
        shiftKms: lineShiftKms,
        series,
        dqExcluded: null,
        errors: bandErrors,
        errorsWidenRange: !documentMode,
        readoutErr: seriesErr,
        fluxUnit,
        color: region ? "#22d3ee" : "#a855f7",
        fallbackWavelengths: wavelengths,
        fallbackUnit: cubeDims?.spectral_classification?.axis_unit,
      }),
    [docPlot, axis, mode, view, correction, correctionResult, lineShiftKms, series, bandErrors, documentMode, seriesErr, fluxUnit, region, wavelengths, cubeDims],
  );
  const plotModel = plotParts.model;
  const frameLabel = useMemo(() => {
    if (!frameLinked) return null;
    if (ramp) return formatRampFrameLabel(frame, ramp);
    return (
      filterFrameLabel(frame, totalFrames, photometricPlanes) ??
      formatFrameLabel(
        frame,
        totalFrames,
        frameAxisValue(frame, plotModel.xValues),
        plotModel.xUnit,
        plotModel.xValues ? channelWidthAt(plotModel.xValues, frame) : undefined,
      )
    );
  }, [frameLinked, frame, totalFrames, plotModel, ramp, photometricPlanes]);
  const lineOverlayVisible = runForSource(lineResult, lineSource, region ? regionView : MEASURED_SERIES_VIEW) !== null;
  const visibleLineResult = runForSource(lineResult, lineSource, MEASURED_SERIES_VIEW)?.result ?? null;
  const measureSourceKey = measureSource ? spectrumSourceKey(measureSource, MEASURED_SERIES_VIEW) : null;
  const lineSourceKey = lineSource ? spectrumSourceKey(lineSource, MEASURED_SERIES_VIEW) : null;
  const continuumKey = JSON.stringify(windows);
  const pickedRegionKey = JSON.stringify([picked.target?.shape, picked.background?.shape]);
  const previousRegionKeyRef = useRef(pickedRegionKey);
  const previousRegionInputRef = useRef(inputKey);

  useLayoutEffect(() => {
    invalidateCubeResultRef.current = onInvalidateCubeResult;
  });

  useLayoutEffect(() => {
    const collapseSeq = collapseSeqRef;
    collapseSeqRef.current++;
    if (collapseProductsRef.current.length) invalidateCubeResultRef.current?.(collapseProductsRef.current);
    collapseProductsRef.current = [];
    setCollapseResult(null);
    setCollapseError(null);
    setCollapseLoading(false);
    return () => { collapseSeq.current++; };
  }, [inputKey, totalFrames, collapseZeroPolicy, useDq]);

  useLayoutEffect(() => {
    const rangeSeq = rangeSeqRef;
    rangeSeqRef.current++;
    if (rangeProductsRef.current.length) invalidateCubeResultRef.current?.(rangeProductsRef.current);
    rangeProductsRef.current = [];
    setRangeRun(null);
    setRangeError(null);
    setRangeLoading(false);
    return () => { rangeSeq.current++; };
  }, [inputKey, collapseZeroPolicy, useDq]);

  useLayoutEffect(() => {
    rangeSeqRef.current++;
    setRangeError(null);
    setRangeLoading(false);
  }, [range?.z0, range?.z1, rangeMode]);

  useLayoutEffect(() => {
    const momentSeq = momentSeqRef;
    momentSeqRef.current++;
    if (momentProductsRef.current.length) invalidateCubeResultRef.current?.(momentProductsRef.current);
    momentProductsRef.current = [];
    setMomentRun(null);
    setMomentError(null);
    setMomentLoading(false);
    return () => { momentSeq.current++; };
  }, [inputKey, useDq]);

  useLayoutEffect(() => {
    momentSeqRef.current++;
    setMomentError(null);
    setMomentLoading(false);
  }, [range?.z0, range?.z1, continuumKey, restUm, convention, snrText, maskBelow]);

  useLayoutEffect(() => {
    const lineSeq = lineSeqRef;
    lineSeqRef.current++;
    setLineResult(null);
    setLineError(null);
    setLineLoading(false);
    return () => { lineSeq.current++; };
  }, [inputKey, docSpectrum, lineSourceKey, range?.z0, range?.z1, continuumKey, restUm, redshift, convention, lineModel, lineShiftKms, frameChoice, useErr, useDq, plotParts.pickedMarkId]);

  useLayoutEffect(() => {
    const lineFitSeq = lineFitSeqRef;
    const inspectSeq = inspectSeqRef;
    lineFitSeqRef.current++;
    if (fitProductsRef.current.length) invalidateCubeResultRef.current?.(fitProductsRef.current);
    fitProductsRef.current = [];
    inspectSeqRef.current++;
    inspectKeyRef.current = null;
    setLineFitRun(null);
    setLineFitError(null);
    setLineFitLoading(false);
    setInspect(null);
    setInspectError(null);
    setInspectLoading(false);
    return () => {
      lineFitSeq.current++;
      inspectSeq.current++;
    };
  }, [inputKey, useDq]);

  useLayoutEffect(() => {
    lineFitSeqRef.current++;
    setLineFitError(null);
    setLineFitLoading(false);
  }, [range?.z0, range?.z1, continuumKey, restUm, convention, lineFitSnrText, emissionOnly, useErr, resolvingPowerText, lineFitComponents, redshift]);

  useLayoutEffect(() => {
    const regionSeq = regionSeqRef;
    regionSeqRef.current++;
    if (previousRegionKeyRef.current !== pickedRegionKey || previousRegionInputRef.current !== inputKey) clearRegionSpectrum();
    previousRegionKeyRef.current = pickedRegionKey;
    previousRegionInputRef.current = inputKey;
    return () => {
      regionSeq.current++;
      if (regionPendingRef.current) clearRegionSpectrum();
      regionPendingRef.current = false;
    };
  }, [inputKey, pickedRegionKey]);

  useLayoutEffect(() => {
    const exportSeq = exportSeqRef;
    exportSeqRef.current++;
    setExportBusy(false);
    clearSaved();
    clearCopied();
    setExportError(null);
    return () => {
      exportSeq.current++;
    };
  }, [inputKey, measureSourceKey, clearSaved, clearCopied]);

  const lineConvention = displayConvention(axis, convention);
  const pickedShown = plotParts.pickedMarkId === null ? null : lineById(plotParts.pickedMarkId);
  const lineFrameLabel = redshiftFrameLabel(correction, correctionResult?.method ?? null, axis?.specsys ?? null);

  const lineOverlay = useMemo(
    () => (lineResult && lineOverlayVisible ? { result: lineResult.result, measurementAxis: lineResult.measurementAxis } : null),
    [lineResult, lineOverlayVisible],
  );
  useEffect(() => {
    spectrumViewStore.setOverlay(viewKey, lineOverlay);
    return () => spectrumViewStore.setOverlay(viewKey, null);
  }, [viewKey, lineOverlay]);
  const fitOverlay =
    inspect &&
    lineFitRun &&
    pixelCoord &&
    !region &&
    inspect.x === pixelCoord.x &&
    inspect.y === pixelCoord.y &&
    displayedLineFitPlane(processed, lineFitRun.result) !== null
      ? inspect
      : null;
  const frameMarker = useMemo(() => (frameLinked ? rampFrameMarker(shownFrame, n, ramp) : null), [frameLinked, shownFrame, n, ramp]);
  const referenceChannel = frameLinked ? (ramp ? (frameMarker?.channel ?? null) : shownFrame) : null;
  const focusPlot = useCallback(() => containerRef.current?.focus({ preventScroll: true }), []);

  const handlePlotDoubleClick = useCallback(
    (px: number, mapping: PlotMapping) => {
      if (!frameLinked) return;
      if (performance.now() - lastLinePickRef.current < LINE_PICK_DOUBLE_CLICK_GUARD_MS) return;
      const channel = channelFromPlotPixel(px, mapping, totalFrames);
      if (channel !== null) requestFrame(rampFrameFromPlotChannel(channel, n, ramp, rampIntegration));
    },
    [frameLinked, totalFrames, requestFrame, n, ramp, rampIntegration],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      if (!frameLinked) return;
      const next = frameFromKey(e.key, e.shiftKey, frame, totalFrames);
      if (next === null) return;
      e.preventDefault();
      requestFrame(next);
    },
    [frameLinked, frame, totalFrames, requestFrame],
  );

  const commitBrush = useCallback(
    (selected: ChannelRange) => {
      if (activeBrush === "range") {
        patchView({ range: selected });
        return;
      }
      const window: [number, number] = [selected.z0, selected.z1];
      const other: [number, number] = windows ? (activeBrush === "left" ? windows[1] : windows[0]) : window;
      patchView({ windows: activeBrush === "left" ? [window, other] : [other, window] });
    },
    [activeBrush, windows, patchView],
  );

  const handlePlotClick = useCallback(
    (mark: LineMark) => {
      if (!lineClickSetsRest(axis)) return;
      lastLinePickRef.current = performance.now();
      patchView({ restUm: mark.restVacuumUm, pickedLineId: mark.id });
    },
    [axis, patchView],
  );

  const handleBandPick = useCallback(
    (band: BandMark) => {
      const bandRange = bandChannelRange(band, plotModel);
      if (bandRange) patchView({ range: bandRange });
    },
    [plotModel, patchView],
  );

  const handleCollapse = useCallback(
    async (mode: FullCollapseMode) => {
      const request = fullCubeCollapse(totalFrames, mode);
      if (!filePath || !request) return;
      const seq = ++collapseSeqRef.current;
      const publication = framePublishGate.reserve();
      setCollapseLoading(true);
      setCollapseMode(mode);
      setCollapseResult(null);
      setCollapseError(null);
      try {
        const dir = await getOutputDir();
        const result = await collapseCubeRange(filePath, dir, request.z0, request.z1, request.mode, collapseZeroPolicy, scienceDqPolicy);
        if (collapseSeqRef.current !== seq) return;
        collapseProductsRef.current = result.fits_path ? [result.fits_path] : [];
        setCollapseResult(result);
        if (result.previewUrl && framePublishGate.isLatest(publication)) {
          publishCubeResult({
            label: mode === "median" ? "Collapse median" : "Collapse mean",
            previewUrl: result.previewUrl,
            fitsPath: result.fits_path || null,
            dimensions: result.dimensions ?? null,
          });
        }
      } catch (e) {
        if (collapseSeqRef.current === seq) setCollapseError(e instanceof Error ? e.message : String(e));
      } finally {
        if (collapseSeqRef.current === seq) setCollapseLoading(false);
      }
    },
    [filePath, totalFrames, collapseZeroPolicy, scienceDqPolicy, publishCubeResult, framePublishGate],
  );

  const handleRegionSpectrum = useCallback(async () => {
    if (!filePath || !picked.target) return;
    const seq = ++regionSeqRef.current;
    const source: SpectrumSource = { kind: "region", shape: picked.target.shape, background: picked.background?.shape ?? null };
    const request = beginRegionSpectrum();
    regionPendingRef.current = true;
    try {
      const result = await getCubeSpectrumRegion(filePath, source.shape, source.background, continuumToSend);
      if (regionSeqRef.current !== seq) return;
      commitRegionSpectrum(result, source, continuumToSend, request);
      setRegionView((view) => (view === "jy" && !result.flux_jy ? "sum" : view));
    } catch (e) {
      if (regionSeqRef.current === seq) failRegionSpectrum(e instanceof Error ? e.message : String(e), request);
    } finally {
      if (regionSeqRef.current === seq) regionPendingRef.current = false;
    }
  }, [filePath, picked, continuumToSend]);

  const handleExportSpectrum = useCallback(
    async (destination: "copy" | "save") => {
      if (!filePath || !measureSource || n === 0) return;
      const seq = ++exportSeqRef.current;
      clearSaved();
      clearCopied();
      setExportError(null);
      setExportBusy(true);
      try {
        let pixelFluxJy: number[] | null = null;
        let pixelFluxJyError: string | null = null;
        if (measureSource.kind === "pixel") {
          try {
            pixelFluxJy = (await getCubeSpectrum(filePath, measureSource.x, measureSource.y)).flux_jy ?? null;
          } catch (e) {
            pixelFluxJyError = e instanceof Error ? e.message : String(e);
          }
        }
        let sky: { ra: number; dec: number } | null = null;
        if (measureSource.kind !== "file") {
          const centre = measureSource.kind === "pixel" ? { x: measureSource.x, y: measureSource.y } : shapeCentre(measureSource.shape);
          try {
            const point = (await pixelToWorld(filePath, [[centre.x, centre.y]], "icrs")).points[0] ?? null;
            if (point) sky = { ra: point[0], dec: point[1] };
          } catch {
            sky = null;
          }
        }
        if (exportSeqRef.current !== seq) return;
        const match = measureSource.kind === "file" ? null : regionForSource(regionDoc.regions, measureSource);
        const backgroundId =
          measureSource.kind === "region" && measureSource.background
            ? (regionForShape(regionDoc.regions, measureSource.background)?.id ?? null)
            : null;
        const exportInput: SpectrumExportInput = {
          fileName: fileBaseName(filePath),
          axis,
          mode,
          restUm,
          convention,
          correction,
          correctionResult,
          source: measureSource,
          regionId: match?.id ?? null,
          regionText: match?.props.text ?? null,
          backgroundId,
          view: exportView,
          bunit,
          values: series,
          region,
          pixelFluxJy,
          pixelFluxJyError,
          sky,
          exportedAtUtc: new Date().toISOString(),
          fileSpectrum: docSpectrum,
          software: `AstroBurst ${APP_VERSION}`,
          display: docSpectrum
            ? { wavelengthUnit: view.wavelengthUnit, fluxUnit: view.fluxUnit, binFactor: view.binFactor, dqExcluded: plotModel.dqExcluded }
            : { wavelengthUnit: view.wavelengthUnit, fluxUnit: "native", binFactor: 1, dqExcluded: null },
        };
        const csv = spectrumCsv(exportInput);
        if (destination === "copy") {
          await navigator.clipboard.writeText(csv);
          measurementLog.append(spectrumExportEntry(filePath, exportInput, null));
          flagCopied();
        } else {
          const saved = await saveCsvDialog(csv, spectrumCsvFileName(filePath, measureSource, exportInput.display ?? null), "Save spectrum CSV");
          if (saved) showSaved(saved);
          if (saved) measurementLog.append(spectrumExportEntry(filePath, exportInput, saved));
        }
      } catch (e) {
        if (exportSeqRef.current === seq) setExportError(`export failed: ${e instanceof Error ? e.message : String(e)}`);
      } finally {
        if (exportSeqRef.current === seq) setExportBusy(false);
      }
    },
    [
      filePath,
      measureSource,
      n,
      regionDoc.regions,
      axis,
      mode,
      restUm,
      convention,
      correction,
      correctionResult,
      region,
      exportView,
      bunit,
      series,
      docSpectrum,
      view.wavelengthUnit,
      view.fluxUnit,
      view.binFactor,
      plotModel.dqExcluded,
      clearSaved,
      clearCopied,
      flagCopied,
      showSaved,
    ],
  );

  const handleCollapseRange = useCallback(async () => {
    if (!filePath || !rangeRequest) return;
    const request = rangeRequest;
    const seq = ++rangeSeqRef.current;
    const publication = framePublishGate.reserve();
    setRangeLoading(true);
    setRangeError(null);
    try {
      const dir = await getOutputDir();
      const result = await collapseCubeRange(filePath, dir, request.z0, request.z1, request.mode, collapseZeroPolicy, scienceDqPolicy);
      if (rangeSeqRef.current !== seq) return;
      const products = result.fits_path ? [result.fits_path] : [];
      const replaced = rangeProductsRef.current.filter((path) => !products.includes(path));
      rangeProductsRef.current = products;
      setRangeRun({ result, request });
      if (result.previewUrl && framePublishGate.isLatest(publication)) {
        publishCubeResult({
          label: `Collapse ${result.mode} · ch ${result.z0}-${result.z1}`,
          previewUrl: result.previewUrl,
          fitsPath: result.fits_path || null,
          dimensions: result.dimensions ?? null,
        });
      }
      if (replaced.length) invalidateCubeResultRef.current?.(replaced);
    } catch (e) {
      if (rangeSeqRef.current === seq) setRangeError(e instanceof Error ? e.message : String(e));
    } finally {
      if (rangeSeqRef.current === seq) setRangeLoading(false);
    }
  }, [filePath, rangeRequest, collapseZeroPolicy, scienceDqPolicy, publishCubeResult, framePublishGate]);

  const showMoment = useCallback(
    (result: MomentMapsResult, kind: MomentKind) => {
      setMomentKind(kind);
      const map = result[kind];
      if (map.previewUrl) {
        publishCubeResult({
          label: `Moment ${kind} · ${result.n_channels} ch`,
          previewUrl: map.previewUrl,
          fitsPath: map.fits_path || null,
          dimensions: result.dimensions ?? null,
        });
      }
    },
    [publishCubeResult],
  );

  const handleMoments = useCallback(async () => {
    if (!filePath || !momentRequest) return;
    if ("error" in momentRequest) {
      setMomentError(momentRequest.error);
      return;
    }
    const config = momentRequest;
    const seq = ++momentSeqRef.current;
    const publication = framePublishGate.reserve();
    setMomentLoading(true);
    setMomentError(null);
    try {
      const dir = await getOutputDir();
      const result = await computeMomentMaps(filePath, dir, config);
      if (momentSeqRef.current !== seq) return;
      const products = MOMENT_KINDS.map((kind) => result[kind].fits_path).filter(Boolean);
      const replaced = momentProductsRef.current.filter((path) => !products.includes(path));
      momentProductsRef.current = products;
      setMomentRun({ result, config });
      if (framePublishGate.isLatest(publication)) showMoment(result, "m0");
      if (replaced.length) invalidateCubeResultRef.current?.(replaced);
    } catch (e) {
      if (momentSeqRef.current === seq) setMomentError(e instanceof Error ? e.message : String(e));
    } finally {
      if (momentSeqRef.current === seq) setMomentLoading(false);
    }
  }, [filePath, momentRequest, showMoment, framePublishGate]);

  const handleMeasureLine = useCallback(async () => {
    if (!filePath || !range || !lineSource) return;
    if (windows && !windowsAreValid(windows, channelCount)) {
      setLineError(`continuum windows must be channel ranges inside 0–${Math.max(channelCount - 1, 0)}`);
      return;
    }
    const measurementAxis = lineMeasurementAxis(axis, documentMode);
    if (!measurementAxis) {
      setLineError(`${noSpectralAxisText(documentMode)}: the line measurement needs a wavelength, frequency or velocity axis`);
      return;
    }
    const seq = ++lineSeqRef.current;
    setLineLoading(true);
    setLineError(null);
    setLineResult(null);
    try {
      const documentOptions = documentMode ? { useErr, useDq, wavelengthFrame: frameChoice } : {};
      const result = await measureSpectralLine(filePath, lineSource, {
        z0: range.z0,
        z1: range.z1,
        continuum: windows,
        restUm,
        convention,
        model: lineModel,
        velocityShiftKms: lineShiftKms,
        redshift,
        lineId: pickedShown?.id ?? null,
        lineLabel: pickedShown?.label ?? null,
        ...documentOptions,
      });
      if (lineSeqRef.current !== seq) return;
      setLineResult({ key: spectrumSourceKey(lineSource, MEASURED_SERIES_VIEW), measurementAxis, result });
      measurementLog.append(lineEntry(filePath, result, lineModel, documentMode ? useDq : true));
    } catch (e) {
      if (lineSeqRef.current === seq) setLineError(e instanceof Error ? e.message : String(e));
    } finally {
      if (lineSeqRef.current === seq) setLineLoading(false);
    }
  }, [
    filePath,
    range,
    lineSource,
    windows,
    channelCount,
    axis,
    documentMode,
    lineShiftKms,
    restUm,
    redshift,
    convention,
    lineModel,
    useErr,
    useDq,
    frameChoice,
    pickedShown,
  ]);

  const showLineFitPlane = useCallback(
    (run: LineFitRun, plane: LineFitPlane) => {
      const result = lineFitCubeResult(run, plane);
      if (result) publishCubeResult(result);
    },
    [publishCubeResult],
  );

  const handleFitLines = useCallback(async () => {
    if (!filePath || !fitRequest) return;
    if ("error" in fitRequest) {
      setLineFitError(fitRequest.error);
      return;
    }
    const config = fitRequest;
    const seq = ++lineFitSeqRef.current;
    const publication = framePublishGate.reserve();
    setLineFitLoading(true);
    setLineFitError(null);
    try {
      const result = await fitCubeLines(filePath, await getOutputDir(), config);
      if (lineFitSeqRef.current !== seq) return;
      const products = Object.values(result.planes).flatMap((plane) => plane?.fits_path ? [plane.fits_path] : []);
      const replaced = fitProductsRef.current.filter((path) => !products.includes(path));
      fitProductsRef.current = products;
      const run: LineFitRun = { key: `${filePath}#run${seq}`, filePath, config, result, lineLabel: pickedShown?.label ?? null };
      setLineFitRun(run);
      measurementLog.append(lineFitEntry(run));
      if (framePublishGate.isLatest(publication)) showLineFitPlane(run, preferredLineFitPlane(result));
      if (replaced.length) invalidateCubeResultRef.current?.(replaced);
    } catch (e) {
      if (lineFitSeqRef.current === seq) setLineFitError(e instanceof Error ? e.message : String(e));
    } finally {
      if (lineFitSeqRef.current === seq) setLineFitLoading(false);
    }
  }, [filePath, fitRequest, pickedShown, showLineFitPlane, framePublishGate]);

  const handleShowLineFitPlane = useCallback(
    (plane: LineFitPlane) => {
      if (lineFitRun) showLineFitPlane(lineFitRun, plane);
    },
    [lineFitRun, showLineFitPlane],
  );

  useLayoutEffect(() => {
    const shownPlane = lineFitRun ? displayedLineFitPlane(processed, lineFitRun.result) : null;
    if (!lineFitRun || !pixelCoord || region || shownPlane === null) {
      inspectSeqRef.current++;
      inspectKeyRef.current = null;
      setInspect(null);
      setInspectLoading(false);
      setInspectError(null);
      return;
    }
    const key = inspectKey(lineFitRun.filePath, pixelCoord.x, pixelCoord.y, lineFitRun);
    if (inspectKeyRef.current === key) return;
    inspectKeyRef.current = key;
    const seq = ++inspectSeqRef.current;
    setInspect(null);
    setInspectError(null);
    setInspectLoading(true);
    inspectLineFitSpaxel(lineFitRun.filePath, lineFitRun.config, pixelCoord.x, pixelCoord.y)
      .then((spaxel) => {
        if (inspectSeqRef.current === seq) setInspect(spaxel);
      })
      .catch((e: unknown) => {
        if (inspectSeqRef.current !== seq) return;
        inspectKeyRef.current = null;
        setInspectError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (inspectSeqRef.current === seq) setInspectLoading(false);
      });
  }, [pixelCoord, processed, lineFitRun, region]);

  const commitRange = useCallback(
    (edge: keyof ChannelRange, value: number) => {
      patchView({ range: commitRangeEdge(range ?? { z0: 0, z1: Math.max(channelCount - 1, 0) }, edge, value) });
    },
    [channelCount, range, patchView],
  );

  const commitWindow = useCallback(
    (side: WindowSide, edge: WindowEdge, value: number) => {
      patchView({
        windows: commitWindowEdge(windows ?? (range ? guardedContinuumWindows(range, channelCount) : null), side, edge, value),
      });
    },
    [channelCount, range, windows, patchView],
  );

  const labelMapping = useMemo(() => spectrumPlotMapping(plotModel, LABEL_MAPPING_WIDTH), [plotModel]);
  const rangeLabel = range ? formatRangeLabel(range, labelMapping, plotModel.xUnit) : "drag on the plot to select channels";
  const rangeBandRow = useMemo(() => {
    const band = rangeBand(range, plotParts.bands, plotModel);
    return band === null ? null : bandById(band.id);
  }, [range, plotParts.bands, plotModel]);
  const axisFields = fieldUnit === "axis" && plotModel.xValues !== null;
  const fieldMapping = axisFields ? labelMapping : null;
  const regionAvailable = !documentMode && !!filePath && !!picked.target && totalFrames > 1;
  const brushable = !!filePath && (documentMode ? n > 0 : !!cubeDims && totalFrames > 1);
  const velocityReady = mode === "velocity" || axis?.kind === "vrad" || axis?.kind === "vopt" || axis?.kind === "velo";
  const momentsReady = !!range && (restUm !== null || (axis?.rest_wavelength_um ?? null) !== null || velocityReady);
  const lineHint = !range
    ? "brush a line range first"
    : !axis
      ? noSpectralAxisText(documentMode)
      : !lineSource
        ? "click the preview or pick a region first"
        : null;
  const lineDisabled = lineLoading || lineHint !== null || !filePath;
  const overlapNote = continuumOverlapNote(windows, range, documentMode);
  const measuredSeriesNote =
    [
      region && shownRegionView !== "sum"
        ? `measured on the sum series (${fluxUnitLabel(bunit, "sum", "region")}), not on the ${shownRegionView === "jy" ? "Jy" : "mean"} series shown`
        : null,
      docSpectrum && plotModel.bins !== null ? "measured on the unbinned channels" : null,
      docSpectrum && plotModel.fluxState.choice !== "native" ? `measured in the unit of the file (${nativeFluxLabel(docSpectrum.flux_unit)})` : null,
    ]
      .filter((part) => part !== null)
      .join("; ") || null;
  const regionFactor = region?.noise_factor ?? null;
  const regionFactorStale = regionFactor ? noiseFactorStaleNote(regionFactor, continuumToSend) : null;
  const regionNotes = region ? regionNotesShown(region) : [];
  const regionErrors = region ? regionErrorLine(region.err_source, regionFactor, regionContinuum) : null;
  const bandKind = errorBandKind(bandErrors);
  const bandLegend = errorBandLegend(bandKind);
  const frameKeyHint = frameLinked ? (ramp ? rampKeyHint(ramp) : FRAME_KEY_HINT) : null;

  const regionButton = regionAvailable && (
    <button
      onClick={handleRegionSpectrum}
      disabled={regionLoading}
      className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[10px] font-medium transition-all disabled:opacity-40"
      style={{
        background: "rgba(34,211,238,0.08)",
        border: "1px solid rgba(34,211,238,0.2)",
        color: "#22d3ee",
      }}
      title={
        picked.background
          ? `${picked.target?.shape.shape} region with annulus background`
          : `${picked.target?.shape.shape} region (link an annulus as background for sky subtraction)`
      }
    >
      <CircleDot size={10} />
      {regionLoading ? "Extracting…" : "Region spectrum"}
    </button>
  );

  const frameNav = filePath && cubeDims && totalFrames > 1 && (
    <div key="frame-nav" className="px-2 pb-2 self-stretch">
      <CubeFrameNav
        filePath={filePath}
        totalFrames={totalFrames}
        frame={frame}
        requestSeq={requestSeq}
        onFrameRequest={requestFrame}
        frameLabel={frameLabel}
        labelFor={navLabelFor}
        loop={loopPlayback}
        onLoopChange={setLoopPlayback}
        onFrameChange={onFramePreview}
        publishGate={framePublishGate}
      />
    </div>
  );

  if (documentMode && n === 0 && !isLoading) {
    return (
      <div className="ab-panel p-6 flex flex-col items-center gap-3">
        <div
          className="w-10 h-10 rounded-xl flex items-center justify-center"
          style={{ background: "rgba(168,85,247,0.06)", border: "1px solid rgba(168,85,247,0.1)" }}
        >
          <FileText size={18} style={{ color: "var(--ab-violet)", opacity: 0.5 }} />
        </div>
        {error ? (
          <ErrorAlert compact className="text-center" message={`This spectrum could not be read: ${error}`} />
        ) : (
          <p className="text-[11px] text-zinc-500">This spectrum has no flux values to plot</p>
        )}
      </div>
    );
  }

  if (n === 0 && !isLoading && !regionLoading) {
    return (
      <div className="ab-panel p-6 flex flex-col items-center gap-3">
        <div
          className="w-10 h-10 rounded-xl flex items-center justify-center"
          style={{ background: "rgba(168,85,247,0.06)", border: "1px solid rgba(168,85,247,0.1)" }}
        >
          <Activity size={18} style={{ color: "var(--ab-violet)", opacity: 0.5 }} />
        </div>
        <p ref={holdShowCubeFocus} tabIndex={-1} className="text-[11px] text-zinc-500">
          Click on the preview image to extract a spectrum
        </p>
        {reopenedNote}
        {regionButton}
        {error && <ErrorAlert compact className="text-center" message={`Spectrum extraction failed: ${error}`} />}
        {regionError && <ErrorAlert compact className="text-center" message={`Region spectrum failed: ${regionError}`} />}
        {spectral && filePath && cubeDims && totalFrames > 1 && (
          <SpectrumComparisonSection
            filePath={filePath}
            regionDoc={regionDoc}
            axis={axis}
            mode={mode}
            restUm={restUm}
            convention={convention}
            correction={correction}
            correctionResult={correctionResult}
            pixelCoord={pixelCoord ?? null}
            view={regionView}
            bunit={bunit}
            channelCount={totalFrames}
            windows={windows}
            dqMasked={documentMode ? view.useDq : true}
          />
        )}
        {frameNav}
      </div>
    );
  }

  return (
    <div className="ab-panel overflow-hidden animate-fade-in">
      <div className="ab-panel-header">
        <div className="flex items-center gap-1.5">
          <Activity size={12} style={{ color: "var(--ab-violet)" }} />
          <span ref={holdShowCubeFocus} tabIndex={-1} className="text-[10px] font-semibold text-zinc-400 uppercase tracking-wider">
            Spectroscopy
          </span>
        </div>
        <div className="flex items-center gap-2 text-[10px] font-mono text-zinc-500">
          {(isLoading || regionLoading) && (
            <div
              className="w-3 h-3 rounded-full animate-spin"
              style={{ border: "1.5px solid transparent", borderTopColor: "var(--ab-violet)" }}
            />
          )}
          {region ? (
            <span className="flex items-center gap-1" style={{ color: "#22d3ee" }}>
              <CircleDot size={10} />
              {regionSource?.kind === "region" ? regionSource.shape.shape : "region"} · {region.npix.toFixed(1)} px
              {region.bg_subtracted ? " · sky-subtracted" : ""}
            </span>
          ) : documentMode && lineSource ? (
            <span className="flex items-center gap-1">
              <FileText size={10} />
              {spectrumSourceLabel(lineSource)}
            </span>
          ) : (
            pixelCoord && (
              <span className="flex items-center gap-1">
                <Crosshair size={10} />({pixelCoord.x}, {pixelCoord.y})
              </span>
            )
          )}
          {elapsed > 0 && <span>{elapsed}ms</span>}
        </div>
      </div>

      {reopenedNote && <div className="px-3 pb-2">{reopenedNote}</div>}

      <SpectrumViewControls viewKey={viewKey} spectrum={docSpectrum} model={plotModel} disabled={lineLoading} />

      {spectral && (
        <SpectralAxisControls
          filePath={filePath ?? null}
          axis={axis}
          mode={mode}
          onModeChange={changeMode}
          restValue={restUm}
          onRestChange={changeRestUm}
          convention={convention}
          onConventionChange={changeConvention}
          correction={correction}
          onCorrectionChange={setCorrection}
          onCorrectionLoaded={setCorrectionResult}
          correctionUnavailable={documentMode ? SPECTRUM_FILE_CORRECTION_NOTE : null}
        />
      )}

      {spectral && (
        <LineListControls
          visible={view.lineListVisible}
          onVisibleChange={changeLineListVisible}
          redshift={redshift}
          onRedshiftChange={changeRedshift}
          convention={lineConvention}
          families={view.lineFamilies}
          onFamiliesChange={changeLineFamilies}
          frameLabel={lineFrameLabel}
          availability={plotParts.availability}
          clickHint={lineClickHint(axis)}
          pickedLabel={pickedShown ? pickedLineLabel(pickedShown) : null}
          drawnCount={plotParts.marks.length}
          bandSpecies={view.bandSpecies}
          onBandSpeciesChange={changeBandSpecies}
          bandDrawnCount={plotParts.bands.length}
        />
      )}

      {spectral && !axis && axisError && (
        <p className="mx-3 mb-2 text-[10px] text-amber-300/80 px-2 py-1 rounded bg-amber-900/15 border border-amber-800/20">
          No spectral axis for this file: {axisError}
        </p>
      )}

      <div
        ref={containerRef}
        tabIndex={frameLinked ? 0 : undefined}
        onKeyDown={frameLinked ? handleKeyDown : undefined}
        title={frameKeyHint ?? undefined}
        className="p-2 rounded-md outline-none focus-visible:ring-1 focus-visible:ring-violet-500/50"
      >
        {isLoading ? (
          <div
            className="flex items-center justify-center rounded-md"
            style={{ height: SPECTRUM_PLOT_HEIGHT, background: "rgba(9,9,11,0.8)" }}
          >
            <div
              className="w-5 h-5 rounded-full animate-spin"
              style={{ border: "2px solid transparent", borderTopColor: "var(--ab-violet)" }}
            />
          </div>
        ) : (
          <SpectrumPlot
            model={plotModel}
            range={range}
            windows={windows}
            activeBrush={activeBrush}
            lineOverlay={lineOverlay}
            fitOverlay={fitOverlay}
            marks={plotParts.marks}
            bands={plotParts.bands}
            pickedMarkId={plotParts.pickedMarkId}
            frameMarker={frameMarker}
            referenceChannel={referenceChannel}
            onBrush={commitBrush}
            onPickMark={handlePlotClick}
            onPickBand={handleBandPick}
            onPlotDoubleClick={handlePlotDoubleClick}
            onPress={focusPlot}
            cursorKeys={documentMode}
            keyHint={documentMode ? DOCUMENT_CURSOR_KEY_HINT : frameKeyHint}
            status={frameLinked ? frameLabel : null}
            onViewChange={patchView}
          />
        )}
      </div>
      {docSpectrum?.err && (
        <div className="px-3 pb-2 text-[10px] font-mono text-zinc-500">
          {`error band: ${PLUS_MINUS}1 sigma from the file (${documentErrorOrigin(docSpectrum.source)})`}
        </div>
      )}

      {region && (
        <div className="px-3 pb-2 flex items-center gap-1.5 text-[10px] font-mono">
          <span className="text-zinc-500">Series</span>
          <span role="group" aria-label="Region series" className="flex items-center gap-1.5">
            {(["sum", "mean", ...(region.flux_jy ? ["jy"] : [])] as RegionView[]).map((view) => (
              <button
                key={view}
                aria-pressed={regionView === view}
                onClick={() => setRegionView(view)}
                className={`px-2 py-0.5 rounded border transition-colors${regionView === view ? " bg-zinc-800/80 font-semibold" : ""}`}
                style={{
                  borderColor: regionView === view ? "#22d3ee" : "var(--ab-border)",
                  color: regionView === view ? "#22d3ee" : "var(--ab-text-3)",
                }}
              >
                {view === "jy" ? "Jy" : view}
              </button>
            ))}
          </span>
          {region.bg_subtracted && <span className="text-zinc-600">bg {region.n_bg} px</span>}
          <button
            onClick={() => clearRegionSpectrum()}
            className="ml-auto px-2 py-0.5 rounded border transition-colors text-zinc-500"
            style={{ borderColor: "var(--ab-border)" }}
          >
            pixel
          </button>
        </div>
      )}
      {region && (region.err !== null || regionNotes.length > 0) && (
        <div className="px-3 pb-2 flex flex-col gap-1 text-[10px] font-mono text-zinc-500">
          {region.err !== null && regionErrors && (
            <span>
              <span data-noise-factor={regionErrors.factor}>{regionErrors.text}</span>
              {bandLegend && <span className="text-zinc-600">{` (${bandLegend})`}</span>}
            </span>
          )}
          {regionFactorStale && <span className="text-zinc-600">{regionFactorStale}</span>}
          {regionFactor?.warning && (
            <p
              data-noise-warning=""
              className="text-amber-300/80 px-2 py-1 rounded bg-amber-900/15 border border-amber-800/20 break-words"
            >
              {regionFactor.warning}
            </p>
          )}
          {regionNotes.length > 0 && (
            <details className="text-zinc-600">
              <summary className="cursor-pointer select-none">notes ({regionNotes.length})</summary>
              <ul className="mt-0.5 flex flex-col gap-0.5">
                {regionNotes.map((note, i) => (
                  <li key={i} className="break-words">
                    {note}
                  </li>
                ))}
              </ul>
            </details>
          )}
        </div>
      )}
      {regionError && <ErrorAlert compact className="mx-3 mb-2" message={`Region spectrum failed: ${regionError}`} />}

      {n > 0 && filePath && measureSource && (
        <div className="px-3 pb-2 flex items-center gap-2 text-[10px] font-mono">
          <span className="text-zinc-500">Export</span>
          <button
            onClick={() => void handleExportSpectrum("copy")}
            disabled={exportBusy}
            className={SMALL_BUTTON_CLASS}
          >
            Copy CSV
          </button>
          <button
            onClick={() => void handleExportSpectrum("save")}
            disabled={exportBusy}
            className={SMALL_BUTTON_CLASS}
          >
            Save CSV
          </button>
        </div>
      )}
      {n > 0 && filePath && measureSource && (
        <div className="px-3 pb-2">
          <ExportNotice saved={savedPath} copied={copied} error={exportError} />
        </div>
      )}

      {cubeDims && (
        <div className="px-3 pb-1 text-[10px] font-mono text-zinc-600">
          Cube: {cubeDims.width} × {cubeDims.height} × {cubeDims.frames}
          {n > 0 ? ` — ${n} channels` : ""}
        </div>
      )}

      {docSpectrum && (
        <div className="px-3 pb-1 text-[10px] font-mono text-zinc-600">
          <span>{documentSummary(docSpectrum)}</span>
          {setupFrom !== null && (
            <div data-setup-from className="mt-0.5 flex items-center gap-2">
              <span>{`line setup carried over from ${setupFrom}`}</span>
              <button
                data-setup-clear
                title="Reset the axis, rest wavelength, redshift, line families, model, range and windows of this spectrum"
                onClick={() => {
                  spectrumViewStore.dismissSetup(viewKey);
                  patchView(clearedSetup());
                  setSetupFrom(null);
                }}
                className="text-zinc-500 hover:text-zinc-300"
              >
                clear
              </button>
            </div>
          )}
          {docSpectrum.notes.length > 0 && (
            <details className="mt-0.5">
              <summary className="cursor-pointer select-none">notes ({docSpectrum.notes.length})</summary>
              <ul className="mt-0.5 flex flex-col gap-0.5">
                {docSpectrum.notes.map((note, i) => (
                  <li key={i} className="break-words">
                    {note}
                  </li>
                ))}
              </ul>
            </details>
          )}
        </div>
      )}

      {filePath && cubeDims && totalFrames > 1 && (
        <div className="px-3 pb-2 flex items-center gap-2 flex-wrap" style={{ borderTop: "1px solid var(--ab-border)", paddingTop: 8 }}>
          <label className="w-full flex items-center gap-2 text-[10px] text-zinc-400">Science DQ<select aria-label="Science DQ policy" className={SELECT_CLASS} value={scienceDqPolicy} onChange={(e) => changeUseDq(e.target.value === "do_not_use")}><option value="do_not_use">Exclude DO_NOT_USE (bit 0)</option><option value="ignore">Ignore DQ flags</option></select></label>
          <p className="w-full text-[10px] text-zinc-500">Shared by Fit lines, Moment Maps and Collapse. Only DO_NOT_USE (bit 0) excludes samples; other valid flag bits do not. Results report whether DQ was applied and warn when it is unavailable.</p>
          <label className="w-full flex items-center gap-2 text-[10px] text-zinc-400">Collapse zeros<select aria-label="Collapse zero handling" className={SELECT_CLASS} value={collapseZeroPolicy} onChange={(e) => setCollapseZeroPolicy(e.target.value as CollapseZeroPolicy)}><option value="include_physical_zeros">Include measured zeros</option><option value="exclude_zero_padding">Exclude zero padding (legacy)</option></select></label>
          <p className="w-full text-[10px] text-zinc-500">Applies to full-cube and range Collapse. Exclude zeros only when they represent known padding. Non-finite values are always excluded.</p>
          <CollapseBtn
            label="Collapse Mean"
            loading={collapseLoading && collapseMode === "mean"}
            disabled={collapseLoading}
            color="var(--ab-violet)"
            onClick={() => handleCollapse("mean")}
          />
          <CollapseBtn
            label="Collapse Median"
            loading={collapseLoading && collapseMode === "median"}
            disabled={collapseLoading}
            color="var(--ab-amber)"
            onClick={() => handleCollapse("median")}
          />
          {regionButton}
          {collapseResult && !collapseLoading && (
            <span className="text-[10px] font-mono text-zinc-500 ml-auto">
              {collapseResult.elapsed_ms}ms
            </span>
          )}
          {collapseError && <ErrorAlert compact className="w-full" message={`Cube collapse failed: ${collapseError}`} />}
          {collapseResult?.fits_path && !collapseLoading && (
            <div className="w-full">
              <p className="text-[10px] text-zinc-500">Zero handling: {collapseResult.zero_policy === "include_physical_zeros" ? "measured zeros included" : "exact zero padding excluded (legacy)"}</p>
              <p className="text-[10px] text-zinc-500" data-dq-policy="full-collapse">{scienceDqSummary(collapseResult)}</p>
              {collapseResult.notes.length > 0 && <ul className="list-disc pl-4 text-[10px] text-zinc-500">{collapseResult.notes.map((note, index) => <li key={index}>{note}</li>)}</ul>}
              <OutputNote path={collapseResult.fits_path} />
              {collapseResult.provenance_path && <OutputNote path={collapseResult.provenance_path} kind="manifest" provenance={collapseResult.provenance} />}
            </div>
          )}
        </div>
      )}

      {brushable && (
        <div className="px-3 pb-2 flex flex-col gap-2" style={{ borderTop: "1px solid var(--ab-border)", paddingTop: 8 }}>
          <div className="flex items-center gap-2 flex-wrap text-[10px] font-mono">
            {spectral && (
              <>
                <label htmlFor={brushTargetId} className={LABEL_CLASS}>
                  Brush
                </label>
                <select id={brushTargetId} value={brushTarget} onChange={(e) => setBrushTarget(e.target.value as BrushTarget)} className={SELECT_CLASS}>
                  <option value="range">line range</option>
                  <option value="left">continuum A</option>
                  <option value="right">continuum B</option>
                </select>
              </>
            )}
            <span className="text-zinc-500">{rangeLabel}</span>
            {rangeBandRow && (
              <span data-range-band className="text-zinc-600">
                {pickedBandLabel(rangeBandRow)}
              </span>
            )}
          </div>

          <div className="flex items-center gap-2 flex-wrap text-[10px] font-mono">
            <span className={LABEL_CLASS}>Range</span>
            {plotModel.xValues !== null && (
              <span role="group" aria-label="Range field unit" className="flex items-center gap-1">
                {RANGE_FIELD_UNITS.map((unit) => (
                  <button
                    key={unit}
                    data-range-fields={unit}
                    aria-pressed={fieldUnit === unit}
                    onClick={() => setFieldUnit(unit)}
                    className="px-1.5 py-0.5 rounded border transition-colors"
                    style={{
                      borderColor: fieldUnit === unit ? "var(--ab-violet)" : "var(--ab-border)",
                      color: fieldUnit === unit ? "var(--ab-violet)" : "var(--ab-text-3)",
                    }}
                  >
                    {unit === "channel" ? "ch" : plotModel.xUnit}
                  </button>
                ))}
              </span>
            )}
            <RangeEdgeField
              edge="range:z0"
              channel={range ? range.z0 : null}
              channelCount={channelCount}
              mapping={fieldMapping}
              unit={plotModel.xUnit}
              channelLabel="Range first channel z0"
              axisLabel={`Range first channel, in ${plotModel.xUnit}`}
              placeholder="z0"
              onCommit={(value) => commitRange("z0", value)}
            />
            <RangeEdgeField
              edge="range:z1"
              channel={range ? range.z1 : null}
              channelCount={channelCount}
              mapping={fieldMapping}
              unit={plotModel.xUnit}
              channelLabel="Range last channel z1"
              axisLabel={`Range last channel, in ${plotModel.xUnit}`}
              placeholder="z1"
              onCommit={(value) => commitRange("z1", value)}
            />
            {!documentMode && (
              <>
                <select
                  value={rangeMode}
                  aria-label="Range collapse mode"
                  onChange={(e) => setRangeMode(e.target.value as CollapseRangeMode)}
                  className={SELECT_CLASS}
                >
                  {COLLAPSE_RANGE_MODES.map((m) => (
                    <option key={m} value={m}>
                      {m}
                    </option>
                  ))}
                </select>
                <CollapseBtn
                  label="Collapse range"
                  loading={rangeLoading}
                  disabled={rangeLoading || !range}
                  color="var(--ab-violet)"
                  onClick={handleCollapseRange}
                />
              </>
            )}
            {range && (
              <button onClick={() => patchView({ range: null })} className="text-zinc-500 hover:text-zinc-300">
                clear
              </button>
            )}
          </div>
          {rangeError && <ErrorAlert compact message={rangeError} />}
          {rangeStale && !documentMode && <StaleResultNote />}
          {rangeResult && !documentMode && (
            <div className="text-[10px] font-mono text-zinc-500">
              <span>{`${rangeResult.mode} collapse, ch ${rangeResult.z0}–${rangeResult.z1}: `}</span>
              <span data-err-plane="collapse">{errPlaneLabel(rangeResult.has_err, rangeResult.err_source, null)}</span>
              <p>Zero handling: {rangeResult.zero_policy === "include_physical_zeros" ? "measured zeros included" : "exact zero padding excluded (legacy)"}</p>
              <p data-dq-policy="collapse">{scienceDqSummary(rangeResult)}</p>
              {rangeResult.fits_path && <OutputNote path={rangeResult.fits_path} />}
              {rangeResult.provenance_path && <OutputNote path={rangeResult.provenance_path} kind="manifest" provenance={rangeResult.provenance} />}
              {rangeResult.notes.length > 0 && (
                <details className="mt-0.5 text-zinc-600">
                  <summary className="cursor-pointer select-none">notes ({rangeResult.notes.length})</summary>
                  <ul className="mt-0.5 flex flex-col gap-0.5">
                    {rangeResult.notes.map((note, i) => (
                      <li key={i} className="break-words">
                        {note}
                      </li>
                    ))}
                  </ul>
                </details>
              )}
            </div>
          )}

          {spectral && (
            <>
              <div className="flex items-center gap-2 flex-wrap text-[10px] font-mono">
                <span className={LABEL_CLASS}>Continuum</span>
                {([0, 1] as const).map((side) => (
                  <span key={side} className="flex items-center gap-1">
                    <span className="text-zinc-600">{side === 0 ? "A" : "B"}</span>
                    <RangeEdgeField
                      edge={`window:${side}:0`}
                      channel={windows ? windows[side][0] : null}
                      channelCount={channelCount}
                      mapping={fieldMapping}
                      unit={plotModel.xUnit}
                      channelLabel={`Continuum ${side === 0 ? "A" : "B"} first channel`}
                      axisLabel={`Continuum ${side === 0 ? "A" : "B"} first channel, in ${plotModel.xUnit}`}
                      placeholder="from"
                      onCommit={(value) => commitWindow(side, 0, value)}
                    />
                    <RangeEdgeField
                      edge={`window:${side}:1`}
                      channel={windows ? windows[side][1] : null}
                      channelCount={channelCount}
                      mapping={fieldMapping}
                      unit={plotModel.xUnit}
                      channelLabel={`Continuum ${side === 0 ? "A" : "B"} last channel`}
                      axisLabel={`Continuum ${side === 0 ? "A" : "B"} last channel, in ${plotModel.xUnit}`}
                      placeholder="to"
                      onCommit={(value) => commitWindow(side, 1, value)}
                    />
                  </span>
                ))}
                <button
                  onClick={() => range && patchView({ windows: guardedContinuumWindows(range, channelCount) })}
                  disabled={!range}
                  title={AUTO_WINDOWS_RULE}
                  className="text-zinc-500 hover:text-zinc-300 disabled:opacity-40"
                >
                  auto
                </button>
                {windows && (
                  <button
                    onClick={() => patchView({ windows: null })}
                    title="no windows: the measurement then takes two windows as wide as the line range, touching it"
                    className="text-zinc-500 hover:text-zinc-300"
                  >
                    none
                  </button>
                )}
              </div>
              {axisFields && windows && (
                <p data-continuum-channels className="text-[10px] font-mono text-zinc-500">
                  {`A ch ${windows[0][0]}–${windows[0][1]} · B ch ${windows[1][0]}–${windows[1][1]}`}
                </p>
              )}
              {overlapNote && (
                <p className="text-[10px] text-amber-300/80 px-2 py-1 rounded bg-amber-900/15 border border-amber-800/20">{overlapNote}</p>
              )}

              {!documentMode && (
                <>
                  <div className="flex items-center gap-2 flex-wrap text-[10px] font-mono">
                    <label htmlFor={snrId} className={LABEL_CLASS}>
                      SNR
                    </label>
                    <DecimalField
                      id={snrId}
                      min={SNR_FIELD.min}
                      value={snrText}
                      onChange={setSnrText}
                      className={INPUT_CLASS}
                    />
                    <label className="flex items-center gap-1 text-zinc-500 cursor-pointer">
                      <input type="checkbox" checked={maskBelow} onChange={(e) => setMaskBelow(e.target.checked)} />
                      mask M0
                    </label>
                    <CollapseBtn
                      label="Moments"
                      icon={<BarChart3 size={10} />}
                      loading={momentLoading}
                      disabled={momentLoading || !momentsReady}
                      color="var(--ab-amber)"
                      onClick={handleMoments}
                    />
                    {!momentsReady && range && (
                      <span className="text-zinc-600">set a rest wavelength for velocity moments</span>
                    )}
                    {momentResult && (
                      <span role="group" aria-label="Moment map" className="flex items-center gap-1 ml-auto">
                        {MOMENT_KINDS.map((kind) => (
                          <button
                            key={kind}
                            aria-pressed={momentKind === kind}
                            onClick={() => showMoment(momentResult, kind)}
                            className={`px-2 py-0.5 rounded border transition-colors uppercase${momentKind === kind ? " bg-zinc-800/80 font-semibold" : ""}`}
                            style={{
                              borderColor: momentKind === kind ? "var(--ab-amber)" : "var(--ab-border)",
                              color: momentKind === kind ? "var(--ab-amber)" : "var(--ab-text-3)",
                            }}
                          >
                            {kind}
                          </button>
                        ))}
                      </span>
                    )}
                  </div>
                  {momentError && <ErrorAlert compact message={momentError} />}
                  {momentStale && <StaleResultNote />}
                  {momentResult && (
                    <div className="text-[10px] font-mono text-zinc-500">
                      <span>
                        M0 in {momentResult.m0_unit}, M1/M2 in {momentResult.velocity_unit}, {momentResult.n_channels} channels
                        {momentResult.noise_per_channel !== null ? `, noise ${momentResult.noise_per_channel.toExponential(2)}` : ""}
                        {`, ${momentVelocityFrameNote(axis?.specsys ?? null, correction, lineShiftKms)}`}
                      </span>
                      <span className="block" data-err-plane="moments">
                        {errPlaneLabel(momentResult.has_err, momentResult.err_source, momentResult.noise_factor)}
                      </span>
                      <p data-dq-policy="moments">{scienceDqSummary(momentResult)}</p>
                      {momentResult[momentKind].fits_path && <OutputNote path={momentResult[momentKind].fits_path} />}
                      {momentResult.provenance_path && <OutputNote path={momentResult.provenance_path} kind="manifest" provenance={momentResult.provenance} />}
                      {momentResult.notes.length > 0 && (
                        <details className="mt-0.5 text-zinc-600">
                          <summary className="cursor-pointer select-none">notes ({momentResult.notes.length})</summary>
                          <ul className="mt-0.5 flex flex-col gap-0.5">
                            {momentResult.notes.map((note, i) => (
                              <li key={i} className="break-words">
                                {note}
                              </li>
                            ))}
                          </ul>
                        </details>
                      )}
                    </div>
                  )}
                </>
              )}
            </>
          )}
        </div>
      )}

      {spectral && filePath && cubeDims && totalFrames > 1 && (
        <LineFitSection
          ready={momentsReady && !!range}
          readyHint={LINE_FIT_READY_HINT}
          loading={lineFitLoading}
          error={lineFitError}
          errAvailable={lineFitRun && lineFitRun.config.use_err ? lineFitRun.result.err_hdu !== null : null}
          dqAvailable={lineFitRun && lineFitRun.config.use_dq ? lineFitRun.result.dq_hdu !== null : null}
          useErr={useErr}
          onUseErr={setUseErr}
          useDq={useDq}
          onUseDq={changeUseDq}
          emissionOnly={emissionOnly}
          onEmissionOnly={setEmissionOnly}
          snrText={lineFitSnrText}
          onSnrText={setLineFitSnrText}
          resolvingPowerText={resolvingPowerText}
          onResolvingPowerText={setResolvingPowerText}
          components={lineFitComponents}
          onComponents={setLineFitComponents}
          onFit={handleFitLines}
          run={lineFitRun}
          stale={fitStale}
          shownPlane={lineFitRun ? (displayedLineFitPlane(processed, lineFitRun.result) ?? null) : null}
          onShowPlane={handleShowLineFitPlane}
          inspect={inspect}
          inspectLoading={inspectLoading}
          inspectError={inspectError}
          frameNote={lineFitFrameNote(axis?.specsys ?? null)}
          redshift={redshift}
        />
      )}

      {spectral && brushable && (
        <LineMeasurementSection
          result={visibleLineResult}
          loading={lineLoading}
          error={lineError}
          model={lineModel}
          onModelChange={changeLineModel}
          onMeasure={handleMeasureLine}
          disabled={lineDisabled}
          disabledHint={lineHint}
          errControls={
            docSpectrum
              ? {
                  useErr,
                  onUseErr: setUseErr,
                  useDq,
                  onUseDq: changeUseDq,
                  errAvailable: docSpectrum.err !== null,
                  dqAvailable: docSpectrum.dq !== null,
                }
              : null
          }
          axisDisplayUnit={view.wavelengthUnit}
          measuredSeriesNote={measuredSeriesNote}
          filePath={filePath ?? null}
        />
      )}

      {spectral && filePath && (docSpectrum ? n > 0 : !!cubeDims && totalFrames > 1) && (
        <SpectrumComparisonSection
          filePath={filePath}
          regionDoc={regionDoc}
          axis={axis}
          mode={mode}
          restUm={restUm}
          convention={convention}
          correction={correction}
          correctionResult={correctionResult}
          pixelCoord={pixelCoord ?? null}
          view={regionView}
          bunit={bunit}
          channelCount={docSpectrum ? n : totalFrames}
          windows={windows}
          fileSpectrum={docSpectrum}
          fileSpectrumSource={documentMode ? (spectrumDocument?.source ?? null) : null}
          dqMasked={documentMode ? view.useDq : true}
        />
      )}

      {frameNav}
    </div>
  );
}

function CollapseBtn({
  label,
  loading,
  disabled,
  color,
  icon,
  onClick,
}: {
  label: string;
  loading: boolean;
  disabled: boolean;
  color: string;
  icon?: React.ReactNode;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[10px] font-medium transition-all disabled:opacity-40"
      style={{
        background: `color-mix(in srgb, ${color} 8%, transparent)`,
        border: `1px solid color-mix(in srgb, ${color} 20%, transparent)`,
        color,
      }}
    >
      {loading ? (
        <div className="w-2.5 h-2.5 rounded-full animate-spin" style={{ border: "1.5px solid transparent", borderTopColor: color }} />
      ) : (
        (icon ?? <Layers size={10} />)
      )}
      {label}
    </button>
  );
}

export default memo(SpectroscopyPanel);
