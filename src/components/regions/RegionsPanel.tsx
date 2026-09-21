import { memo, useCallback, useState } from "react";
import { Shapes, FileUp, FileDown, Trash2, Loader2 } from "lucide-react";
import type { Region, RegionStatsEntry, RegionSystem } from "../../shared/types";
import { importRegions, exportRegions, toWire } from "../../services/regions";
import { useRegionDoc } from "../../hooks/useRegionStore";
import { useRegionStats } from "../../hooks/useRegionStats";
import { regionStore } from "../../utils/regionStore";
import { normalizeProps } from "../../utils/regionPersistence";
import { shapeSummary, isRegionShape } from "../../utils/regionGeometry";
import { useDqContext } from "../../context/PreviewContext";
import { generateId } from "../../utils/format";

interface RegionsPanelProps {
  filePath: string | null;
}

const SYSTEMS: RegionSystem[] = ["image", "fk5", "icrs"];
const DEFAULT_SWATCH = "#7dd3fc";

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
          {s.clipped && <span className="text-amber-400/80">clipped</span>}
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

function RegionsPanel({ filePath }: RegionsPanelProps) {
  const doc = useRegionDoc(filePath);
  const { excludeDq } = useDqContext();
  const { stats, loading, error: statsError } = useRegionStats(filePath, doc.regions, excludeDq);
  const [system, setSystem] = useState<RegionSystem>("image");
  const [busy, setBusy] = useState(false);
  const [ioError, setIoError] = useState<string | null>(null);
  const [warnings, setWarnings] = useState<string[]>([]);

  const annuli = doc.regions.filter((r) => r.shape.shape === "annulus");

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

  return (
    <div className="ab-panel overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <Shapes size={12} className="text-sky-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">Regions</span>
          <span className="text-[9px] text-zinc-600 font-mono">{doc.regions.length}</span>
          {excludeDq && <span className="text-[9px] px-1.5 py-0.5 rounded text-amber-300 bg-amber-900/30">DQ masked</span>}
        </div>
        <div className="flex items-center gap-1">
          {(loading || busy) && <Loader2 size={12} className="animate-spin text-sky-400/70" />}
          <select
            value={system}
            onChange={(e) => setSystem(e.target.value as RegionSystem)}
            className="bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[9px] text-zinc-300 font-mono"
            aria-label="Coordinate system for export"
            title="Coordinate system for export"
          >
            {SYSTEMS.map((s) => (
              <option key={s} value={s}>
                {s}
              </option>
            ))}
          </select>
          <button
            onClick={handleImport}
            disabled={!filePath || busy}
            className="flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded text-zinc-400 hover:text-zinc-200 disabled:opacity-40"
            title="Import a DS9 .reg file"
          >
            <FileUp size={11} /> Import
          </button>
          <button
            onClick={handleExport}
            disabled={!filePath || busy || doc.regions.length === 0}
            className="flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded text-zinc-400 hover:text-zinc-200 disabled:opacity-40"
            title="Export the regions as a DS9 .reg file"
          >
            <FileDown size={11} /> Export
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
