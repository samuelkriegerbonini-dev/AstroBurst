import { useState, useEffect, useRef, useCallback, useMemo, useId, memo } from "react";
import { Crosshair, Grid3X3, Loader2 } from "lucide-react";
import { pixelTable } from "../../services/analysis";
import type { PixelTableResult } from "../../shared/types/analysis";
import { useMousePixel, usePixelClick } from "../../hooks/useMousePixelStore";
import { Toggle } from "../ui";
import MeasurementBadge from "./MeasurementBadge";
import {
  DEFAULT_PIXEL_TABLE_SIZE,
  PIXEL_TABLE_SIZES,
  cellTone,
  columnIndices,
  displayedPlane,
  formatCell,
  pixelTableCsv,
  rowIndices,
  type CellTone,
} from "../../utils/pixelTable";

interface PixelTablePanelProps {
  filePath: string | null;
  measureKey?: string | null;
}

const FOLLOW_DEBOUNCE_MS = 80;
const NOTICE_MS = 1500;
const CELL_DIGITS = 3;
const SUMMARY_DIGITS = 4;
const SELECT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-1.5 py-0.5 text-[10px] text-zinc-200 font-mono focus:border-sky-500/50";
const BUTTON_CLASS =
  "text-[10px] px-2 py-0.5 rounded border border-zinc-700/50 bg-zinc-900 text-zinc-300 hover:border-sky-500/50 disabled:opacity-40 disabled:cursor-default";
const CELL_BASE = "px-1 py-0.5 text-right whitespace-nowrap border border-zinc-800/60";
const CELL_TONE_CLASS: Record<CellTone, string> = {
  normal: "text-zinc-300",
  max: "text-amber-200 bg-amber-900/40",
  min: "text-sky-200 bg-sky-900/30",
  nan: "text-zinc-600 bg-zinc-900/60",
  dq: "text-red-200 bg-red-900/40",
};
const CENTRE_CLASS = "outline outline-1 outline-sky-400 -outline-offset-1";
const AXIS_CLASS = "px-1 py-0.5 text-zinc-500 whitespace-nowrap";
const CENTRE_AXIS_CLASS = "px-1 py-0.5 text-sky-400 whitespace-nowrap";

function parseSize(text: string): number {
  const n = Number(text);
  return (PIXEL_TABLE_SIZES as readonly number[]).includes(n) ? n : DEFAULT_PIXEL_TABLE_SIZE;
}

function cellTitle(x: number, y: number, value: number | null, err: number | null | undefined, dqNames: string | null, unit: string | null): string {
  const parts = [`(${x}, ${y}): ${formatCell(value, SUMMARY_DIGITS)}${unit && value !== null ? ` ${unit}` : ""}`];
  if (err !== undefined) parts.push(`ERR ${formatCell(err, SUMMARY_DIGITS)}`);
  if (dqNames) parts.push(`DQ ${dqNames}`);
  return parts.join("\n");
}

function PixelTablePanel({ filePath, measureKey }: PixelTablePanelProps) {
  const sizeId = useId();
  const [armed, setArmed] = useState(false);
  const [follow, setFollow] = useState(false);
  const [showErr, setShowErr] = useState(false);
  const [sizeText, setSizeText] = useState(String(DEFAULT_PIXEL_TABLE_SIZE));
  const [result, setResult] = useState<PixelTableResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const click = usePixelClick();
  const mouse = useMousePixel();
  const requestSeqRef = useRef(0);
  const busyRef = useRef(false);
  const pendingRef = useRef<{ x: number; y: number } | null>(null);
  const centreRef = useRef<{ x: number; y: number } | null>(null);
  const lastSeqRef = useRef(0);
  const clickRef = useRef(click);
  clickRef.current = click;
  const fetchRef = useRef<(x: number, y: number) => void>(() => {});
  const noticeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const size = parseSize(sizeText);

  useEffect(() => {
    requestSeqRef.current++;
    lastSeqRef.current = clickRef.current?.seq ?? lastSeqRef.current;
    busyRef.current = false;
    pendingRef.current = null;
    centreRef.current = null;
    setResult(null);
    setError(null);
    setLoading(false);
  }, [filePath, measureKey]);

  useEffect(() => {
    return () => {
      if (noticeTimerRef.current) clearTimeout(noticeTimerRef.current);
    };
  }, []);

  const showNotice = useCallback((text: string) => {
    setNotice(text);
    if (noticeTimerRef.current) clearTimeout(noticeTimerRef.current);
    noticeTimerRef.current = setTimeout(() => setNotice(null), NOTICE_MS);
  }, []);

  const fetchTable = useCallback(
    async (x: number, y: number) => {
      if (!filePath) return;
      if (busyRef.current) {
        pendingRef.current = { x, y };
        return;
      }
      const seq = ++requestSeqRef.current;
      busyRef.current = true;
      centreRef.current = { x, y };
      setLoading(true);
      try {
        const res = await pixelTable(filePath, x, y, size);
        if (requestSeqRef.current !== seq) return;
        setResult(res);
        setError(null);
      } catch (e: unknown) {
        if (requestSeqRef.current === seq) setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (requestSeqRef.current === seq) {
          busyRef.current = false;
          setLoading(false);
          const next = pendingRef.current;
          pendingRef.current = null;
          if (next) fetchRef.current(next.x, next.y);
        }
      }
    },
    [filePath, size],
  );
  fetchRef.current = fetchTable;

  useEffect(() => {
    if (!armed || !click || click.seq === lastSeqRef.current) return;
    lastSeqRef.current = click.seq;
    fetchTable(click.x, click.y);
  }, [armed, click, fetchTable]);

  useEffect(() => {
    if (!follow || !mouse) return;
    const { x, y } = mouse;
    const timer = setTimeout(() => fetchTable(x, y), FOLLOW_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [follow, mouse, fetchTable]);

  useEffect(() => {
    const centre = centreRef.current;
    if (centre) fetchRef.current(centre.x, centre.y);
  }, [size]);

  const shown = useMemo(() => (result ? displayedPlane(result, showErr) : null), [result, showErr]);

  const copyCsv = useCallback(async () => {
    if (!result || !shown) return;
    try {
      await navigator.clipboard.writeText(pixelTableCsv(result, shown.grid));
      showNotice(`${shown.plane === "ERR" ? "ERR" : "Pixel"} table copied to the clipboard as CSV`);
    } catch (e: unknown) {
      setError(`Clipboard copy failed: ${e instanceof Error ? e.message : String(e)}`);
    }
  }, [result, shown, showNotice]);

  const cols = useMemo(() => (result ? columnIndices(result) : []), [result]);
  const rows = useMemo(() => (result ? rowIndices(result) : []), [result]);
  const half = result ? Math.floor(result.size / 2) : 0;
  const grid = shown?.grid ?? [];
  const stats = shown?.stats;
  const plane = shown?.plane;

  return (
    <div className="ab-panel overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <Grid3X3 size={12} className="text-sky-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">Pixel table</span>
          <MeasurementBadge />
        </div>
        {loading && <Loader2 size={12} className="animate-spin text-sky-400/70" />}
      </div>

      <div className="px-3 py-2 space-y-2">
        <Toggle label="Read on image click" checked={armed} accent="sky" onChange={setArmed} />
        <Toggle label="Follow cursor" checked={follow} accent="sky" onChange={setFollow} />

        <div className="flex items-center justify-between gap-2">
          <label htmlFor={sizeId} className="text-xs text-zinc-400">
            Grid size
          </label>
          <select id={sizeId} value={sizeText} onChange={(e) => setSizeText(e.target.value)} className={SELECT_CLASS}>
            {PIXEL_TABLE_SIZES.map((s) => (
              <option key={s} value={s}>
                {s} x {s}
              </option>
            ))}
          </select>
        </div>

        {armed && (
          <div className="flex items-center gap-1.5 text-[10px] text-sky-400/70">
            <Crosshair size={10} />
            <span>Enable crosshair mode in the viewer toolbar, then click a pixel.</span>
          </div>
        )}

        {error && (
          <div className="text-[10px] text-red-400 bg-red-900/20 border border-red-800/30 rounded px-2.5 py-1.5 break-words">
            {error}
          </div>
        )}

        {result && stats && (
          <div className="flex flex-col gap-1.5">
            <div className="overflow-x-auto">
              <table className="font-mono text-[9px] border-collapse">
                <thead>
                  <tr>
                    <th className={AXIS_CLASS}>y \ x</th>
                    {cols.map((x) => (
                      <th key={x} className={x === result.x ? CENTRE_AXIS_CLASS : AXIS_CLASS}>
                        {x}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {rows.map((y, r) => (
                    <tr key={y}>
                      <td className={y === result.y ? CENTRE_AXIS_CLASS : AXIS_CLASS}>{y}</td>
                      {cols.map((x, c) => {
                        const v = grid[r]?.[c] ?? null;
                        const dqNames = result.dq_names?.[r]?.[c] ?? null;
                        const tone = cellTone(v, stats, dqNames);
                        const centre = r === half && c === half;
                        return (
                          <td
                            key={x}
                            title={cellTitle(x, y, result.values[r]?.[c] ?? null, result.err ? result.err[r]?.[c] ?? null : undefined, dqNames, result.unit)}
                            className={`${CELL_BASE} ${CELL_TONE_CLASS[tone]}${centre ? ` ${CENTRE_CLASS}` : ""}`}
                          >
                            {formatCell(v, CELL_DIGITS)}
                          </td>
                        );
                      })}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>

            <div className="text-[9px] text-zinc-500 font-mono flex flex-wrap gap-x-2">
              {result.err != null && <span>{plane}</span>}
              {result.unit && <span>unit {result.unit}</span>}
              <span>min {formatCell(stats.min, SUMMARY_DIGITS)}</span>
              <span>max {formatCell(stats.max, SUMMARY_DIGITS)}</span>
              <span>mean {formatCell(stats.mean, SUMMARY_DIGITS)}</span>
              <span>median {formatCell(stats.median, SUMMARY_DIGITS)}</span>
              <span>
                {stats.n_finite} finite{stats.n_nan > 0 ? `, ${stats.n_nan} NaN` : ""}
              </span>
              {result.dq_table && <span>DQ {result.dq_table}</span>}
              <span>{result.elapsed_ms} ms</span>
            </div>

            <div className="flex items-center justify-between gap-2">
              <Toggle label="Show ERR" checked={showErr} disabled={result.err == null} accent="sky" onChange={setShowErr} />
              <button type="button" onClick={copyCsv} className={BUTTON_CLASS}>
                Copy CSV
              </button>
            </div>
            {notice && <div className="text-[9px] text-emerald-400/90">{notice}</div>}
          </div>
        )}

        {!result && !error && (
          <div className="text-[10px] text-zinc-600">
            Raw pixel values around a clicked or hovered pixel, with the ERR value and the decoded DQ flags per
            cell, so cosmic rays, saturated cores, snowballs and DQ bit combinations can be judged by reading the
            numbers.
          </div>
        )}
      </div>
    </div>
  );
}

export default memo(PixelTablePanel);
