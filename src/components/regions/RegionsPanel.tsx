import { memo, useCallback, useEffect, useMemo, useState } from "react";
import { Shapes, FileUp, FileDown, ClipboardCopy, Plus, Trash2, Loader2 } from "lucide-react";
import type { Region, RegionStats, RegionStatsEntry, RegionSystem } from "../../shared/types";
import { REGION_SYSTEMS, type RegionNetErrTerms, type RegionSky } from "../../shared/types/regions";
import { importRegions, exportRegions, toWire } from "../../services/regions";
import { useRegionDoc } from "../../hooks/useRegionStore";
import {
  DEFAULT_CLIP_ITERS,
  DEFAULT_CLIP_SIGMA,
  MAX_CLIP_ITERS,
  clipFieldError,
  parseClipIters,
  parseClipSigma,
  useRegionStats,
} from "../../hooks/useRegionStats";
import { regionStore, shapeKindKey } from "../../utils/regionStore";
import { normalizeProps } from "../../utils/regionPersistence";
import { shapeSummary, shapeSummaryTitle, isRegionShape } from "../../utils/regionGeometry";
import { regionSkyOf, regionsCsvFileName, regionsTableCsv, type RegionCsvContext } from "../../utils/regionCsv";
import { summedFluxUnit } from "../../utils/photometryTable";
import { formatLat, formatLon } from "../../utils/coordFormat";
import { useDqContext } from "../../context/PreviewContext";
import { useMeasurementProvenance } from "../../hooks/useMeasurementLog";
import { measurementLog, regionLogReady, regionStatsEntries } from "../../utils/measurementLog";
import { generateId } from "../../utils/format";
import { formatSignificant } from "../../utils/formatSignificant";
import { formatFluxDensityJy, formatWithUncertainty } from "../../utils/formatUncertainty";
import { SIGMA_MAD_LABEL, SIGMA_MAD_TITLE } from "../../utils/analysisLabels";
import {
  EXCLUDED_CELL_TITLE,
  EXCLUDE_TITLE,
  backgroundCandidates,
  excludedCellLabel,
  toggledInclude,
} from "../../utils/regionExclude";
import MeasurementBadge from "../analysis/MeasurementBadge";
import { CopyCsvButton, DecimalField, ErrorAlert } from "../ui";
import { ARMED_CONFIRM_WINDOW_MS, CONFIRM_DELETE_LABEL, useArmedConfirm } from "../../hooks/useArmedConfirm";

interface RegionsPanelProps {
  filePath: string | null;
  measurePath: string | null;
  measureKey: string | null;
}

const DEFAULT_SWATCH = "#7dd3fc";
const DEFAULT_CLIP_SIGMA_TEXT = String(DEFAULT_CLIP_SIGMA);
const DEFAULT_CLIP_ITERS_TEXT = String(DEFAULT_CLIP_ITERS);
const CLIP_SIGMA_TITLE = `Sigma-clipping threshold for the clipped mean and sigma of each region (shown in the row when pixels are rejected, and logged); blank uses the default ${DEFAULT_CLIP_SIGMA}`;
const CLIP_ITERS_TITLE = `Maximum sigma-clipping iterations, a whole number from 1 to ${MAX_CLIP_ITERS}; blank uses the default ${DEFAULT_CLIP_ITERS}`;
const CLIP_ERROR_CLASS = "text-[9px] text-amber-400/90 px-3";
const DQ_NOT_APPLIED_PREFIX = "DQ exclusion requested but not applied: ";
const NO_DQ_MASK_TITLE = "No DQ mask was applied to these statistics";
const PARTLY_OFF_IMAGE_TITLE = "Part of the region lies outside the image; the statistics use the pixels inside";
const ZERO_PADDING_TITLE = "exact zeros are treated as padding and left out";
const NO_DATA_TITLE = "pixels without data (NaN or infinite) are left out";
const MAG_DECIMALS = 3;
const SURFACE_BRIGHTNESS_DECIMALS = 2;
const AREA_DECIMALS = 3;
const POSITION_ANGLE_DECIMALS = 1;
const MISSING_CALIBRATION_REASON = "no flux calibration in the header";
const MISSING_VALUE = "--";
const CLIP_INPUT_CLASS =
  "w-9 bg-zinc-900 border rounded px-1 py-0.5 text-[9px] text-zinc-300 font-mono text-right focus:outline-none focus:border-sky-500/60 focus-visible:ring-1 focus-visible:ring-sky-500/60";
const HEADER_BUTTON_CLASS =
  "flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded text-zinc-400 hover:text-zinc-200 disabled:opacity-40";
const DANGER_BUTTON_CLASS = "flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded text-red-400 hover:text-red-300";
const DELETE_POINTS_CONFIRM_MS = 5000;
const INCLUDE_TOGGLE_CLASS = "text-[9px] px-1 rounded border shrink-0";
const INCLUDE_ON_CLASS = "text-zinc-500 border-zinc-700/60 hover:text-zinc-300";
const INCLUDE_OFF_CLASS = "text-amber-400/90 border-amber-700/60 bg-amber-900/20 hover:text-amber-300";
const DELETE_IDLE_CLASS = "text-zinc-600 hover:text-red-400";
const DELETE_ARMED_CLASS = "text-[9px] px-1 rounded border border-red-700/60 text-red-400 hover:text-red-300";
const DELETE_ARMED_TITLE = `Press again within ${ARMED_CONFIRM_WINDOW_MS / 1000} s to delete the region`;
const SNR_DECIMALS = 1;
const NET_ERROR_TITLES: Record<RegionNetErrTerms, string> = {
  "err_plane+background_level":
    "Error of the net sum: ERR plane of the region in quadrature with the background level term n² σ_bg² / n_bg",
  "background_scatter+background_level":
    "Error of the net sum: no ERR plane, so the pixel noise n σ_bg² from the background scatter, plus the background level term n² σ_bg² / n_bg",
};
const NO_NET_ERROR_TITLE = "No error: the background has zero scatter and the region has no ERR noise";
const SNR_TITLE = "Net sum divided by its error";
const COPY_CSV_TITLE = "Copy the region table (statistics and calibrated fluxes) as CSV";
const SAVE_CSV_TITLE = "Save the region table (statistics and calibrated fluxes) as a CSV file";
const STATS_PENDING_TITLE = "Waiting for statistics that match the current regions, clipping and DQ settings";

function regionDeleteKey(id: string): string {
  return `region:${id}`;
}

function formatMagnitude(v: number | null, decimals: number): string {
  if (v == null || !Number.isFinite(v)) return MISSING_VALUE;
  return v.toFixed(decimals);
}

function isPositiveError(err: number | null | undefined): err is number {
  return err != null && Number.isFinite(err) && err > 0;
}

function formatArea(areaArcsec2: number | null | undefined): string | null {
  return areaArcsec2 != null && Number.isFinite(areaArcsec2) ? `${areaArcsec2.toFixed(AREA_DECIMALS)}"^2` : null;
}

function skyLine(sky: RegionSky | null): string | null {
  if (sky?.ra == null || sky.dec == null) return null;
  const centre = `RA ${formatLon(sky.ra, { hours: true, format: "sexagesimal" })} Dec ${formatLat(sky.dec, { format: "sexagesimal" })}`;
  const pa =
    sky.pa_sky_deg != null && Number.isFinite(sky.pa_sky_deg)
      ? `${sky.pa_sky_deg.toFixed(POSITION_ANGLE_DECIMALS)}° E of N`
      : MISSING_VALUE;
  return `${centre} PA ${pa}`;
}

function fileStem(path: string): string {
  const base = path.split(/[\\/]/).pop() ?? "regions";
  const dot = base.lastIndexOf(".");
  return dot > 0 ? base.slice(0, dot) : base;
}

function fmt(v: number | null | undefined): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return MISSING_VALUE;
  return formatSignificant(v);
}

function countCell(s: RegionStats): { value: string; title: string | undefined } {
  const excludedLabel = excludedCellLabel(s);
  const parts = [
    excludedLabel && { label: excludedLabel, title: EXCLUDED_CELL_TITLE },
    s.n_padding > 0 && { label: `(−${s.n_padding} zero)`, title: ZERO_PADDING_TITLE },
    s.n_nan > 0 && { label: `(−${s.n_nan} no data)`, title: NO_DATA_TITLE },
  ].filter((part): part is { label: string; title: string } => !!part);
  return {
    value: [String(s.count), ...parts.map((part) => part.label)].join(" "),
    title: parts.length > 0 ? parts.map((part) => part.title).join("; ") : undefined,
  };
}

function fmtSnr(v: number): string {
  const a = Math.abs(v);
  if (a !== 0 && (a >= 1e5 || a < 1e-3)) return v.toExponential(SNR_DECIMALS);
  return v.toFixed(SNR_DECIMALS);
}

function StatCell({ label, value, title }: { label: string; value: string; title?: string }) {
  return (
    <span className="whitespace-nowrap" title={title}>
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
  onToggleInclude,
  deleteArmed,
  onDelete,
  onDeleteBlur,
  clip,
}: {
  region: Region;
  selected: boolean;
  entry: RegionStatsEntry | undefined;
  annuli: Region[];
  onSelect: (id: string) => void;
  onBackground: (id: string, bg: string | null) => void;
  onToggleInclude: (region: Region) => void;
  deleteArmed: boolean;
  onDelete: (id: string) => void;
  onDeleteBlur: (id: string) => void;
  clip: { sigma: number; maxiters: number };
}) {
  const s = entry?.stats ?? null;
  const sky = regionSkyOf(s);
  const area = formatArea(sky?.area_arcsec2);
  const skyText = skyLine(sky);
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
        <span className="font-mono text-zinc-300 truncate" title={shapeSummaryTitle(region.shape)}>
          {shapeSummary(region.shape)}
        </span>
        {region.props.text && <span className="text-zinc-500 truncate">{region.props.text}</span>}
        {canBackground ? (
          <button
            type="button"
            aria-pressed={!region.props.include}
            title={EXCLUDE_TITLE}
            onClick={(e) => {
              e.stopPropagation();
              onToggleInclude(region);
            }}
            className={`${INCLUDE_TOGGLE_CLASS} ${region.props.include ? INCLUDE_ON_CLASS : INCLUDE_OFF_CLASS}`}
          >
            {region.props.include ? "incl" : "excl"}
          </button>
        ) : (
          !region.props.include && <span className="text-[9px] text-amber-400/80">excl</span>
        )}
        <button
          type="button"
          aria-live="polite"
          aria-label={deleteArmed ? undefined : "Delete region"}
          onClick={(e) => {
            e.stopPropagation();
            onDelete(region.id);
          }}
          onBlur={() => onDeleteBlur(region.id)}
          className={`ml-auto shrink-0 min-w-6 min-h-6 inline-flex items-center justify-center ${deleteArmed ? DELETE_ARMED_CLASS : DELETE_IDLE_CLASS}`}
          title={deleteArmed ? DELETE_ARMED_TITLE : "Delete region"}
        >
          {deleteArmed ? CONFIRM_DELETE_LABEL : <Trash2 size={11} />}
        </button>
      </div>
      {entry?.error && <div className="mt-1 text-[9px] text-red-400 break-words">{entry.error}</div>}
      {s && (
        <div className="mt-1 flex flex-wrap gap-x-3 gap-y-0.5 text-[9px] font-mono">
          <StatCell label="n" {...countCell(s)} />
          <StatCell label="mean" value={fmt(s.mean)} />
          <StatCell label="med" value={fmt(s.median)} />
          <StatCell label="sum" value={isPositiveError(s.sum_err) ? formatWithUncertainty(s.sum, s.sum_err) : fmt(s.sum)} />
          <StatCell label={SIGMA_MAD_LABEL} value={fmt(s.sigma)} title={SIGMA_MAD_TITLE} />
          {s.n_rejected > 0 && s.clipped_mean !== null && s.clipped_sigma !== null && (
            <StatCell
              label="sigma-clip"
              value={`mean ${fmt(s.clipped_mean)}, sigma ${fmt(s.clipped_sigma)}, ${s.n_rejected} rejected`}
              title={`After sigma clipping at ${clip.sigma} sigma and at most ${clip.maxiters} iterations: ${s.n_rejected} of ${s.count} pixels rejected. Every other value of this row uses all the pixels.`}
            />
          )}
          {s.weighted_mean != null && <StatCell label="wmean" value={fmt(s.weighted_mean)} />}
          {s.net_sum !== null && (
            <StatCell
              label="net"
              value={isPositiveError(s.net_err) ? formatWithUncertainty(s.net_sum, s.net_err) : fmt(s.net_sum)}
              title={s.net_err_terms ? NET_ERROR_TITLES[s.net_err_terms] : NO_NET_ERROR_TITLE}
            />
          )}
          {s.net_snr !== null && <StatCell label="snr" value={fmtSnr(s.net_snr)} title={SNR_TITLE} />}
          {s.calibrated && (
            <>
              <StatCell label="Jy" value={formatFluxDensityJy(s.calibrated.flux_jy, s.calibrated.flux_err_jy)} />
              <StatCell
                label="AB"
                value={
                  isPositiveError(s.calibrated.mag_ab_err)
                    ? formatWithUncertainty(s.calibrated.mag_ab, s.calibrated.mag_ab_err)
                    : formatMagnitude(s.calibrated.mag_ab, MAG_DECIMALS)
                }
              />
              <StatCell label="SB" value={formatMagnitude(s.calibrated.sb_mag_arcsec2, SURFACE_BRIGHTNESS_DECIMALS)} />
            </>
          )}
          {area && <StatCell label="area" value={area} />}
          {s.clipped && (
            <span className="text-amber-400/80" title={PARTLY_OFF_IMAGE_TITLE}>
              partly off image
            </span>
          )}
        </div>
      )}
      {skyText && (
        <div className="mt-0.5 text-[9px] font-mono text-zinc-400 whitespace-nowrap overflow-hidden text-ellipsis">
          {skyText}
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

function RegionsPanel({ filePath, measurePath, measureKey }: RegionsPanelProps) {
  const doc = useRegionDoc(filePath);
  const { excludeDq } = useDqContext();
  const provenance = useMeasurementProvenance();
  const [sigmaText, setSigmaText] = useState(DEFAULT_CLIP_SIGMA_TEXT);
  const [itersText, setItersText] = useState(DEFAULT_CLIP_ITERS_TEXT);
  const sigma = parseClipSigma(sigmaText);
  const maxiters = parseClipIters(itersText);
  const clip = useMemo(() => ({ sigma, maxiters }), [sigma, maxiters]);
  const sigmaError = clipFieldError("sigma", sigmaText);
  const itersError = clipFieldError("iters", itersText);
  const {
    stats,
    loading,
    error: statsError,
    photcal,
    calibrationWarnings,
    measured: statsMeasured,
    notes: statsNotes,
    units,
  } = useRegionStats(measurePath, doc.regions, excludeDq, clip, measureKey);
  const [system, setSystem] = useState<RegionSystem>("image");
  const [busy, setBusy] = useState(false);
  const [ioError, setIoError] = useState<string | null>(null);
  const [warnings, setWarnings] = useState<string[]>([]);
  const [armedPointsKey, setArmedPointsKey] = useState<string | null>(null);

  const pointCount = doc.regions.filter((r) => r.shape.shape === "point").length;
  const pointsKey = useMemo(() => shapeKindKey(doc.regions, "point"), [doc.regions]);
  const confirmingDeletePoints = armedPointsKey !== null && armedPointsKey === pointsKey;
  if (armedPointsKey !== null && !confirmingDeletePoints) setArmedPointsKey(null);
  const measured = doc.regions.length > 0 && stats.size > 0;
  const dqNotApplied = statsMeasured !== null && statsMeasured.excludeDq && statsMeasured.dqExcluded === null;
  const rowClip = { sigma: statsMeasured?.sigma ?? DEFAULT_CLIP_SIGMA, maxiters: statsMeasured?.maxiters ?? DEFAULT_CLIP_ITERS };

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
  const deleteConfirm = useArmedConfirm();
  const { press: pressDelete, blur: blurDelete } = deleteConfirm;
  const handleDelete = useCallback(
    (id: string) => {
      pressDelete(regionDeleteKey(id), () => {
        if (filePath) regionStore.remove(filePath, id);
      });
    },
    [filePath, pressDelete],
  );
  const handleDeleteBlur = useCallback((id: string) => blurDelete(regionDeleteKey(id)), [blurDelete]);
  const handleToggleInclude = useCallback(
    (region: Region) => {
      if (filePath) regionStore.update(filePath, region.id, toggledInclude(region));
    },
    [filePath],
  );

  useEffect(() => {
    setArmedPointsKey(null);
  }, [filePath]);

  useEffect(() => {
    if (!confirmingDeletePoints) return;
    const timer = setTimeout(() => setArmedPointsKey(null), DELETE_POINTS_CONFIRM_MS);
    return () => clearTimeout(timer);
  }, [confirmingDeletePoints]);

  const handleDeletePoints = useCallback(() => {
    setArmedPointsKey(null);
    if (filePath) regionStore.removeShapeKind(filePath, "point");
  }, [filePath]);

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

  const csvContext = useCallback(
    (): RegionCsvContext => ({
      exportedUtc: new Date().toISOString(),
      fileName: provenance.file,
      image: provenance.image,
      bunit: units.bunit,
      summedUnit: units.summedUnit ?? summedFluxUnit(photcal),
      calibration: photcal?.origin || photcal?.label || null,
      excludeDq: statsMeasured?.excludeDq ?? excludeDq,
      dqExcludedPx: statsMeasured?.dqExcluded ?? null,
      regionExcludedPx: statsMeasured?.regionExcluded ?? 0,
      clipSigma: statsMeasured?.sigma ?? DEFAULT_CLIP_SIGMA,
      clipMaxiters: statsMeasured?.maxiters ?? DEFAULT_CLIP_ITERS,
      notes: [...new Set([...statsNotes, ...calibrationWarnings])],
    }),
    [provenance, units, photcal, statsMeasured, excludeDq, statsNotes, calibrationWarnings],
  );

  const copyText = useCallback(() => {
    setIoError(null);
    return doc.regions.length === 0 ? null : regionsTableCsv(doc.regions, stats, csvContext());
  }, [doc.regions, stats, csvContext]);

  const handleSaveCsv = useCallback(async () => {
    if (!filePath || busy || doc.regions.length === 0) return;
    const context = csvContext();
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
      await writeTextFile(target, regionsTableCsv(doc.regions, stats, context));
    } catch (e) {
      setIoError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }, [filePath, measurePath, busy, doc.regions, stats, csvContext]);

  const logReady = regionLogReady(statsMeasured, { regions: doc.regions, excludeDq, sigma, maxiters });
  const statsPending = doc.regions.length > 0 && (!logReady || loading);
  const handleLogRows = useCallback(() => {
    if (!logReady || !statsMeasured) return;
    measurementLog.appendAll(regionStatsEntries(provenance, statsMeasured, stats, photcal));
  }, [logReady, statsMeasured, provenance, stats, photcal]);

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
          {excludeDq &&
            (dqNotApplied ? (
              <span
                className="text-[9px] px-1.5 py-0.5 rounded text-amber-300 bg-amber-900/30"
                title={statsNotes.find((note) => note.startsWith(DQ_NOT_APPLIED_PREFIX)) ?? NO_DQ_MASK_TITLE}
              >
                DQ not applied
              </span>
            ) : (
              <span className="text-[9px] px-1.5 py-0.5 rounded text-amber-300 bg-amber-900/30">DQ masked</span>
            ))}
          {(statsMeasured?.regionExcluded ?? 0) > 0 && (
            <span
              className="text-[9px] px-1.5 py-0.5 rounded text-amber-300 bg-amber-900/30"
              title={`${statsMeasured?.regionExcluded} pixels inside exclude regions are left out of the other regions`}
            >
              regions excluded
            </span>
          )}
          {calibrationBadge}
          {measured && units.bunit !== null && (
            <span
              className="text-[9px] px-1.5 py-0.5 rounded text-zinc-400 bg-zinc-800/60"
              title={`mean, median, sigma, min and max are pixel values in ${units.bunit}; sum and net are sums over pixels in ${units.summedUnit ?? units.bunit}`}
              data-region-unit
            >
              unit {units.bunit}
            </span>
          )}
        </div>
        <div className="flex flex-wrap items-center gap-1">
          {(loading || busy) && <Loader2 size={12} className="animate-spin text-sky-400/70" />}
          <label className="flex items-center gap-1 text-[9px] text-zinc-500" title={CLIP_SIGMA_TITLE}>
            sigma
            <DecimalField
              optional
              value={sigmaText}
              onChange={setSigmaText}
              error={sigmaError}
              aria-label="Sigma clipping threshold"
              className={`${CLIP_INPUT_CLASS} border-zinc-800`}
              title={CLIP_SIGMA_TITLE}
            />
          </label>
          <label className="flex items-center gap-1 text-[9px] text-zinc-500" title={CLIP_ITERS_TITLE}>
            iters
            <DecimalField
              optional
              value={itersText}
              onChange={setItersText}
              error={itersError}
              aria-label="Sigma clipping iterations"
              className={`${CLIP_INPUT_CLASS} border-zinc-800`}
              title={CLIP_ITERS_TITLE}
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
          <CopyCsvButton
            text={copyText}
            disabled={doc.regions.length === 0 || statsPending}
            title={statsPending ? STATS_PENDING_TITLE : COPY_CSV_TITLE}
            className={HEADER_BUTTON_CLASS}
            icon={<ClipboardCopy size={11} />}
            onError={setIoError}
          />
          <button
            onClick={() => void handleSaveCsv()}
            disabled={!filePath || busy || doc.regions.length === 0 || statsPending}
            className={HEADER_BUTTON_CLASS}
            title={statsPending ? STATS_PENDING_TITLE : SAVE_CSV_TITLE}
          >
            <FileDown size={11} /> Save CSV
          </button>
          <button
            type="button"
            onClick={handleLogRows}
            disabled={!logReady || loading}
            className={HEADER_BUTTON_CLASS}
            title="Log one row per region with the statistics on screen; enabled once they match the regions, clipping and DQ settings"
          >
            <Plus size={11} /> Log
          </button>
          {confirmingDeletePoints && pointCount > 0 ? (
            <span role="group" aria-label="Confirm deleting the point regions" className="flex items-center gap-1 text-[10px] text-zinc-300">
              Delete {pointCount} point{pointCount === 1 ? "" : "s"}?
              <button type="button" onClick={handleDeletePoints} className={DANGER_BUTTON_CLASS} title="Remove every Point region of this file">
                Confirm
              </button>
              <button type="button" onClick={() => setArmedPointsKey(null)} className={HEADER_BUTTON_CLASS} title="Keep the point regions">
                Cancel
              </button>
            </span>
          ) : (
            <button
              type="button"
              onClick={() => setArmedPointsKey(pointsKey)}
              disabled={!filePath || pointCount === 0}
              className={HEADER_BUTTON_CLASS}
              title="Remove every Point region of this file at once (points added from the photometry table, targets or catalog); asks for confirmation first"
            >
              <Trash2 size={11} /> Delete points ({pointCount})
            </button>
          )}
        </div>
      </div>
      {sigmaError && (
        <div className={CLIP_ERROR_CLASS} role="status" data-clip-error="sigma">
          {`sigma: ${sigmaError}; using the default ${DEFAULT_CLIP_SIGMA}`}
        </div>
      )}
      {itersError && (
        <div className={CLIP_ERROR_CLASS} role="status" data-clip-error="iters">
          {`iters: ${itersError}; using the default ${DEFAULT_CLIP_ITERS}`}
        </div>
      )}
      <div className="px-3 py-2 space-y-1.5">
        {!filePath && <div className="text-[10px] text-zinc-600">Select a file to draw regions.</div>}
        {filePath && doc.regions.length === 0 && (
          <div className="text-[10px] text-zinc-600">
            Pick a shape tool in the viewer toolbar and drag on the image, or import a .reg file.
          </div>
        )}
        <ErrorAlert compact message={ioError ?? statsError} />
        {warnings.length > 0 && (
          <div className="text-[9px] text-amber-300/90 bg-amber-900/15 border border-amber-800/30 rounded px-2 py-1 space-y-0.5">
            {warnings.map((w, i) => (
              <div key={i}>{w}</div>
            ))}
          </div>
        )}
        {doc.regions.length > 0 && statsNotes.length > 0 && (
          <div
            data-stats-notes
            className="text-[9px] text-amber-300/90 bg-amber-900/15 border border-amber-800/30 rounded px-2 py-1 space-y-0.5"
          >
            {statsNotes.map((note) => (
              <div key={note}>{note}</div>
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
                annuli={backgroundCandidates(doc.regions, r)}
                onSelect={handleSelect}
                onBackground={handleBackground}
                onToggleInclude={handleToggleInclude}
                deleteArmed={deleteConfirm.armedKey === regionDeleteKey(r.id)}
                onDelete={handleDelete}
                onDeleteBlur={handleDeleteBlur}
                clip={rowClip}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

export default memo(RegionsPanel);
