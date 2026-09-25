import { memo, useCallback, useMemo, useState } from "react";
import { Shapes, FileUp, FileDown, ClipboardCopy, Trash2, Loader2 } from "lucide-react";
import type { Region, RegionStatsEntry, RegionSystem } from "../../shared/types";
import { REGION_SYSTEMS, type RegionCalibrated } from "../../shared/types/regions";
import { importRegions, exportRegions, toWire } from "../../services/regions";
import { useRegionDoc } from "../../hooks/useRegionStore";
import { useRegionStats } from "../../hooks/useRegionStats";
import { regionStore } from "../../utils/regionStore";
import { normalizeProps } from "../../utils/regionPersistence";
import { shapeSummary, isRegionShape } from "../../utils/regionGeometry";
import { regionsCsvFileName, regionsTableCsv } from "../../utils/regionCsv";
import { formatLat, formatLon } from "../../utils/coordFormat";
import { useDqContext } from "../../context/PreviewContext";
import { generateId } from "../../utils/format";
import MeasurementBadge from "../analysis/MeasurementBadge";

interface RegionsPanelProps {
  filePath: string | null;
  measurePath: string | null;
}

const DEFAULT_SWATCH = "#7dd3fc";
const DEFAULT_CLIP_SIGMA_TEXT = "3";
const DEFAULT_CLIP_ITERS_TEXT = "5";
const MAG_DECIMALS = 3;
const SURFACE_BRIGHTNESS_DECIMALS = 2;
const AREA_DECIMALS = 3;
const POSITION_ANGLE_DECIMALS = 1;
const MISSING_CALIBRATION_REASON = "no flux calibration in the header";
const CLIP_INPUT_CLASS =
  "w-9 bg-zinc-900 border rounded px-1 py-0.5 text-[9px] text-zinc-300 font-mono text-right focus:outline-none focus:border-sky-500/60";
const HEADER_BUTTON_CLASS =
  "flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded text-zinc-400 hover:text-zinc-200 disabled:opacity-40";

function parsePositive(text: string): number | null {
  const trimmed = text.trim();
  if (trimmed === "") return null;
  const v = Number(trimmed);
  return Number.isFinite(v) && v > 0 ? v : null;
}

function formatJansky(v: number | null | undefined): string {
  if (v == null || !Number.isFinite(v)) return "--";
  const a = Math.abs(v);
  if (a >= 1) return `${v.toFixed(4)} Jy`;
  if (a >= 1e-3) return `${(v * 1e3).toFixed(4)} mJy`;
  if (a >= 1e-6) return `${(v * 1e6).toFixed(4)} uJy`;
  return `${(v * 1e9).toFixed(4)} nJy`;
}

function formatJanskyWithError(v: number, err: number | null): string {
  return err != null && Number.isFinite(err) ? `${formatJansky(v)} ± ${formatJansky(err)}` : formatJansky(v);
}

function formatMagnitude(v: number | null, err: number | null, decimals: number): string {
  if (v == null || !Number.isFinite(v)) return "--";
  const base = v.toFixed(decimals);
  return err != null && Number.isFinite(err) ? `${base} ± ${err.toFixed(decimals)}` : base;
}

function formatArea(cal: RegionCalibrated, count: number): string {
  return cal.area_arcsec2 != null && Number.isFinite(cal.area_arcsec2)
    ? `${cal.area_arcsec2.toFixed(AREA_DECIMALS)}"^2`
    : `${count} px`;
}

function skyLine(cal: RegionCalibrated): string | null {
  if (cal.ra == null || cal.dec == null) return null;
  const centre = `RA ${formatLon(cal.ra, { hours: true, format: "sexagesimal" })} Dec ${formatLat(cal.dec, { format: "sexagesimal" })}`;
  return cal.pa_sky_deg != null ? `${centre} PA ${cal.pa_sky_deg.toFixed(POSITION_ANGLE_DECIMALS)}° E of N` : centre;
}

function fileStem(path: string): string {
  const base = path.split(/[\\/]/).pop() ?? "regions";
  const dot = base.lastIndexOf(".");
  return dot > 0 ? base.slice(0, dot) : base;
}

function fmt(v: number | null | undefined, digits = 3): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return "--";
  const a = Math.abs(v);
  if (a !== 0 && (a >= 1e5 || a < 1e-3)) return v.toExponential(digits);
  return v.toFixed(digits);
}

function StatCell({ label, value }: { label: string; value: string }) {
  return (
    <span className="whitespace-nowrap">
      <span className="text-zinc-600">{label} </span>
      <span className="text-zinc-300">{value}</span>
    </span>
  );
}

function RegionRow({
  region,
  selected,
  entry,
  annuli,
  onSelect,
  onBackground,
  onDelete,
}: {
  region: Region;
  selected: boolean;
  entry: RegionStatsEntry | undefined;
  annuli: Region[];
  onSelect: (id: string) => void;
  onBackground: (id: string, bg: string | null) => void;
  onDelete: (id: string) => void;
}) {
  const s = entry?.stats ?? null;
  const canBackground = region.shape.shape !== "line" && region.shape.shape !== "point";
  return (
    <div
      role="option"
      tabIndex={0}
      aria-selected={selected}
      onClick={() => onSelect(region.id)}
      onKeyDown={(e) => {
        if (e.target !== e.currentTarget) return;
        if (e.key !== "Enter" && e.key !== " ") return;
        e.preventDefault();
        onSelect(region.id);
      }}
      className={`rounded px-2 py-1.5 cursor-pointer border focus:outline-none focus-visible:ring-1 focus-visible:ring-sky-400 ${
        selected ? "border-sky-500/50 bg-sky-900/15" : "border-transparent bg-zinc-900/70 hover:bg-zinc-800/60"
      }`}
    >
      <div className="flex items-center gap-2 text-[10px]">
        <span
          className="w-2.5 h-2.5 rounded-sm shrink-0 border border-black/40"
          style={{ background: region.props.color ?? DEFAULT_SWATCH }}
        />
        <span className="font-mono text-zinc-300 truncate">{shapeSummary(region.shape)}</span>
        {region.props.text && <span className="text-zinc-500 truncate">{region.props.text}</span>}
        {!region.props.include && <span className="text-[9px] text-amber-400/80">excl</span>}
        <button
          onClick={(e) => {
            e.stopPropagation();
            onDelete(region.id);
          }}
          className="ml-auto text-zinc-600 hover:text-red-400 shrink-0"
          title="Delete region"
        >
          <Trash2 size={11} />
        </button>
      </div>
      {entry?.error && <div className="mt-1 text-[9px] text-red-400 break-words">{entry.error}</div>}
      {s && (
        <div className="mt-1 flex flex-wrap gap-x-3 gap-y-0.5 text-[9px] font-mono">
          <StatCell label="n" value={`${s.count}${s.n_excluded > 0 ? ` (−${s.n_excluded} dq)` : ""}`} />
          <StatCell label="mean" value={fmt(s.mean)} />
          <StatCell label="med" value={fmt(s.median)} />
          <StatCell label="sum" value={s.sum_err != null ? `${fmt(s.sum)} ± ${fmt(s.sum_err)}` : fmt(s.sum)} />
          <StatCell label="σ" value={fmt(s.sigma)} />
          {s.weighted_mean != null && <StatCell label="wmean" value={fmt(s.weighted_mean)} />}
          {s.net_sum !== null && <StatCell label="net" value={fmt(s.net_sum)} />}
          {s.net_snr !== null && <StatCell label="snr" value={fmt(s.net_snr, 1)} />}
          {s.calibrated && (
            <>
              <StatCell label="Jy" value={formatJanskyWithError(s.calibrated.flux_jy, s.calibrated.flux_err_jy)} />
              <StatCell label="AB" value={formatMagnitude(s.calibrated.mag_ab, s.calibrated.mag_ab_err, MAG_DECIMALS)} />
              <StatCell label="SB" value={formatMagnitude(s.calibrated.sb_mag_arcsec2, null, SURFACE_BRIGHTNESS_DECIMALS)} />
              <StatCell label="area" value={formatArea(s.calibrated, s.count)} />
            </>
          )}
          {s.clipped && <span className="text-amber-400/80">clipped</span>}
        </div>
      )}
      {s?.calibrated && skyLine(s.calibrated) && (
        <div className="mt-0.5 text-[9px] font-mono text-zinc-400 whitespace-nowrap overflow-hidden text-ellipsis">
          {skyLine(s.calibrated)}
        </div>
      )}
      {canBackground && annuli.length > 0 && (
        <div className="mt-1 flex items-center gap-1.5 text-[9px]" onClick={(e) => e.stopPropagation()}>
          <span className="text-zinc-600">bg</span>
          <select
            value={region.backgroundId ?? ""}
            aria-label={`bg annulus for ${shapeSummary(region.shape)}`}
            onChange={(e) => onBackground(region.id, e.target.value || null)}
            className="bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[9px] text-zinc-300 font-mono max-w-[180px]"
          >
            <option value="">none</option>
            {annuli.map((a) => (
              <option key={a.id} value={a.id}>
                {shapeSummary(a.shape)}
              </option>
            ))}
          </select>
        </div>
      )}
    </div>
  );
}

function RegionsPanel({ filePath, measurePath }: RegionsPanelProps) {
  const doc = useRegionDoc(filePath);
  const { excludeDq } = useDqContext();
  const [sigmaText, setSigmaText] = useState(DEFAULT_CLIP_SIGMA_TEXT);
  const [itersText, setItersText] = useState(DEFAULT_CLIP_ITERS_TEXT);
  const sigma = parsePositive(sigmaText);
  const maxiters = parsePositive(itersText);
  const clip = useMemo(() => ({ sigma, maxiters }), [sigma, maxiters]);
  const { stats, loading, error: statsError, photcal, calibrationWarnings } = useRegionStats(
    measurePath,
    doc.regions,
    excludeDq,
    clip,
  );
  const [system, setSystem] = useState<RegionSystem>("image");
  const [busy, setBusy] = useState(false);
  const [ioError, setIoError] = useState<string | null>(null);
  const [warnings, setWarnings] = useState<string[]>([]);

  const annuli = doc.regions.filter((r) => r.shape.shape === "annulus");
  const measured = doc.regions.length > 0 && stats.size > 0;

  const handleSelect = useCallback(
    (id: string) => {
      if (filePath) regionStore.select(filePath, doc.selectedId === id ? null : id);
    },
    [filePath, doc.selectedId],
  );
  const handleBackground = useCallback(
    (id: string, bg: string | null) => {
      if (filePath) regionStore.setBackground(filePath, id, bg);
    },
    [filePath],
  );
  const handleDelete = useCallback(
    (id: string) => {
      if (filePath) regionStore.remove(filePath, id);
    },
    [filePath],
  );

  const handleImport = useCallback(async () => {
    if (!filePath || busy) return;
    setBusy(true);
    setIoError(null);
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({ multiple: false, filters: [{ name: "DS9 regions", extensions: ["reg"] }], title: "Import regions" });
      const regPath = Array.isArray(picked) ? picked[0] : picked;
      if (!regPath) return;
      const { readTextFile } = await import("@tauri-apps/plugin-fs");
      const text = await readTextFile(regPath);
      const res = await importRegions(filePath, text);
      let added = 0;
      for (const w of res.regions) {
        if (!isRegionShape(w.shape)) continue;
        regionStore.add(filePath, { id: generateId(), shape: w.shape, props: normalizeProps(w.props), backgroundId: null });
        added += 1;
      }
      const notes = res.warnings.slice();
      if (!res.has_wcs && res.regions.length === 0) notes.push("image has no WCS: sky-system regions cannot be imported");
      if (res.regions.length === 0) notes.push("no supported regions found in the file");
      else if (added < res.regions.length) {
        const skipped = res.regions.length - added;
        notes.push(`${skipped} region${skipped === 1 ? "" : "s"} skipped as invalid`);
      }
      setWarnings(notes);
    } catch (e) {
      setIoError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }, [filePath, busy]);

  const handleExport = useCallback(async () => {
    if (!filePath || busy || doc.regions.length === 0) return;
    setBusy(true);
    setIoError(null);
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const target = await save({
        defaultPath: `${fileStem(filePath)}.reg`,
        filters: [{ name: "DS9 regions", extensions: ["reg"] }],
        title: "Export regions",
      });
      if (!target) return;
      const res = await exportRegions(filePath, doc.regions.map(toWire), system, true);
      const { writeTextFile } = await import("@tauri-apps/plugin-fs");
      await writeTextFile(target, res.reg_text);
      setWarnings([]);
    } catch (e) {
      setIoError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }, [filePath, busy, doc.regions, system]);

  const handleCopyCsv = useCallback(async () => {
    if (doc.regions.length === 0) return;
    setIoError(null);
    try {
      await navigator.clipboard.writeText(regionsTableCsv(doc.regions, stats));
    } catch (e) {
      setIoError(`Clipboard copy failed: ${e instanceof Error ? e.message : String(e)}`);
    }
  }, [doc.regions, stats]);

  const handleSaveCsv = useCallback(async () => {
    if (!filePath || busy || doc.regions.length === 0) return;
    setBusy(true);
    setIoError(null);
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const target = await save({
        defaultPath: regionsCsvFileName(measurePath ?? filePath),
        filters: [{ name: "CSV", extensions: ["csv"] }],
        title: "Save the region table as CSV",
      });
      if (!target) return;
      const { writeTextFile } = await import("@tauri-apps/plugin-fs");
      await writeTextFile(target, regionsTableCsv(doc.regions, stats));
    } catch (e) {
      setIoError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }, [filePath, measurePath, busy, doc.regions, stats]);

  const calibrationBadge = measured ? (
    photcal ? (
      <span
        className="text-[9px] px-1.5 py-0.5 rounded text-emerald-300 bg-emerald-900/30 truncate max-w-[220px]"
        title={[
          `${photcal.label}. Region membership is centre-in on the pixel lattice, so a circle region and a click aperture of the same radius differ by their edge pixels.`,
          ...calibrationWarnings,
        ].join("\n")}
      >
        Jy/AB: {photcal.label} - exact-pixel region
      </span>
    ) : (
      <span
        className="text-[9px] px-1.5 py-0.5 rounded text-amber-300 bg-amber-900/30 truncate max-w-[220px]"
        title={calibrationWarnings.join("\n") || MISSING_CALIBRATION_REASON}
      >
        uncalibrated: {calibrationWarnings[0] ?? MISSING_CALIBRATION_REASON}
      </span>
    )
  ) : null;

  return (
    <div className="ab-panel overflow-hidden">
      <div className="flex flex-wrap items-center justify-between gap-y-1 px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2 min-w-0">
          <Shapes size={12} className="text-sky-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">Regions</span>
          <span className="text-[9px] text-zinc-600 font-mono">{doc.regions.length}</span>
          {doc.regions.length > 0 && <MeasurementBadge />}
          {excludeDq && <span className="text-[9px] px-1.5 py-0.5 rounded text-amber-300 bg-amber-900/30">DQ masked</span>}
          {calibrationBadge}
        </div>
        <div className="flex flex-wrap items-center gap-1">
          {(loading || busy) && <Loader2 size={12} className="animate-spin text-sky-400/70" />}
          <label className="flex items-center gap-1 text-[9px] text-zinc-500" title="Sigma-clipping threshold in sigma units (blank or invalid uses the default 3)">
            sigma
            <input
              type="text"
              inputMode="decimal"
              value={sigmaText}
              onChange={(e) => setSigmaText(e.target.value)}
              aria-label="Sigma clipping threshold"
              className={`${CLIP_INPUT_CLASS} ${sigma === null && sigmaText.trim() !== "" ? "border-amber-500/60" : "border-zinc-800"}`}
            />
          </label>
          <label className="flex items-center gap-1 text-[9px] text-zinc-500" title="Maximum sigma-clipping iterations (blank or invalid uses the default 5)">
            iters
            <input
              type="text"
              inputMode="numeric"
              value={itersText}
              onChange={(e) => setItersText(e.target.value)}
              aria-label="Sigma clipping iterations"
              className={`${CLIP_INPUT_CLASS} ${maxiters === null && itersText.trim() !== "" ? "border-amber-500/60" : "border-zinc-800"}`}
            />
          </label>
          <select
            value={system}
            onChange={(e) => setSystem(e.target.value as RegionSystem)}
            className="bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[9px] text-zinc-300 font-mono"
            aria-label="Coordinate system for export"
            title="Coordinate system for export"
          >
            {REGION_SYSTEMS.map((s) => (
              <option key={s} value={s}>
                {s}
              </option>
            ))}
          </select>
          <button onClick={handleImport} disabled={!filePath || busy} className={HEADER_BUTTON_CLASS} title="Import a DS9 .reg file">
            <FileUp size={11} /> Import
          </button>
          <button
            onClick={handleExport}
            disabled={!filePath || busy || doc.regions.length === 0}
            className={HEADER_BUTTON_CLASS}
            title="Export the regions as a DS9 .reg file"
          >
            <FileDown size={11} /> Export
          </button>
          <button
            onClick={() => void handleCopyCsv()}
            disabled={doc.regions.length === 0}
            className={HEADER_BUTTON_CLASS}
            title="Copy the region table (statistics and calibrated fluxes) as CSV"
          >
            <ClipboardCopy size={11} /> Copy CSV
          </button>
          <button
            onClick={() => void handleSaveCsv()}
            disabled={!filePath || busy || doc.regions.length === 0}
            className={HEADER_BUTTON_CLASS}
            title="Save the region table (statistics and calibrated fluxes) as a CSV file"
          >
            <FileDown size={11} /> Save CSV
          </button>
        </div>
      </div>
      <div className="px-3 py-2 space-y-1.5">
        {!filePath && <div className="text-[10px] text-zinc-600">Select a file to draw regions.</div>}
        {filePath && doc.regions.length === 0 && (
          <div className="text-[10px] text-zinc-600">
            Pick a shape tool in the viewer toolbar and drag on the image, or import a .reg file.
          </div>
        )}
        {(ioError || statsError) && (
          <div className="text-[10px] text-red-400 bg-red-900/20 border border-red-800/30 rounded px-2.5 py-1.5 break-words">
            {ioError ?? statsError}
          </div>
        )}
        {warnings.length > 0 && (
          <div className="text-[9px] text-amber-300/90 bg-amber-900/15 border border-amber-800/30 rounded px-2 py-1 space-y-0.5">
            {warnings.map((w, i) => (
              <div key={i}>{w}</div>
            ))}
          </div>
        )}
        {doc.regions.length > 0 && (
          <div role="listbox" aria-label="Regions" className="space-y-1.5">
            {doc.regions.map((r) => (
              <RegionRow
                key={r.id}
                region={r}
                selected={r.id === doc.selectedId}
                entry={stats.get(r.id)}
                annuli={annuli.filter((a) => a.id !== r.id)}
                onSelect={handleSelect}
                onBackground={handleBackground}
                onDelete={handleDelete}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

export default memo(RegionsPanel);
