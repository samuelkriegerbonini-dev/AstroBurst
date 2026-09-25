import { memo, useCallback, useEffect, useId, useRef, useState } from "react";
import { ClipboardCopy, Eye, EyeOff, Layers, Loader2, Shapes } from "lucide-react";
import { contourLines } from "../../services/contours";
import { CONTOUR_MODES } from "../../shared/types/contours";
import type { ContourMode, ContourResult } from "../../shared/types/contours";
import { useDqContext } from "../../context/PreviewContext";
import { overlayStore } from "../../utils/overlayStore";
import { regionStore } from "../../utils/regionStore";
import { DEFAULT_REGION_PROPS } from "../../utils/regionPersistence";
import { generateId } from "../../utils/format";
import {
  buildContourRequest,
  closedContourPolygons,
  formatLevelValue,
  levelColour,
  levelsText,
  suggestBin,
  CONTOUR_BIN_CHOICES,
  MAX_REGION_POLYGONS,
  type ContourBinChoice,
  type ContourColourMode,
} from "../../utils/contourLevels";
import { CONTOUR_LAYER_ID, CONTOUR_LAYER_KIND, createContourPainter } from "../viewer/painters/contourPainter";
import { ErrorAlert, RunButton, Toggle, WarningList } from "../ui";

interface ContourPanelProps {
  filePath: string | null;
  overlayKey: string | null;
  imageWidth?: number;
  imageHeight?: number;
}

const DEFAULT_MODE: ContourMode = "sigma";
const DEFAULT_SIGMA_MULTIPLES = "1, 2, 3, 5, 10";
const DEFAULT_N_LEVELS = "5";
const DEFAULT_SMOOTH_SIGMA = "1";
const DEFAULT_COLOUR_MODE: ContourColourMode = "ramp";
const DEFAULT_LINE_WIDTH = "1";
const LINE_WIDTHS = ["1", "1.5", "2"] as const;
const COLOUR_MODES: readonly ContourColourMode[] = ["single", "ramp"];
const NOTICE_MS = 6000;
const HINT_TEXT =
  "Contours are traced on the image on screen; contours from another file need reprojection (not available).";

const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-teal-500/50 w-full";
const SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono w-full";
const LABEL_CLASS = "text-[9px] text-zinc-500 uppercase";
const SMALL_BUTTON_CLASS =
  "flex items-center gap-1 px-2 py-1 rounded text-[10px] border border-zinc-700/60 text-zinc-300 hover:bg-zinc-800/80 disabled:opacity-40 disabled:cursor-not-allowed";

const MODE_LABELS: Record<ContourMode, string> = {
  list: "Explicit list",
  linear: "Linear steps",
  log: "Log steps",
  sqrt: "Sqrt steps",
  sigma: "Sigma above sky",
};

function fmt(v: number, digits = 2): string {
  return Number.isFinite(v) ? v.toFixed(digits) : "--";
}

function ContourPanel({ filePath, overlayKey, imageWidth, imageHeight }: ContourPanelProps) {
  const modeId = useId();
  const levelsId = useId();
  const nLevelsId = useId();
  const loId = useId();
  const hiId = useId();
  const sigmaId = useId();
  const smoothId = useId();
  const binId = useId();
  const colourId = useId();
  const widthId = useId();
  const { excludeDq } = useDqContext();

  const [show, setShow] = useState(true);
  const [mode, setMode] = useState<ContourMode>(DEFAULT_MODE);
  const [levelsInput, setLevelsInput] = useState("");
  const [nLevelsText, setNLevelsText] = useState(DEFAULT_N_LEVELS);
  const [loText, setLoText] = useState("");
  const [hiText, setHiText] = useState("");
  const [sigmaText, setSigmaText] = useState(DEFAULT_SIGMA_MULTIPLES);
  const [smoothText, setSmoothText] = useState(DEFAULT_SMOOTH_SIGMA);
  const [binChoice, setBinChoice] = useState<ContourBinChoice>("auto");
  const [colourMode, setColourMode] = useState<ContourColourMode>(DEFAULT_COLOUR_MODE);
  const [lineWidthText, setLineWidthText] = useState<string>(DEFAULT_LINE_WIDTH);
  const [result, setResult] = useState<ContourResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [hidden, setHidden] = useState<ReadonlySet<number>>(() => new Set());
  const [highlightIndex, setHighlightIndex] = useState<number | null>(null);

  const requestSeqRef = useRef(0);
  const noticeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    requestSeqRef.current++;
    setResult(null);
    setError(null);
    setLoading(false);
    setHidden(new Set());
    setHighlightIndex(null);
    setNotice(null);
  }, [filePath]);

  useEffect(
    () => () => {
      if (noticeTimerRef.current) clearTimeout(noticeTimerRef.current);
    },
    [],
  );

  const showNotice = useCallback((text: string) => {
    setNotice(text);
    if (noticeTimerRef.current) clearTimeout(noticeTimerRef.current);
    noticeTimerRef.current = setTimeout(() => setNotice(null), NOTICE_MS);
  }, []);

  useEffect(() => {
    if (!overlayKey || !show || !result) return;
    overlayStore.add(overlayKey, {
      id: CONTOUR_LAYER_ID,
      kind: CONTOUR_LAYER_KIND,
      visible: true,
      paint: createContourPainter({
        levels: result.levels,
        hidden,
        highlightIndex,
        colourMode,
        lineWidth: Number(lineWidthText),
      }),
    });
    return () => overlayStore.remove(overlayKey, CONTOUR_LAYER_ID);
  }, [overlayKey, show, result, hidden, highlightIndex, colourMode, lineWidthText]);

  const run = useCallback(async () => {
    if (!filePath) return;
    const request = buildContourRequest({
      mode,
      levelsText: levelsInput,
      nLevelsText,
      loText,
      hiText,
      sigmaText,
      smoothText,
      binChoice,
      imageWidth: imageWidth ?? 0,
      imageHeight: imageHeight ?? 0,
      excludeDq,
    });
    if (!request.ok) {
      setError(request.error);
      return;
    }
    const seq = ++requestSeqRef.current;
    setLoading(true);
    setError(null);
    try {
      const next = await contourLines(filePath, request.options);
      if (requestSeqRef.current !== seq) return;
      setResult(next);
      setHidden(new Set());
      setHighlightIndex(null);
    } catch (e: unknown) {
      if (requestSeqRef.current === seq) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (requestSeqRef.current === seq) setLoading(false);
    }
  }, [filePath, mode, levelsInput, nLevelsText, loText, hiText, sigmaText, smoothText, binChoice, imageWidth, imageHeight, excludeDq]);

  const toggleHidden = useCallback((index: number) => {
    setHidden((prev) => {
      const next = new Set(prev);
      if (next.has(index)) next.delete(index);
      else next.add(index);
      return next;
    });
  }, []);

  const copyLevels = useCallback(async () => {
    if (!result) return;
    try {
      await navigator.clipboard.writeText(levelsText(result.levels.map((l) => l.value)));
      showNotice("Levels copied to the clipboard");
    } catch (e: unknown) {
      setError(`Clipboard copy failed: ${e instanceof Error ? e.message : String(e)}`);
    }
  }, [result, showNotice]);

  const toRegions = useCallback(() => {
    if (!overlayKey || !result) return;
    const pick = closedContourPolygons(result.levels, hidden, MAX_REGION_POLYGONS);
    for (const polygon of pick.polygons) {
      regionStore.add(overlayKey, {
        id: generateId(),
        shape: { shape: "polygon", points: polygon.points },
        props: { ...DEFAULT_REGION_PROPS, text: `c=${formatLevelValue(polygon.value)}` },
        backgroundId: null,
      });
    }
    showNotice(
      pick.skipped > 0
        ? `${pick.polygons.length} polygon regions added, ${pick.skipped} closed contours skipped (limit ${MAX_REGION_POLYGONS})`
        : `${pick.polygons.length} polygon regions added`,
    );
  }, [overlayKey, result, hidden, showNotice]);

  const autoBin = suggestBin(imageWidth ?? 0, imageHeight ?? 0);
  const generated = mode === "linear" || mode === "log" || mode === "sqrt";
  const closedCount = result ? result.levels.reduce((n, l) => n + l.closed.filter(Boolean).length, 0) : 0;

  return (
    <div className="ab-panel overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <Layers size={12} className="text-teal-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">Contours</span>
        </div>
        {loading && <Loader2 size={12} className="animate-spin text-teal-400/70" />}
      </div>

      <div className="px-3 py-2 space-y-2">
        <Toggle label="Show contours" checked={show} accent="teal" onChange={setShow} />

        <div className="flex flex-col gap-0.5">
          <label htmlFor={modeId} className={LABEL_CLASS}>
            Levels
          </label>
          <select id={modeId} value={mode} onChange={(e) => setMode(e.target.value as ContourMode)} className={SELECT_CLASS}>
            {CONTOUR_MODES.map((m) => (
              <option key={m} value={m}>
                {MODE_LABELS[m]}
              </option>
            ))}
          </select>
        </div>

        {mode === "list" && (
          <div className="flex flex-col gap-0.5">
            <label htmlFor={levelsId} className={LABEL_CLASS}>
              Level values
            </label>
            <textarea
              id={levelsId}
              rows={2}
              value={levelsInput}
              placeholder="e.g. 100, 200, 500"
              onChange={(e) => setLevelsInput(e.target.value)}
              className={INPUT_CLASS}
            />
          </div>
        )}

        {generated && (
          <div className="grid grid-cols-3 gap-2">
            <div className="flex flex-col gap-0.5">
              <label htmlFor={nLevelsId} className={LABEL_CLASS}>
                Levels
              </label>
              <input
                id={nLevelsId}
                type="number"
                min={1}
                max={32}
                step={1}
                value={nLevelsText}
                onChange={(e) => setNLevelsText(e.target.value)}
                className={INPUT_CLASS}
              />
            </div>
            <div className="flex flex-col gap-0.5">
              <label htmlFor={loId} className={LABEL_CLASS}>
                Low
              </label>
              <input id={loId} type="number" value={loText} onChange={(e) => setLoText(e.target.value)} className={INPUT_CLASS} />
            </div>
            <div className="flex flex-col gap-0.5">
              <label htmlFor={hiId} className={LABEL_CLASS}>
                High
              </label>
              <input id={hiId} type="number" value={hiText} onChange={(e) => setHiText(e.target.value)} className={INPUT_CLASS} />
            </div>
          </div>
        )}

        {mode === "sigma" && (
          <div className="flex flex-col gap-0.5">
            <label htmlFor={sigmaId} className={LABEL_CLASS}>
              Sigma multiples above the median
            </label>
            <input
              id={sigmaId}
              type="text"
              value={sigmaText}
              placeholder={DEFAULT_SIGMA_MULTIPLES}
              onChange={(e) => setSigmaText(e.target.value)}
              className={INPUT_CLASS}
            />
          </div>
        )}

        <div className="grid grid-cols-2 gap-2">
          <div className="flex flex-col gap-0.5">
            <label htmlFor={smoothId} className={LABEL_CLASS}>
              Smoothing sigma (px)
            </label>
            <input
              id={smoothId}
              type="number"
              min={0}
              max={20}
              step={0.5}
              value={smoothText}
              onChange={(e) => setSmoothText(e.target.value)}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={binId} className={LABEL_CLASS}>
              Bin
            </label>
            <select id={binId} value={binChoice} onChange={(e) => setBinChoice(e.target.value as ContourBinChoice)} className={SELECT_CLASS}>
              {CONTOUR_BIN_CHOICES.map((b) => (
                <option key={b} value={b}>
                  {b === "auto" ? `auto (${autoBin})` : b}
                </option>
              ))}
            </select>
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={colourId} className={LABEL_CLASS}>
              Colour
            </label>
            <select id={colourId} value={colourMode} onChange={(e) => setColourMode(e.target.value as ContourColourMode)} className={SELECT_CLASS}>
              {COLOUR_MODES.map((c) => (
                <option key={c} value={c}>
                  {c === "single" ? "single (teal)" : "ramp (blue to amber)"}
                </option>
              ))}
            </select>
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={widthId} className={LABEL_CLASS}>
              Width (px)
            </label>
            <select id={widthId} value={lineWidthText} onChange={(e) => setLineWidthText(e.target.value)} className={SELECT_CLASS}>
              {LINE_WIDTHS.map((w) => (
                <option key={w} value={w}>
                  {w}
                </option>
              ))}
            </select>
          </div>
        </div>

        <RunButton
          label="Compute"
          runningLabel="Tracing contours..."
          running={loading}
          disabled={!filePath || loading}
          accent="teal"
          icon={<Layers size={12} />}
          onClick={() => void run()}
        />

        <ErrorAlert message={error} />

        {result && (
          <>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5 text-[10px]">
              <div className="text-zinc-500">Background and trace</div>
              <div className="text-zinc-300 font-mono">
                background {fmt(result.background_median, 3)} +/- {fmt(result.background_sigma, 3)}, bin {result.bin},{" "}
                {result.n_points} points, {result.elapsed_ms} ms{result.masked ? ", DQ excluded" : ""}
              </div>
            </div>

            <WarningList warnings={result.notes} />

            <div className="flex flex-wrap gap-1">
              {result.levels.map((level, index) => {
                const isHidden = hidden.has(index);
                const colour = levelColour(index, result.levels.length, colourMode);
                return (
                  <button
                    key={index}
                    type="button"
                    onClick={() => toggleHidden(index)}
                    onMouseEnter={() => setHighlightIndex(index)}
                    onMouseLeave={() => setHighlightIndex((current) => (current === index ? null : current))}
                    title={isHidden ? "Show this level" : "Hide this level"}
                    className={`flex items-center gap-1 px-1.5 py-0.5 rounded border text-[9px] font-mono ${
                      isHidden ? "border-zinc-800 text-zinc-600" : "border-zinc-700/60 text-zinc-200"
                    } ${highlightIndex === index ? "bg-zinc-800/80" : "bg-zinc-900/60"}`}
                  >
                    <span className="inline-block w-2 h-2 rounded-sm" style={{ background: isHidden ? "transparent" : colour, border: `1px solid ${colour}` }} />
                    {formatLevelValue(level.value)}
                    <span className="text-zinc-500">({level.polylines.length})</span>
                    {isHidden ? <EyeOff size={9} /> : <Eye size={9} />}
                  </button>
                );
              })}
            </div>

            <div className="flex flex-wrap items-center gap-1.5">
              <button type="button" onClick={() => void copyLevels()} className={SMALL_BUTTON_CLASS} title="Copy the level values as text">
                <ClipboardCopy size={10} />
                Copy levels
              </button>
              <button
                type="button"
                onClick={toRegions}
                disabled={!overlayKey || closedCount === 0}
                className={SMALL_BUTTON_CLASS}
                title={`Add the closed contours of the visible levels as Polygon regions (at most ${MAX_REGION_POLYGONS})`}
              >
                <Shapes size={10} />
                To regions
              </button>
            </div>
          </>
        )}

        {notice && <div className="text-[9px] text-emerald-400/90">{notice}</div>}

        <div className="text-[10px] text-zinc-600">{HINT_TEXT}</div>
      </div>
    </div>
  );
}

export default memo(ContourPanel);
