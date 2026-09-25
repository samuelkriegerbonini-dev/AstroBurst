import { memo, useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { ClipboardCopy, Download, Loader2, Shapes, Aperture } from "lucide-react";
import { measurePhotometryBatch } from "../../services/analysis";
import type { BatchPhotometryResult } from "../../services/analysis";
import { useDqContext } from "../../context/PreviewContext";
import { useRegionDoc } from "../../hooks/useRegionStore";
import { overlayStore } from "../../utils/overlayStore";
import { regionStore } from "../../utils/regionStore";
import { DEFAULT_REGION_PROPS } from "../../utils/regionPersistence";
import { generateId } from "../../utils/format";
import {
  flagDuplicates,
  medianSnr,
  parsePositions,
  photometryCsvFileName,
  photometryTableCsv,
  sortRows,
  type PhotometryColumnKey,
  type PhotometryTableRow,
  type SortDirection,
} from "../../utils/photometryTable";
import { APERTURE_LAYER_ID, APERTURE_LAYER_KIND, createAperturePainter, type ApertureMarker } from "../viewer/painters/aperturePainter";
import { ErrorAlert, RunButton, Toggle, WarningList } from "../ui";
import MeasurementBadge from "./MeasurementBadge";
import type { Star } from "./PlateSolvePanel";

interface PhotometryTablePanelProps {
  filePath: string | null;
  overlayKey: string | null;
  stars: Star[];
}

type PositionSource = "stars" | "regions" | "pasted";

interface BatchRun {
  result: BatchPhotometryResult;
  labels: string[];
}

interface TableColumn {
  key: PhotometryColumnKey;
  header: string;
  digits: number;
}

const DEFAULT_APERTURE_RADIUS_PX = "5";
const TABLE_ROW_LIMIT = 200;
const DUPLICATE_TOLERANCE_PX = 1;
const SAVED_NOTICE_MS = 6000;
const PASTE_ROWS = 4;
const ANNULUS_NEEDS_BOTH = "sky annulus needs both an inner and an outer radius";
const SEMANTICS_TEXT =
  "Each position snaps to the brightest pixel within 8 px, so two inputs on one star measure the same star (flagged as duplicates). The aperture radius is fixed for every source (leave it blank for 1.5 x FWHM per star). When the curve of growth has no plateau before the sky annulus, the correction, total flux and EE radii are empty.";

const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-amber-500/50 w-full";
const SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono w-full";
const SMALL_BUTTON_CLASS =
  "flex items-center gap-1 px-2 py-1 rounded text-[10px] border border-zinc-700/60 text-zinc-300 hover:bg-zinc-800/80 disabled:opacity-40 disabled:cursor-not-allowed";

const TABLE_COLUMNS: TableColumn[] = [
  { key: "label", header: "label", digits: 0 },
  { key: "x", header: "x", digits: 1 },
  { key: "y", header: "y", digits: 1 },
  { key: "net_flux", header: "flux", digits: 1 },
  { key: "snr", header: "SNR", digits: 1 },
  { key: "mag_ab", header: "AB", digits: 2 },
  { key: "mag_inst", header: "inst", digits: 2 },
  { key: "fwhm", header: "FWHM", digits: 2 },
  { key: "ee50_radius", header: "EE50", digits: 2 },
  { key: "aperture_correction", header: "corr", digits: 3 },
];

function parseOptionalNumber(text: string): number | undefined {
  const v = parseFloat(text);
  return Number.isFinite(v) && v > 0 ? v : undefined;
}

function fmt(v: number | null | undefined, digits: number): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return "--";
  return Math.abs(v) >= 1e6 ? v.toExponential(2) : v.toFixed(digits);
}

function cellText(row: PhotometryTableRow, column: TableColumn): string {
  if (column.key === "label") return row.label;
  const p = row.photometry;
  if (!p) return "--";
  switch (column.key) {
    case "x":
      return fmt(p.x, column.digits);
    case "y":
      return fmt(p.y, column.digits);
    case "net_flux":
      return fmt(p.net_flux, column.digits);
    case "snr":
      return fmt(p.snr, column.digits);
    case "mag_ab":
      return fmt(p.mag_ab, column.digits);
    case "mag_inst":
      return fmt(p.mag_inst, column.digits);
    case "fwhm":
      return fmt(p.fwhm, column.digits);
    case "ee50_radius":
      return fmt(p.ee50_radius, column.digits);
    case "aperture_correction":
      return fmt(p.aperture_correction, column.digits);
    default:
      return "--";
  }
}

function rowFlags(row: PhotometryTableRow, duplicate: boolean): string {
  if (row.error) return row.error;
  const p = row.photometry;
  const flags: string[] = [];
  if (duplicate) flags.push("dup");
  if (p?.saturated) flags.push("sat");
  if (p && p.n_masked > 0) flags.push(`${p.n_masked} masked`);
  if (p && p.aperture_correction == null) flags.push("no plateau");
  return flags.join(", ");
}

function PhotometryTablePanel({ filePath, overlayKey, stars }: PhotometryTablePanelProps) {
  const sourceId = useId();
  const apertureId = useId();
  const skyInId = useId();
  const skyOutId = useId();
  const gainId = useId();
  const pasteId = useId();
  const [source, setSource] = useState<PositionSource>("stars");
  const [apertureText, setApertureText] = useState(DEFAULT_APERTURE_RADIUS_PX);
  const [skyInText, setSkyInText] = useState("");
  const [skyOutText, setSkyOutText] = useState("");
  const [gainText, setGainText] = useState("");
  const [withGrowthCurve, setWithGrowthCurve] = useState(false);
  const [pastedText, setPastedText] = useState("");
  const [run, setRun] = useState<BatchRun | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedPath, setSavedPath] = useState<string | null>(null);
  const [sortKey, setSortKey] = useState<PhotometryColumnKey>("index");
  const [sortDir, setSortDir] = useState<SortDirection>("asc");
  const [highlightIndex, setHighlightIndex] = useState<number | null>(null);
  const { excludeDq } = useDqContext();
  const regionDoc = useRegionDoc(overlayKey);
  const requestSeqRef = useRef(0);

  useEffect(() => {
    requestSeqRef.current++;
    setRun(null);
    setError(null);
    setRunning(false);
    setHighlightIndex(null);
    setSavedPath(null);
  }, [filePath]);

  const pointRegions = useMemo(() => regionDoc.regions.filter((r) => r.shape.shape === "point"), [regionDoc]);
  const pasted = useMemo(() => parsePositions(pastedText), [pastedText]);

  const input = useMemo((): { points: [number, number][]; labels: string[] } => {
    if (source === "stars") {
      return { points: stars.map((s) => [s.x, s.y]), labels: stars.map((_, i) => `star ${i + 1}`) };
    }
    if (source === "regions") {
      const points: [number, number][] = [];
      const labels: string[] = [];
      for (const region of pointRegions) {
        if (region.shape.shape !== "point") continue;
        points.push([region.shape.x, region.shape.y]);
        labels.push(region.props.text || `point ${labels.length + 1}`);
      }
      return { points, labels };
    }
    return { points: pasted.points, labels: pasted.points.map((_, i) => `p${i + 1}`) };
  }, [source, stars, pointRegions, pasted]);

  const apertureRadius = parseOptionalNumber(apertureText);
  const annulusInner = parseOptionalNumber(skyInText);
  const annulusOuter = parseOptionalNumber(skyOutText);
  const gain = parseOptionalNumber(gainText);
  const annulusHalfFilled = (annulusInner === undefined) !== (annulusOuter === undefined);

  const measure = useCallback(async () => {
    if (!filePath || input.points.length === 0) return;
    const seq = ++requestSeqRef.current;
    const labels = input.labels;
    setRunning(true);
    setError(null);
    setHighlightIndex(null);
    try {
      const result = await measurePhotometryBatch(filePath, input.points, {
        apertureRadius,
        annulusInner,
        annulusOuter,
        gain,
        excludeDq,
        withGrowthCurve,
      });
      if (requestSeqRef.current !== seq) return;
      setRun({ result, labels });
    } catch (e: unknown) {
      if (requestSeqRef.current === seq) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (requestSeqRef.current === seq) setRunning(false);
    }
  }, [filePath, input, apertureRadius, annulusInner, annulusOuter, gain, excludeDq, withGrowthCurve]);

  const rows = useMemo(
    (): PhotometryTableRow[] =>
      run
        ? run.result.rows.map((r) => ({
            index: r.index,
            label: run.labels[r.index] ?? String(r.index),
            photometry: r.photometry,
            sky: r.sky,
            error: r.error,
          }))
        : [],
    [run],
  );
  const duplicates = useMemo(() => flagDuplicates(rows, DUPLICATE_TOLERANCE_PX), [rows]);
  const sorted = useMemo(() => sortRows(rows, sortKey, sortDir), [rows, sortKey, sortDir]);
  const shown = useMemo(() => sorted.slice(0, TABLE_ROW_LIMIT), [sorted]);
  const median = useMemo(() => medianSnr(rows), [rows]);

  useEffect(() => {
    if (!overlayKey || rows.length === 0) return;
    const apertures: ApertureMarker[] = [];
    const indices: number[] = [];
    for (const row of rows) {
      const p = row.photometry;
      if (!p) continue;
      apertures.push({ x: p.x, y: p.y, rAp: p.aperture_radius, skyIn: p.sky_inner, skyOut: p.sky_outer, label: row.label });
      indices.push(row.index);
    }
    const highlight = highlightIndex === null ? -1 : indices.indexOf(highlightIndex);
    overlayStore.add(overlayKey, {
      id: APERTURE_LAYER_ID,
      kind: APERTURE_LAYER_KIND,
      visible: true,
      paint: createAperturePainter({ apertures, highlightIndex: highlight >= 0 ? highlight : null }),
    });
    return () => overlayStore.remove(overlayKey, APERTURE_LAYER_ID);
  }, [overlayKey, rows, highlightIndex]);

  const toggleSort = useCallback(
    (key: PhotometryColumnKey) => {
      if (key === sortKey) setSortDir((d) => (d === "asc" ? "desc" : "asc"));
      else {
        setSortKey(key);
        setSortDir("asc");
      }
    },
    [sortKey],
  );

  const copyCsv = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(photometryTableCsv(rows));
    } catch (e: unknown) {
      setError(`Clipboard copy failed: ${e instanceof Error ? e.message : String(e)}`);
    }
  }, [rows]);

  const saveCsv = useCallback(async () => {
    if (!filePath || rows.length === 0) return;
    setError(null);
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const target = await save({
        defaultPath: photometryCsvFileName(filePath),
        filters: [{ name: "CSV", extensions: ["csv"] }],
        title: "Save photometry table",
      });
      if (!target) return;
      const { writeTextFile } = await import("@tauri-apps/plugin-fs");
      await writeTextFile(target, photometryTableCsv(rows));
      setSavedPath(target);
      setTimeout(() => setSavedPath(null), SAVED_NOTICE_MS);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [filePath, rows]);

  const addAsRegions = useCallback(() => {
    if (!overlayKey) return;
    for (const row of rows) {
      const p = row.photometry;
      if (!p) continue;
      regionStore.add(overlayKey, {
        id: generateId(),
        shape: { shape: "point", x: p.x, y: p.y },
        props: { ...DEFAULT_REGION_PROPS, text: row.label },
        backgroundId: null,
      });
    }
  }, [overlayKey, rows]);

  const result = run?.result ?? null;
  const calibration = result ? (result.photcal ? result.photcal.label : "uncalibrated") : null;
  const canMeasure = !!filePath && input.points.length > 0 && !annulusHalfFilled;

  return (
    <div className="ab-panel overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <Aperture size={12} className="text-amber-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">Photometry table</span>
          <MeasurementBadge />
        </div>
        {running && <Loader2 size={12} className="animate-spin text-amber-400/70" />}
      </div>

      <div className="px-3 py-2 space-y-2">
        <div className="flex flex-col gap-0.5">
          <label htmlFor={sourceId} className="text-[9px] text-zinc-500 uppercase">
            Positions
          </label>
          <select id={sourceId} value={source} onChange={(e) => setSource(e.target.value as PositionSource)} className={SELECT_CLASS}>
            <option value="stars">Detected stars ({stars.length})</option>
            <option value="regions">Point regions ({pointRegions.length})</option>
            <option value="pasted">Pasted positions</option>
          </select>
        </div>

        {source === "pasted" && (
          <div className="flex flex-col gap-0.5">
            <label htmlFor={pasteId} className="text-[9px] text-zinc-500 uppercase">
              x y per line (0-based pixels, # comments)
            </label>
            <textarea
              id={pasteId}
              rows={PASTE_ROWS}
              value={pastedText}
              placeholder={"120.5 88.2\n301, 240"}
              onChange={(e) => setPastedText(e.target.value)}
              className={INPUT_CLASS}
              spellCheck={false}
            />
            {pasted.errors.length > 0 && (
              <div className="text-[9px] text-red-400 space-y-0.5">
                {pasted.errors.slice(0, 5).map((msg) => (
                  <div key={msg}>{msg}</div>
                ))}
                {pasted.errors.length > 5 && <div>{pasted.errors.length - 5} more</div>}
              </div>
            )}
          </div>
        )}

        <div className="grid grid-cols-4 gap-1.5">
          <div className="flex flex-col gap-0.5">
            <label htmlFor={apertureId} className="text-[9px] text-zinc-500">
              r_ap px
            </label>
            <input
              id={apertureId}
              type="number"
              min={1}
              max={60}
              step={0.5}
              value={apertureText}
              placeholder="auto 1.5 x FWHM"
              onChange={(e) => setApertureText(e.target.value)}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={skyInId} className="text-[9px] text-zinc-500">
              sky in px
            </label>
            <input
              id={skyInId}
              type="number"
              min={1}
              step={0.5}
              value={skyInText}
              placeholder="2 x r_ap"
              onChange={(e) => setSkyInText(e.target.value)}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={skyOutId} className="text-[9px] text-zinc-500">
              sky out px
            </label>
            <input
              id={skyOutId}
              type="number"
              min={1}
              step={0.5}
              value={skyOutText}
              placeholder="3 x r_ap"
              onChange={(e) => setSkyOutText(e.target.value)}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={gainId} className="text-[9px] text-zinc-500">
              gain e-/ADU
            </label>
            <input
              id={gainId}
              type="number"
              min={0.01}
              step={0.1}
              value={gainText}
              placeholder="none"
              onChange={(e) => setGainText(e.target.value)}
              className={INPUT_CLASS}
            />
          </div>
        </div>
        {annulusHalfFilled && <div className="text-[9px] text-amber-400/90">{ANNULUS_NEEDS_BOTH}</div>}

        <Toggle label="Growth curves" checked={withGrowthCurve} accent="amber" onChange={setWithGrowthCurve} />

        <RunButton
          label={`Measure ${input.points.length}`}
          runningLabel="Measuring..."
          running={running}
          disabled={!canMeasure}
          accent="amber"
          icon={<Aperture size={12} />}
          onClick={() => void measure()}
        />

        <ErrorAlert message={error} />

        {result && (
          <div className="grid grid-cols-2 gap-1.5 text-[10px]">
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">Measured / failed</div>
              <div className="text-amber-300 font-mono">
                {result.n_measured} / {result.n_failed}
                <span className="text-zinc-500"> in {result.elapsed_ms} ms</span>
              </div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">Calibration</div>
              <div className={`font-mono ${result.photcal ? "text-zinc-300" : "text-amber-400/90"}`}>{calibration}</div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">Median SNR</div>
              <div className="text-amber-300 font-mono">{fmt(median, 1)}</div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">Duplicates (within {DUPLICATE_TOLERANCE_PX} px)</div>
              <div className={`font-mono ${duplicates.size > 0 ? "text-amber-400/90" : "text-zinc-300"}`}>{duplicates.size}</div>
            </div>
          </div>
        )}

        <WarningList warnings={result?.warnings} />

        {shown.length > 0 && (
          <div className="max-h-56 overflow-auto border border-zinc-800/60 rounded">
            <table className="w-full text-[9px] font-mono">
              <thead className="sticky top-0 bg-zinc-900 text-zinc-500">
                <tr>
                  {TABLE_COLUMNS.map((column) => (
                    <th
                      key={column.key}
                      onClick={() => toggleSort(column.key)}
                      className={`${column.key === "label" ? "text-left" : "text-right"} px-1.5 py-0.5 cursor-pointer select-none hover:text-zinc-300`}
                      title={`Sort by ${column.header}`}
                    >
                      {column.header}
                      {sortKey === column.key ? (sortDir === "asc" ? " ^" : " v") : ""}
                    </th>
                  ))}
                  <th className="text-left px-1.5 py-0.5">flags</th>
                </tr>
              </thead>
              <tbody>
                {shown.map((row) => {
                  const duplicate = duplicates.has(row.index);
                  const tone = row.error ? "text-red-300" : duplicate ? "text-amber-200" : "text-zinc-300";
                  return (
                    <tr
                      key={row.index}
                      onMouseEnter={() => setHighlightIndex(row.index)}
                      onMouseLeave={() => setHighlightIndex(null)}
                      className={`${tone} ${highlightIndex === row.index ? "bg-zinc-800/80" : ""}`}
                    >
                      {TABLE_COLUMNS.map((column) => (
                        <td
                          key={column.key}
                          className={`${column.key === "label" ? "text-left truncate max-w-[90px]" : "text-right"} px-1.5 py-0.5`}
                          title={column.key === "label" ? row.label : undefined}
                        >
                          {cellText(row, column)}
                        </td>
                      ))}
                      <td className="text-left px-1.5 py-0.5 truncate max-w-[140px]" title={rowFlags(row, duplicate)}>
                        {rowFlags(row, duplicate)}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
            {rows.length > TABLE_ROW_LIMIT && (
              <div className="text-[9px] text-zinc-600 px-1.5 py-0.5">
                showing {TABLE_ROW_LIMIT} of {rows.length}; the CSV holds every row
              </div>
            )}
          </div>
        )}

        {rows.length > 0 && (
          <div className="flex flex-wrap items-center gap-1.5">
            <button type="button" onClick={() => void copyCsv()} className={SMALL_BUTTON_CLASS}>
              <ClipboardCopy size={10} />
              Copy CSV
            </button>
            <button type="button" onClick={() => void saveCsv()} disabled={!filePath} className={SMALL_BUTTON_CLASS}>
              <Download size={10} />
              Save CSV
            </button>
            <button
              type="button"
              onClick={addAsRegions}
              disabled={!overlayKey || (result?.n_measured ?? 0) === 0}
              className={SMALL_BUTTON_CLASS}
              title="Add every measured centroid as a Point region named by its label"
            >
              <Shapes size={10} />
              Add as Point regions
            </button>
          </div>
        )}

        {savedPath && <div className="text-[9px] text-emerald-400/90 break-all">Saved {savedPath}</div>}

        {!result && !error && <div className="text-[10px] text-zinc-600">{SEMANTICS_TEXT}</div>}
      </div>
    </div>
  );
}

export default memo(PhotometryTablePanel);
