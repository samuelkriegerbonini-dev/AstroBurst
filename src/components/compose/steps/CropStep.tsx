import { useState, useCallback, useEffect, useId, useMemo, useRef } from "react";
import { Scissors, Wand2 } from "lucide-react";
import type { WizardState } from "../wizard";
import {
  channelOverlayPreview,
  cropChannels,
  detectCropBounds,
  type CropResult,
} from "../../../services/compose";
import type { ChannelOverlayPreview, CropBounds } from "../../../shared/types/compose";
import { getOutputDir } from "../../../infrastructure/tauri";
import { useComposeWizardContext } from "../../../context/ComposeWizardContext";
import { RunButton } from "../../ui";
import CropEditor from "../CropEditor";
import { alignOverlayColours, discardsNotice, MAX_OVERLAY_CHANNELS, rerunDiscards } from "../../../utils/wizard";
import {
  applyWaitsForDetection,
  cropOverlayKeys,
  draftMargins,
  editDraft,
  gridSize,
  INITIAL_MARGIN_DRAFT,
  MARGIN_EDGES,
  marginEdgeMax,
  marginsFromBounds,
  prefillDraft,
  replaceDraft,
  setMarginEdge,
  typeMarginDraft,
  ZERO_MARGINS,
  type CropMargins,
  type MarginDraft,
  type MarginEdge,
} from "../../../utils/cropRect";

interface CropStepProps {
  state: WizardState;
  onCropped: (paths: Record<string, string>) => void;
}

const OVERLAY_MAX_DIM = 1024;
const CONTROLS_MIN_PX = 220;
const CONTROLS_MAX_PX = 460;

const MARGIN_LABELS: Record<MarginEdge, string> = {
  top: "Top",
  bottom: "Bottom",
  left: "Left",
  right: "Right",
};

const SECONDARY_BUTTON_CLASS =
  "px-3 py-1.5 rounded text-[10px] text-zinc-400 hover:text-zinc-200 border border-zinc-700/50 hover:border-zinc-600 transition-all disabled:opacity-40";

const MARGIN_INPUT_CLASS =
  "w-20 rounded border border-zinc-700/50 bg-zinc-900/80 px-2 py-1 text-right font-mono text-[11px] text-zinc-200 transition-colors hover:border-zinc-600 focus:border-cyan-500/60";

export default function CropStep({ state, onCropped }: CropStepProps) {
  const marginId = useId();
  const autoReasonId = useId();
  const [draft, setDraft] = useState<MarginDraft>(INITIAL_MARGIN_DRAFT);
  const [bounds, setBounds] = useState<CropBounds | null>(null);
  const [boundsError, setBoundsError] = useState("");
  const [detecting, setDetecting] = useState(true);
  const [overlay, setOverlay] = useState<ChannelOverlayPreview | null>(null);
  const [overlayError, setOverlayError] = useState("");
  const [previewing, setPreviewing] = useState(true);
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<CropResult | null>(null);
  const [error, setError] = useState("");
  const [skipped, setSkipped] = useState(false);
  const [stepHeight, setStepHeight] = useState(0);
  const loadSeqRef = useRef(0);
  const { alignRun } = useComposeWizardContext();
  const aligning = alignRun?.running ?? false;

  const observeStep = useCallback((element: HTMLDivElement | null) => {
    if (!element) return;
    const update = () => {
      const style = getComputedStyle(element);
      const padding = parseFloat(style.paddingTop) + parseFloat(style.paddingBottom);
      const height = element.getBoundingClientRect().height - (Number.isFinite(padding) ? padding : 0);
      setStepHeight(Math.max(0, Math.floor(height)));
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const alignedEntries = useMemo(() => {
    return Object.entries(state.alignedPaths).filter(([, p]) => !!p);
  }, [state.alignedPaths]);
  const alignedKeys = useMemo(() => alignedEntries.map(([, path]) => path), [alignedEntries]);
  const overlayLegend = useMemo(() => {
    const labels = alignedEntries
      .slice(0, MAX_OVERLAY_CHANNELS)
      .map(([binId]) => state.bins.find((b) => b.id === binId)?.shortLabel ?? binId);
    return alignOverlayColours(labels);
  }, [alignedEntries, state.bins]);

  useEffect(() => {
    const seq = ++loadSeqRef.current;
    const isCurrent = () => loadSeqRef.current === seq;
    setBounds(null);
    setBoundsError("");
    setOverlay(null);
    setOverlayError("");
    setResult(null);
    setError("");
    setSkipped(false);
    setDraft((prev) => replaceDraft(prev, ZERO_MARGINS, "auto"));
    const load = !aligning && alignedKeys.length > 0;
    setDetecting(load);
    setPreviewing(load);
    if (!load) return;

    detectCropBounds(alignedKeys)
      .then((res) => {
        if (!isCurrent()) return;
        setBounds(res);
        setDraft((prev) => prefillDraft(prev, marginsFromBounds(res)));
      })
      .catch((e) => {
        if (isCurrent()) setBoundsError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (isCurrent()) setDetecting(false);
      });

    const { keys, maskKeys } = cropOverlayKeys(alignedKeys, MAX_OVERLAY_CHANNELS);
    channelOverlayPreview(keys, { maskKeys: maskKeys.length > 0 ? maskKeys : undefined, maxDim: OVERLAY_MAX_DIM })
      .then((res) => {
        if (isCurrent()) setOverlay(res);
      })
      .catch((e) => {
        if (isCurrent()) setOverlayError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (isCurrent()) setPreviewing(false);
      });

    return () => {
      loadSeqRef.current += 1;
    };
  }, [alignedKeys, aligning]);

  const grid = useMemo(
    () => gridSize(bounds?.dimensions) ?? gridSize(overlay?.dimensions),
    [bounds, overlay],
  );
  const previewSize = useMemo(() => gridSize(overlay?.preview_dimensions), [overlay]);
  const margins = useMemo(() => draftMargins(draft, grid), [draft, grid]);

  const handleEditorChange = useCallback((next: CropMargins, basedOnRevision: number) => {
    setDraft((prev) => editDraft(prev, next, basedOnRevision));
  }, []);

  const handleMarginInput = useCallback(
    (edge: MarginEdge, raw: string) => {
      const parsed = parseInt(raw, 10);
      const next = setMarginEdge(margins, edge, Number.isFinite(parsed) ? parsed : 0, grid);
      setDraft((prev) => typeMarginDraft(prev, next, edge));
    },
    [margins, grid],
  );

  const handleAuto = useCallback(() => {
    if (!bounds?.auto_detected) return;
    const detected = marginsFromBounds(bounds);
    setDraft((prev) => replaceDraft(prev, detected, "auto"));
  }, [bounds]);

  const handleCrop = useCallback(async () => {
    if (alignedEntries.length === 0 || aligning) return;
    setLoading(true);
    setError("");
    setSkipped(false);
    try {
      const paths = alignedEntries.map(([, p]) => p);
      const binIds = alignedEntries.map(([binId]) => binId);
      const dir = await getOutputDir();
      const res = await cropChannels(
        paths,
        dir,
        margins.top,
        margins.bottom,
        margins.left,
        margins.right,
        false,
        binIds,
      );
      setResult(res);

      const cropped: Record<string, string> = {};
      const keys = res.cache_keys;
      if (keys && keys.length > 0) {
        alignedEntries.forEach(([binId], i) => {
          if (keys[i]) {
            cropped[binId] = keys[i];
          }
        });
      } else if (res.paths) {
        alignedEntries.forEach(([binId], i) => {
          if (res.paths[i]) {
            cropped[binId] = res.paths[i];
          }
        });
      }
      onCropped(cropped);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [alignedEntries, aligning, margins, onCropped]);

  const handleSkip = useCallback(() => {
    if (aligning) return;
    setSkipped(true);
    const passthrough: Record<string, string> = {};
    for (const [binId, path] of alignedEntries) {
      passthrough[binId] = path;
    }
    onCropped(passthrough);
  }, [alignedEntries, aligning, onCropped]);

  if (aligning) {
    return (
      <div className="flex items-center justify-center py-12 text-zinc-500 text-xs">
        Align is running… Crop reloads when it finishes.
      </div>
    );
  }

  if (alignedEntries.length === 0) {
    return (
      <div className="flex items-center justify-center py-12 text-zinc-600 text-xs">
        Run Alignment first to enable cropping.
      </div>
    );
  }

  const autoAvailable = !!bounds?.auto_detected;
  const autoReason = detecting
    ? "Detecting the borders of valid data..."
    : boundsError
      ? `Border detection failed: ${boundsError}`
      : bounds && !bounds.auto_detected
        ? "Auto unavailable: no region holds valid data in every channel."
        : "";
  const sourceText = draft.source === "manual" ? "Manual margins" : autoAvailable ? "Auto-detected margins" : "";
  const applyWaiting = applyWaitsForDetection(draft, detecting);
  const discards = discardsNotice("Apply or Skip", rerunDiscards(state, "crop"));

  return (
    <div ref={observeStep} className="flex h-full flex-wrap items-start gap-3 p-3">
      <CropEditor
        grid={grid}
        availableHeight={stepHeight}
        margins={margins}
        revision={draft.revision}
        imageUrl={overlay?.previewUrl ?? null}
        previewSize={previewSize}
        loading={detecting || previewing}
        previewError={overlayError}
        onChange={handleEditorChange}
      />

      <div
        className="flex flex-col gap-2 pb-3"
        style={{ flex: `1 1 ${CONTROLS_MIN_PX}px`, minWidth: CONTROLS_MIN_PX, maxWidth: CONTROLS_MAX_PX }}
      >
        <div className="flex items-baseline gap-1.5">
          <Scissors size={12} className="self-center text-cyan-400" />
          <span className="text-xs text-zinc-300">Crop aligned channels</span>
          <span className="text-[9px] text-zinc-500">{alignedEntries.length} ready</span>
        </div>

        <div className="grid max-w-[280px] grid-cols-[repeat(auto-fit,minmax(132px,max-content))] justify-start gap-x-4 gap-y-1.5">
          {MARGIN_EDGES.map((edge) => (
            <div key={edge} className="flex items-center gap-2">
              <label htmlFor={`${marginId}-${edge}`} className="w-11 text-[10px] text-zinc-400">
                {MARGIN_LABELS[edge]}
              </label>
              <input
                id={`${marginId}-${edge}`}
                type="number"
                aria-label={`${MARGIN_LABELS[edge]} margin (px)`}
                min={0}
                max={grid ? marginEdgeMax(margins, edge, grid) : undefined}
                step={1}
                value={margins[edge]}
                onChange={(e) => handleMarginInput(edge, e.target.value)}
                className={MARGIN_INPUT_CLASS}
              />
            </div>
          ))}
        </div>

        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={handleAuto}
            disabled={!autoAvailable || loading}
            aria-describedby={autoReason ? autoReasonId : undefined}
            title={autoReason || "Fill the margins with the borders of data valid in every channel"}
            className={`flex items-center gap-1 ${SECONDARY_BUTTON_CLASS}`}
          >
            <Wand2 size={10} />
            Auto
          </button>
          {sourceText && <span className="text-[9px] text-zinc-500">{sourceText}</span>}
        </div>
        {autoReason && (
          <div id={autoReasonId} className={`text-[9px] ${boundsError ? "text-red-400" : "text-zinc-500"}`}>
            {autoReason}
          </div>
        )}

        <div className="flex items-center gap-2">
          <RunButton
            label="Apply Crop"
            runningLabel="Cropping..."
            running={loading}
            disabled={applyWaiting}
            describedBy={applyWaiting ? autoReasonId : undefined}
            accent="cyan"
            onClick={handleCrop}
          />
          <button
            onClick={handleSkip}
            disabled={loading}
            className={SECONDARY_BUTTON_CLASS}
          >
            Skip
          </button>
        </div>
        {discards && !loading && <div className="text-[9px] text-amber-400/80">{discards}</div>}

        {skipped && (
          <div className="text-[9px] text-zinc-500">
            Crop skipped. Aligned paths passed through directly.
          </div>
        )}

        {result && !skipped && (
          <div className="text-[9px] text-zinc-500">
            Cropped to {result.dimensions?.[0]}x{result.dimensions?.[1]}, margins [{result.crop_top}, {result.crop_bottom}, {result.crop_left}, {result.crop_right}], {result.elapsed_ms}ms
          </div>
        )}
        {error && <div className="text-[9px] text-red-400">{error}</div>}

        <div className="flex flex-col gap-1 text-[10px] text-zinc-500">
          <span>
            Drag the box or its handles, or type the margins in pixels. Apply crops every aligned channel to the box.
          </span>
          <span>Image: {overlayLegend}</span>
          <span>
            Pixels with no data in at least one channel show as a dark grey checkerboard, which looks plain dark grey
            when the image is small.
          </span>
        </div>

        <div className="flex flex-col gap-1 pt-1 border-t border-zinc-800/30">
          <span className="text-[9px] text-zinc-600 uppercase tracking-wider">Channels</span>
          {alignedEntries.map(([binId, path]) => {
            const bin = state.bins.find((b) => b.id === binId);
            const hasCropped = !!state.croppedPaths[binId];
            const displayName = path.startsWith("__wizard_ch_")
              ? bin?.shortLabel ?? binId
              : path.split(/[/\\]/).pop();
            return (
              <div key={binId} className="flex items-center gap-1.5 py-0.5">
                <span className="w-2 h-2 rounded-full" style={{ background: bin?.color }} />
                <span className="text-[10px] text-zinc-300">{bin?.shortLabel}</span>
                <span className="text-[8px] text-zinc-700 font-mono truncate flex-1">{displayName}</span>
                {hasCropped && <span className="text-[8px] text-cyan-400/60">cropped</span>}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
