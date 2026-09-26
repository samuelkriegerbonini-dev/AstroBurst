import { memo, useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { ClipboardCopy, Crosshair, Database, Download, Loader2, Shapes } from "lucide-react";
import { catalogConeSearch, catalogCrossMatch, catalogExportCsv, type CatalogExportItems } from "../../services/catalog";
import { GAIA_BANDS } from "../../shared/types/catalog";
import type { CatalogCsvKind, ConeSearchResult, CrossMatchResult, GaiaBand, PlacedCatalogRow } from "../../shared/types/catalog";
import { overlayStore } from "../../utils/overlayStore";
import { regionStore } from "../../utils/regionStore";
import { DEFAULT_REGION_PROPS } from "../../utils/regionPersistence";
import { generateId } from "../../utils/format";
import { catalogCsvFileName, catalogRowsCsv, matchesCsv, sourcesCsv } from "../../utils/catalogCsv";
import { crossMatchEntry, measurementLog } from "../../utils/measurementLog";
import { CATALOG_LAYER_ID, CATALOG_LAYER_KIND, createCatalogPainter } from "../viewer/painters/catalogPainter";
import { ErrorAlert, RunButton, Toggle } from "../ui";
import CrossMatchPlots from "./CrossMatchPlots";

interface CatalogPanelProps {
  filePath: string | null;
}

const DEFAULT_MAG_LIMIT = 18;
const DEFAULT_MAX_ROWS = 5000;
const DEFAULT_MATCH_RADIUS_ARCSEC = 2;
const DEFAULT_DETECTION_SIGMA = 5;
const DEFAULT_MAX_STARS = 500;
const TABLE_ROW_LIMIT = 200;
const SAVED_NOTICE_MS = 6000;

const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-cyan-500/50 w-full";
const SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono";
const SMALL_BUTTON_CLASS =
  "flex items-center gap-1 px-2 py-1 rounded text-[10px] border border-zinc-700/60 text-zinc-300 hover:bg-zinc-800/80 disabled:opacity-40 disabled:cursor-not-allowed";

function fmt(v: number | null | undefined, digits = 2): string {
  return v === null || v === undefined || !Number.isFinite(v) ? "--" : v.toFixed(digits);
}

function parseOptionalNumber(text: string): number | null {
  const v = parseFloat(text);
  return Number.isFinite(v) && v > 0 ? v : null;
}

function parseNumberOr(text: string, fallback: number): number {
  return parseOptionalNumber(text) ?? fallback;
}

function compareByMagnitude(a: PlacedCatalogRow, b: PlacedCatalogRow): number {
  const ga = a.g ?? Number.POSITIVE_INFINITY;
  const gb = b.g ?? Number.POSITIVE_INFINITY;
  return ga - gb;
}

function CatalogPanel({ filePath }: CatalogPanelProps) {
  const radiusId = useId();
  const magLimitId = useId();
  const bandId = useId();
  const matchRadiusId = useId();
  const [radiusText, setRadiusText] = useState("");
  const [magLimitText, setMagLimitText] = useState(String(DEFAULT_MAG_LIMIT));
  const [band, setBand] = useState<GaiaBand>("G");
  const [colourTerm, setColourTerm] = useState(true);
  const [matchRadiusText, setMatchRadiusText] = useState(String(DEFAULT_MATCH_RADIUS_ARCSEC));
  const [cone, setCone] = useState<ConeSearchResult | null>(null);
  const [cross, setCross] = useState<CrossMatchResult | null>(null);
  const [searching, setSearching] = useState(false);
  const [matching, setMatching] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedPath, setSavedPath] = useState<string | null>(null);
  const [overlay, setOverlay] = useState(false);
  const [labels, setLabels] = useState(false);
  const [hoverId, setHoverId] = useState<string | null>(null);

  const requestSeqRef = useRef(0);

  useEffect(() => {
    requestSeqRef.current++;
    setCone(null);
    setCross(null);
    setError(null);
    setHoverId(null);
    setSearching(false);
    setMatching(false);
  }, [filePath]);

  const matchedIds = useMemo(() => new Set((cross?.matches ?? []).map((m) => m.row.id)), [cross]);
  const separations = useMemo(() => {
    const map = new Map<string, number>();
    for (const m of cross?.matches ?? []) map.set(m.row.id, m.sep_arcsec);
    return map;
  }, [cross]);

  const tableRows = useMemo(
    () =>
      (cone?.rows ?? [])
        .filter((r) => r.on_image)
        .sort(compareByMagnitude)
        .slice(0, TABLE_ROW_LIMIT),
    [cone],
  );

  useEffect(() => {
    if (!filePath || !overlay || !cone) return;
    overlayStore.add(filePath, {
      id: CATALOG_LAYER_ID,
      kind: CATALOG_LAYER_KIND,
      visible: true,
      paint: createCatalogPainter({ rows: cone.rows, matchedIds, labels, highlightId: hoverId }),
    });
    return () => overlayStore.remove(filePath, CATALOG_LAYER_ID);
  }, [filePath, overlay, cone, matchedIds, labels, hoverId]);

  const runSearch = useCallback(async (): Promise<ConeSearchResult | null> => {
    if (!filePath) return null;
    const seq = ++requestSeqRef.current;
    setSearching(true);
    setError(null);
    try {
      const result = await catalogConeSearch(filePath, {
        radiusArcmin: parseOptionalNumber(radiusText),
        magLimit: parseNumberOr(magLimitText, DEFAULT_MAG_LIMIT),
        maxRows: DEFAULT_MAX_ROWS,
      });
      if (requestSeqRef.current !== seq) return null;
      setCone(result);
      return result;
    } catch (e: unknown) {
      if (requestSeqRef.current === seq) setError(e instanceof Error ? e.message : String(e));
      return null;
    } finally {
      if (requestSeqRef.current === seq) setSearching(false);
    }
  }, [filePath, radiusText, magLimitText]);

  const runCrossMatch = useCallback(async () => {
    if (!filePath) return;
    const seq = ++requestSeqRef.current;
    setMatching(true);
    setError(null);
    let matched = false;
    try {
      const result = await catalogCrossMatch(filePath, {
        sigma: DEFAULT_DETECTION_SIGMA,
        maxStars: DEFAULT_MAX_STARS,
        radiusArcsec: parseNumberOr(matchRadiusText, DEFAULT_MATCH_RADIUS_ARCSEC),
        band,
        colourTerm,
      });
      if (requestSeqRef.current !== seq) return;
      matched = true;
      setCross(result);
      measurementLog.append(crossMatchEntry(filePath, result, { sigma: DEFAULT_DETECTION_SIGMA, maxStars: DEFAULT_MAX_STARS, colourTerm }));
    } catch (e: unknown) {
      if (requestSeqRef.current === seq) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (requestSeqRef.current === seq) setMatching(false);
    }
    if (matched && !cone) await runSearch();
  }, [filePath, matchRadiusText, band, colourTerm, cone, runSearch]);

  const exportItems = useCallback(
    (kind: CatalogCsvKind): CatalogExportItems | null => {
      if (kind === "catalog") return cone ? { kind, rows: cone.rows } : null;
      if (kind === "sources") return cross ? { kind, rows: cross.sources } : null;
      return cross ? { kind, matches: cross.matches } : null;
    },
    [cone, cross],
  );

  const exportCsv = useCallback(
    async (kind: CatalogCsvKind) => {
      const items = exportItems(kind);
      if (!filePath || !items || exporting) return;
      setExporting(true);
      setError(null);
      try {
        const { save } = await import("@tauri-apps/plugin-dialog");
        const target = await save({
          defaultPath: catalogCsvFileName(filePath, kind),
          filters: [{ name: "CSV", extensions: ["csv"] }],
          title: `Export ${kind} CSV`,
        });
        if (!target) return;
        const result = await catalogExportCsv(filePath, target, items);
        setSavedPath(result.output_path);
        setTimeout(() => setSavedPath(null), SAVED_NOTICE_MS);
      } catch (e: unknown) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setExporting(false);
      }
    },
    [filePath, exportItems, exporting],
  );

  const copyCsv = useCallback(
    async (kind: CatalogCsvKind) => {
      const items = exportItems(kind);
      if (!items) return;
      const text =
        items.kind === "catalog"
          ? catalogRowsCsv(items.rows)
          : items.kind === "sources"
            ? sourcesCsv(items.rows)
            : matchesCsv(items.matches);
      try {
        await navigator.clipboard.writeText(text);
      } catch (e: unknown) {
        setError(`Clipboard copy failed: ${e instanceof Error ? e.message : String(e)}`);
      }
    },
    [exportItems],
  );

  const copyToRegions = useCallback(() => {
    if (!filePath) return;
    for (const row of tableRows) {
      if (row.x === null || row.y === null) continue;
      regionStore.add(filePath, {
        id: generateId(),
        shape: { shape: "point", x: row.x, y: row.y },
        props: { ...DEFAULT_REGION_PROPS, text: row.id },
        backgroundId: null,
      });
    }
  }, [filePath, tableRows]);

  const busy = searching || matching;
  const zp = cross?.zero_point ?? null;
  const astrometry = cross?.astrometry ?? null;
  const warnings = [...(cone?.warnings ?? []), ...(cross?.warnings ?? [])];

  return (
    <div className="ab-panel overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <Database size={12} className="text-cyan-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">Gaia DR3 catalog</span>
        </div>
        {busy && <Loader2 size={12} className="animate-spin text-cyan-400/70" />}
      </div>

      <div className="px-3 py-2 space-y-2">
        <div className="grid grid-cols-2 gap-2">
          <div className="flex flex-col gap-0.5">
            <label htmlFor={radiusId} className="text-[9px] text-zinc-500 uppercase">
              Radius (arcmin)
            </label>
            <input
              id={radiusId}
              type="number"
              min={0.1}
              max={300}
              step={0.5}
              value={radiusText}
              placeholder="auto"
              title="Cone radius in arcmin; blank uses half the image diagonal"
              onChange={(e) => setRadiusText(e.target.value)}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={magLimitId} className="text-[9px] text-zinc-500 uppercase">
              G limit (mag)
            </label>
            <input
              id={magLimitId}
              type="number"
              min={5}
              max={21}
              step={0.5}
              value={magLimitText}
              placeholder={String(DEFAULT_MAG_LIMIT)}
              onChange={(e) => setMagLimitText(e.target.value)}
              className={INPUT_CLASS}
            />
          </div>
        </div>
        <div className="text-[9px] text-zinc-600">blank radius: half the image diagonal</div>

        <RunButton
          label="Search Gaia DR3"
          runningLabel="Querying VizieR..."
          running={searching}
          disabled={!filePath || busy}
          accent="cyan"
          icon={<Database size={12} />}
          onClick={() => void runSearch()}
        />

        <div className="grid grid-cols-3 gap-2 items-end">
          <div className="flex flex-col gap-0.5">
            <label htmlFor={bandId} className="text-[9px] text-zinc-500 uppercase">
              Band
            </label>
            <select id={bandId} value={band} onChange={(e) => setBand(e.target.value as GaiaBand)} className={SELECT_CLASS}>
              {GAIA_BANDS.map((b) => (
                <option key={b} value={b}>
                  {b}
                </option>
              ))}
            </select>
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={matchRadiusId} className="text-[9px] text-zinc-500 uppercase">
              Match radius (")
            </label>
            <input
              id={matchRadiusId}
              type="number"
              min={0.2}
              max={30}
              step={0.5}
              value={matchRadiusText}
              placeholder={String(DEFAULT_MATCH_RADIUS_ARCSEC)}
              onChange={(e) => setMatchRadiusText(e.target.value)}
              className={INPUT_CLASS}
            />
          </div>
          <Toggle label="Colour term" checked={colourTerm} accent="cyan" onChange={setColourTerm} />
        </div>

        <RunButton
          label="Cross-match detected stars"
          runningLabel="Detecting and matching..."
          running={matching}
          disabled={!filePath || busy}
          accent="amber"
          icon={<Crosshair size={12} />}
          onClick={() => void runCrossMatch()}
        />

        <ErrorAlert message={error} />

        {cone && (
          <div className="text-[10px] text-zinc-400 font-mono">
            {cone.n_on_image} of {cone.n_total} rows on image, r = {fmt(cone.radius_arcmin, 1)}&apos;, epoch{" "}
            {cone.epoch_year !== null ? `J${cone.epoch_year.toFixed(2)}` : "J2016.0 (no date)"}
          </div>
        )}

        {cross && (
          <div className="grid grid-cols-2 gap-1.5 text-[10px]">
            <div className="bg-zinc-900/80 rounded px-2 py-1.5 col-span-2">
              <div className="text-zinc-500">Matches</div>
              <div className="text-amber-300 font-mono">
                {cross.matches.length} of {cross.sources.length} measured ({cross.n_detected} detected) within{" "}
                {fmt(cross.match_radius_arcsec, 1)}&quot;
              </div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5 col-span-2">
              <div className="text-zinc-500">Astrometric residuals (star - Gaia)</div>
              <div className="text-zinc-300 font-mono">
                {astrometry
                  ? `median dRA ${fmt(astrometry.median_d_ra_arcsec, 3)}"  dDec ${fmt(astrometry.median_d_dec_arcsec, 3)}"  rms ${fmt(astrometry.rms_arcsec, 3)}"  (n=${astrometry.n})`
                  : "--"}
              </div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5 col-span-2">
              <div className="text-zinc-500">
                Zero point ({cross.band}){cross.photcal_present ? " - informational, image already calibrated" : ""}
              </div>
              <div className="text-cyan-300 font-mono">
                {zp
                  ? `ZP ${fmt(zp.zp, 3)} ± ${fmt(zp.zp_err, 3)}  n=${zp.n_used}, ${zp.n_rejected} rejected, ${zp.n_without_colour} without colour, rms ${fmt(zp.rms, 3)}${
                      zp.colour_coeff !== null ? `, c(BP-RP) = ${fmt(zp.colour_coeff, 4)}` : ", no colour term"
                    }`
                  : "--"}
              </div>
            </div>
          </div>
        )}
        {cross && <CrossMatchPlots cross={cross} />}

        {warnings.length > 0 && (
          <div className="text-[9px] text-amber-300/90 bg-amber-900/15 border border-amber-800/30 rounded px-2 py-1 space-y-0.5">
            {warnings.map((w, i) => (
              <div key={i}>{w}</div>
            ))}
          </div>
        )}

        {tableRows.length > 0 && (
          <div className="max-h-48 overflow-auto border border-zinc-800/60 rounded">
            <table className="w-full text-[9px] font-mono">
              <thead className="sticky top-0 bg-zinc-900 text-zinc-500">
                <tr>
                  <th className="text-left px-1.5 py-0.5">id</th>
                  <th className="text-right px-1.5 py-0.5">G</th>
                  <th className="text-right px-1.5 py-0.5">BP-RP</th>
                  <th className="text-right px-1.5 py-0.5">sep&quot;</th>
                  <th className="text-right px-1.5 py-0.5">x</th>
                  <th className="text-right px-1.5 py-0.5">y</th>
                </tr>
              </thead>
              <tbody>
                {tableRows.map((row) => {
                  const matched = matchedIds.has(row.id);
                  return (
                    <tr
                      key={row.id}
                      onMouseEnter={() => setHoverId(row.id)}
                      onMouseLeave={() => setHoverId(null)}
                      className={`${matched ? "text-amber-200" : "text-zinc-300"} ${hoverId === row.id ? "bg-zinc-800/80" : ""}`}
                    >
                      <td className="px-1.5 py-0.5 truncate max-w-[120px]" title={row.id}>
                        {row.id}
                      </td>
                      <td className="text-right px-1.5 py-0.5">{fmt(row.g)}</td>
                      <td className="text-right px-1.5 py-0.5">{fmt(row.bp_rp)}</td>
                      <td className="text-right px-1.5 py-0.5">{fmt(separations.get(row.id) ?? null)}</td>
                      <td className="text-right px-1.5 py-0.5">{fmt(row.x, 1)}</td>
                      <td className="text-right px-1.5 py-0.5">{fmt(row.y, 1)}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
            {cone && cone.n_on_image > TABLE_ROW_LIMIT && (
              <div className="text-[9px] text-zinc-600 px-1.5 py-0.5">
                brightest {TABLE_ROW_LIMIT} of {cone.n_on_image} rows shown; exports include every row
              </div>
            )}
          </div>
        )}

        {cone && (
          <div className="flex flex-wrap items-center gap-1.5">
            <Toggle label="Overlay" checked={overlay} accent="cyan" onChange={setOverlay} />
            <Toggle label="Labels" checked={labels} disabled={!overlay} accent="cyan" onChange={setLabels} />
            <button type="button" onClick={copyToRegions} disabled={tableRows.length === 0} className={SMALL_BUTTON_CLASS} title="Add the listed rows as Point regions named by their Gaia id">
              <Shapes size={10} />
              Copy to regions
            </button>
          </div>
        )}

        {(cone || cross) && (
          <div className="flex flex-wrap items-center gap-1.5">
            <button type="button" onClick={() => void exportCsv("catalog")} disabled={!cone || exporting} className={SMALL_BUTTON_CLASS}>
              <Download size={10} />
              Catalog CSV
            </button>
            <button type="button" onClick={() => void exportCsv("sources")} disabled={!cross || exporting} className={SMALL_BUTTON_CLASS}>
              <Download size={10} />
              Sources CSV
            </button>
            <button type="button" onClick={() => void exportCsv("matches")} disabled={!cross || exporting} className={SMALL_BUTTON_CLASS}>
              <Download size={10} />
              Matches CSV
            </button>
            <button type="button" onClick={() => void copyCsv(cross ? "matches" : "catalog")} className={SMALL_BUTTON_CLASS} title="Copy the matches (or the catalog when nothing is matched) as CSV">
              <ClipboardCopy size={10} />
              Copy CSV
            </button>
          </div>
        )}

        {savedPath && <div className="text-[9px] text-emerald-400/90 break-all">Saved {savedPath}</div>}

        {!cone && !cross && !error && (
          <div className="text-[10px] text-zinc-600">
            Cone search of Gaia DR3 through VizieR around the image centre with proper motions propagated to the
            observation date, full-field cross-match of detected stars with astrometric residuals and a photometric
            zero point with an optional BP-RP colour term. Requires a WCS.
          </div>
        )}
      </div>
    </div>
  );
}

export default memo(CatalogPanel);
