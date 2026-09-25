import { memo, useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { FileUp, Loader2, Pin, Shapes } from "lucide-react";
import { worldToPixel } from "../../services/astrometry";
import type { SkyFrame, WorldToPixelResult } from "../../shared/types/astrometry";
import { overlayStore } from "../../utils/overlayStore";
import { regionStore } from "../../utils/regionStore";
import { DEFAULT_REGION_PROPS } from "../../utils/regionPersistence";
import { generateId } from "../../utils/format";
import { parseSkyCsv, type SkyTarget } from "../../utils/skyList";
import { TARGET_LAYER_ID, TARGET_LAYER_KIND, createTargetPainter, type PlacedTarget } from "../viewer/painters/targetPainter";
import { ErrorAlert, RunButton, Toggle } from "../ui";

interface TargetsPanelProps {
  filePath: string | null;
}

interface PlacedRow {
  index: number;
  target: SkyTarget;
  x: number | null;
  y: number | null;
  onImage: boolean;
}

const FRAMES: readonly SkyFrame[] = ["icrs", "fk5", "galactic", "ecliptic"];
const TABLE_ROW_LIMIT = 200;
const TEXTAREA_ROWS = 4;
const DEG_DIGITS = 5;
const PIXEL_DIGITS = 1;
const LIST_PLACEHOLDER = "10:45:03.6 -59:41:04 label";

const TEXTAREA_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-cyan-500/50 w-full resize-y";
const SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono";
const SMALL_BUTTON_CLASS =
  "flex items-center gap-1 px-2 py-1 rounded text-[10px] border border-zinc-700/60 text-zinc-300 hover:bg-zinc-800/80 disabled:opacity-40 disabled:cursor-not-allowed";

function fmt(v: number | null, digits: number): string {
  return v === null || !Number.isFinite(v) ? "--" : v.toFixed(digits);
}

function placeRows(targets: SkyTarget[], result: WorldToPixelResult | null): PlacedRow[] {
  return targets.map((target, index) => {
    const point = result?.points[index] ?? null;
    return {
      index,
      target,
      x: point ? point[0] : null,
      y: point ? point[1] : null,
      onImage: result?.on_image[index] ?? false,
    };
  });
}

function TargetsPanel({ filePath }: TargetsPanelProps) {
  const textId = useId();
  const frameId = useId();
  const [text, setText] = useState("");
  const [frame, setFrame] = useState<SkyFrame>("icrs");
  const [placed, setPlaced] = useState<{ targets: SkyTarget[]; result: WorldToPixelResult } | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [markers, setMarkers] = useState(true);
  const [hoverIndex, setHoverIndex] = useState<number | null>(null);
  const requestSeqRef = useRef(0);

  useEffect(() => {
    requestSeqRef.current++;
    setPlaced(null);
    setError(null);
    setLoading(false);
    setHoverIndex(null);
  }, [filePath]);

  const parsed = useMemo(() => (text.trim() ? parseSkyCsv(text) : { targets: [], errors: [] }), [text]);

  const rows = useMemo(() => (placed ? placeRows(placed.targets, placed.result) : []), [placed]);
  const onImageRows = useMemo(() => rows.filter((r) => r.onImage && r.x !== null && r.y !== null), [rows]);
  const tableRows = useMemo(() => rows.slice(0, TABLE_ROW_LIMIT), [rows]);

  const painted = useMemo<PlacedTarget[]>(
    () => rows.map((r) => ({ x: r.x ?? Number.NaN, y: r.y ?? Number.NaN, label: r.target.label })),
    [rows],
  );

  useEffect(() => {
    if (!filePath || !markers || !placed) return;
    overlayStore.add(filePath, {
      id: TARGET_LAYER_ID,
      kind: TARGET_LAYER_KIND,
      visible: true,
      paint: createTargetPainter({ targets: painted, highlightIndex: hoverIndex }),
    });
    return () => overlayStore.remove(filePath, TARGET_LAYER_ID);
  }, [filePath, markers, placed, painted, hoverIndex]);

  const place = useCallback(async () => {
    if (!filePath) return;
    const targets = parsed.targets;
    if (targets.length === 0) {
      setError("No targets to place: enter at least one 'RA Dec [label]' line.");
      return;
    }
    const seq = ++requestSeqRef.current;
    setLoading(true);
    setError(null);
    try {
      const result = await worldToPixel(
        filePath,
        targets.map((t): [number, number] => [t.lon, t.lat]),
        frame,
      );
      if (requestSeqRef.current !== seq) return;
      setPlaced({ targets, result });
      setHoverIndex(null);
    } catch (e: unknown) {
      if (requestSeqRef.current === seq) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (requestSeqRef.current === seq) setLoading(false);
    }
  }, [filePath, parsed, frame]);

  const loadCsv = useCallback(async () => {
    setError(null);
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({
        multiple: false,
        filters: [{ name: "Target lists", extensions: ["csv", "txt", "tsv", "dat"] }],
        title: "Load a target list",
      });
      const path = Array.isArray(picked) ? picked[0] : picked;
      if (!path) return;
      const { readTextFile } = await import("@tauri-apps/plugin-fs");
      setText(await readTextFile(path));
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  const addAsRegions = useCallback(() => {
    if (!filePath) return;
    for (const row of onImageRows) {
      if (row.x === null || row.y === null) continue;
      regionStore.add(filePath, {
        id: generateId(),
        shape: { shape: "point", x: row.x, y: row.y },
        props: { ...DEFAULT_REGION_PROPS, text: row.target.label },
        backgroundId: null,
      });
    }
  }, [filePath, onImageRows]);

  return (
    <div className="ab-panel overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <Pin size={12} className="text-cyan-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">Targets</span>
        </div>
        {loading && <Loader2 size={12} className="animate-spin text-cyan-400/70" />}
      </div>

      <div className="px-3 py-2 space-y-2">
        <div className="flex flex-col gap-0.5">
          <label htmlFor={textId} className="text-[9px] text-zinc-500 uppercase">
            RA Dec [label] per line, or CSV with ra/dec headers
          </label>
          <textarea
            id={textId}
            rows={TEXTAREA_ROWS}
            value={text}
            placeholder={LIST_PLACEHOLDER}
            spellCheck={false}
            onChange={(e) => setText(e.target.value)}
            className={TEXTAREA_CLASS}
          />
        </div>

        {parsed.errors.length > 0 && (
          <div className="text-[9px] text-amber-300/90 bg-amber-900/15 border border-amber-800/30 rounded px-2 py-1 space-y-0.5 max-h-24 overflow-auto">
            {parsed.errors.map((w, i) => (
              <div key={i}>{w}</div>
            ))}
          </div>
        )}

        <div className="flex flex-wrap items-center gap-2">
          <button type="button" onClick={() => void loadCsv()} className={SMALL_BUTTON_CLASS} title="Load a CSV or text list of targets">
            <FileUp size={10} />
            Load CSV
          </button>
          <label htmlFor={frameId} className="text-[9px] text-zinc-500 uppercase">
            Frame
          </label>
          <select id={frameId} value={frame} onChange={(e) => setFrame(e.target.value as SkyFrame)} className={SELECT_CLASS}>
            {FRAMES.map((f) => (
              <option key={f} value={f}>
                {f}
              </option>
            ))}
          </select>
          <span className="text-[9px] text-zinc-600 font-mono">{parsed.targets.length} parsed</span>
        </div>

        <RunButton
          label="Place"
          runningLabel="Projecting..."
          running={loading}
          disabled={!filePath || parsed.targets.length === 0}
          accent="cyan"
          icon={<Pin size={12} />}
          onClick={() => void place()}
        />

        <ErrorAlert message={error} />

        {placed && (
          <div className="text-[10px] text-zinc-400 font-mono">
            {onImageRows.length} of {rows.length} targets on the {placed.result.naxis1} x {placed.result.naxis2} image ({placed.result.frame})
          </div>
        )}

        {tableRows.length > 0 && (
          <div className="max-h-48 overflow-auto border border-zinc-800/60 rounded">
            <table className="w-full text-[9px] font-mono">
              <thead className="sticky top-0 bg-zinc-900 text-zinc-500">
                <tr>
                  <th className="text-left px-1.5 py-0.5">label</th>
                  <th className="text-right px-1.5 py-0.5">lon deg</th>
                  <th className="text-right px-1.5 py-0.5">lat deg</th>
                  <th className="text-right px-1.5 py-0.5">x</th>
                  <th className="text-right px-1.5 py-0.5">y</th>
                  <th className="text-right px-1.5 py-0.5">image</th>
                </tr>
              </thead>
              <tbody>
                {tableRows.map((row) => (
                  <tr
                    key={row.index}
                    onMouseEnter={() => setHoverIndex(row.index)}
                    onMouseLeave={() => setHoverIndex(null)}
                    className={`${row.onImage ? "text-zinc-300" : "text-zinc-500"} ${hoverIndex === row.index ? "bg-zinc-800/80" : ""}`}
                  >
                    <td className="px-1.5 py-0.5 truncate max-w-[100px]" title={row.target.raw}>
                      {row.target.label || row.index + 1}
                    </td>
                    <td className="text-right px-1.5 py-0.5">{fmt(row.target.lon, DEG_DIGITS)}</td>
                    <td className="text-right px-1.5 py-0.5">{fmt(row.target.lat, DEG_DIGITS)}</td>
                    <td className="text-right px-1.5 py-0.5">{fmt(row.x, PIXEL_DIGITS)}</td>
                    <td className="text-right px-1.5 py-0.5">{fmt(row.y, PIXEL_DIGITS)}</td>
                    <td className="text-right px-1.5 py-0.5">
                      <span
                        className={`px-1 rounded ${row.onImage ? "text-emerald-300 bg-emerald-900/30" : "text-zinc-500 bg-zinc-800/60"}`}
                      >
                        {row.onImage ? "on" : row.x === null ? "n/a" : "off"}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            {rows.length > TABLE_ROW_LIMIT && (
              <div className="text-[9px] text-zinc-600 px-1.5 py-0.5">
                first {TABLE_ROW_LIMIT} of {rows.length} targets shown; markers and regions include every row
              </div>
            )}
          </div>
        )}

        {placed && (
          <div className="flex flex-wrap items-center gap-1.5">
            <Toggle label="Markers" checked={markers} accent="cyan" onChange={setMarkers} />
            <button
              type="button"
              onClick={addAsRegions}
              disabled={onImageRows.length === 0}
              className={SMALL_BUTTON_CLASS}
              title="Add the on-image targets as Point regions named by their label"
            >
              <Shapes size={10} />
              Add as Point regions
            </button>
          </div>
        )}

        {!placed && !error && (
          <div className="text-[10px] text-zinc-600">
            Paste RA/Dec pairs (sexagesimal RA is hours, decimal RA is degrees) or load a CSV, pick the input frame and
            place them on the image through its WCS. The table shows the interpreted degrees, the pixel position and
            whether each target lands on the image.
          </div>
        )}
      </div>
    </div>
  );
}

export default memo(TargetsPanel);
