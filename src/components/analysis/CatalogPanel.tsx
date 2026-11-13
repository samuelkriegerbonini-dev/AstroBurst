import { memo, useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { Crosshair, Database, Download, Loader2, Shapes } from "lucide-react";
import { catalogConeSearch, catalogCrossMatch, catalogExportCsv, type CatalogExportItems } from "../../services/catalog";
import { useTimedNotice } from "../../hooks/useTimedNotice";
import { GAIA_BANDS } from "../../shared/types/catalog";
import type { CatalogCsvKind, ConeSearchResult, CrossMatchResult, GaiaBand, PlacedCatalogRow, ZeroPointFit } from "../../shared/types/catalog";
import { APERTURE_RANGE_HINT, apertureRadiusInRange } from "../../utils/photometryTable";
import { overlayStore } from "../../utils/overlayStore";
import { regionStore } from "../../utils/regionStore";
import { DEFAULT_REGION_PROPS } from "../../utils/regionPersistence";
import { generateId } from "../../utils/format";
import { targetProjectionPath } from "../../utils/skyList";
import { catalogCsvFileName } from "../../utils/catalogCsv";
import { crossMatchEntry, fileNameOf, measurementLog } from "../../utils/measurementLog";
import {
  catalogueCsvText,
  coneCsvHeaderLines,
  crossMatchCsvHeaderLines,
  detectionLine,
  gaiaEpochText,
  gaiaListLine,
  zeroPointApertureText,
} from "../../utils/catalogProvenance";
import { APP_VERSION } from "../../utils/constants";
import { CATALOG_LAYER_ID, CATALOG_LAYER_KIND, createCatalogPainter } from "../viewer/painters/catalogPainter";
import { CopyCsvButton, DecimalField, ErrorAlert, RunButton, SMALL_BUTTON_CLASS, Toggle } from "../ui";
import { decimalFieldState, firstFieldError, type DecimalFieldOptions } from "../../utils/decimalText";
import CrossMatchPlots from "./CrossMatchPlots";

interface CatalogPanelProps {
  filePath: string | null;
  measurePath?: string | null;
  measureKey?: string | null;
}

const DEFAULT_MAG_LIMIT = 18;
const DEFAULT_MAX_ROWS = 5000;
const DEFAULT_MATCH_RADIUS_ARCSEC = 2;
export const DEFAULT_DETECTION_SIGMA = 5;
export const DEFAULT_MAX_STARS = 500;
const TABLE_ROW_LIMIT = 200;
const SAVED_NOTICE_MS = 6000;
const NO_IMAGE_REASON = "Select an image first. Gaia DR3 search needs its WCS, from the header or solved in Stars.";
const RADIUS_FIELD: DecimalFieldOptions = { optional: true, min: 0.1 };
const MAG_LIMIT_FIELD: DecimalFieldOptions = { optional: true, min: 5 };
const MATCH_RADIUS_FIELD: DecimalFieldOptions = { optional: true, min: 0.2 };
const APERTURE_FIELD: DecimalFieldOptions = { optional: true };
const APERTURE_TITLE = "Aperture radius in pixels for the zero point; blank uses 1.5 x FWHM of each star";
const ZERO_POINT_APERTURE_TITLE = "The zero point holds only for instrumental magnitudes measured in this aperture";
const ZERO_POINT_SYSTEM_TITLE = "Gaia magnitudes are in the Vega system: this is not an AB zero point";
const LIST_RULE_FIELD = "Cross-match list: every Gaia star in the field, no G limit.";
const LIST_RULE_SEARCH = "Cross-match list: the radius and G limit typed above.";
const COPY_TITLE = "Copy the matches (or the catalog when nothing is matched) as CSV, with the header lines of the saved file";

const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-cyan-500/50 w-full";
const SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono";

function fmt(v: number | null | undefined, digits = 2): string {
  return v === null || v === undefined || !Number.isFinite(v) ? "--" : v.toFixed(digits);
}

function zeroPointUseText(band: GaiaBand, zp: ZeroPointFit): string {
  const coeff = zp.colour_coeff;
  const colour = coeff === null ? "" : ` ${coeff < 0 ? "-" : "+"} ${Math.abs(coeff).toFixed(4)} x (BP-RP)`;
  return `Not applied to the image. Gaia ${band} = mag_inst + ${zp.zp.toFixed(3)}${colour} for a star measured in this aperture.`;
}

function compareByMagnitude(a: PlacedCatalogRow, b: PlacedCatalogRow): number {
  const ga = a.g ?? Number.POSITIVE_INFINITY;
  const gb = b.g ?? Number.POSITIVE_INFINITY;
  return ga - gb;
}

function CatalogPanel({ filePath, measurePath = null, measureKey = null }: CatalogPanelProps) {
  const wcsPath = targetProjectionPath(filePath, measurePath);
  const radiusId = useId();
  const magLimitId = useId();
  const bandId = useId();
  const matchRadiusId = useId();
  const apertureId = useId();
  const reasonId = useId();
  const [radiusText, setRadiusText] = useState("");
  const [magLimitText, setMagLimitText] = useState(String(DEFAULT_MAG_LIMIT));
  const [band, setBand] = useState<GaiaBand>("G");
  const [colourTerm, setColourTerm] = useState(true);
  const [sameList, setSameList] = useState(false);
  const [matchRadiusText, setMatchRadiusText] = useState(String(DEFAULT_MATCH_RADIUS_ARCSEC));
  const [apertureRadiusText, setApertureRadiusText] = useState("");
  const [cone, setCone] = useState<ConeSearchResult | null>(null);
  const [cross, setCross] = useState<CrossMatchResult | null>(null);
  const [searching, setSearching] = useState(false);
  const [matching, setMatching] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedPath, showSavedPath] = useTimedNotice<string>(SAVED_NOTICE_MS);
  const [overlay, setOverlay] = useState(false);
  const [labels, setLabels] = useState(false);
  const [hoverId, setHoverId] = useState<string | null>(null);

  const requestSeqRef = useRef(0);
  const radiusField = decimalFieldState(radiusText, RADIUS_FIELD);
  const magLimitField = decimalFieldState(magLimitText, MAG_LIMIT_FIELD);
  const matchRadiusField = decimalFieldState(matchRadiusText, MATCH_RADIUS_FIELD);
  const apertureField = decimalFieldState(apertureRadiusText, APERTURE_FIELD);
  const radiusArcmin = radiusField.value;
  const magLimit = magLimitField.value ?? DEFAULT_MAG_LIMIT;
  const matchRadiusArcsec = matchRadiusField.value ?? DEFAULT_MATCH_RADIUS_ARCSEC;
  const apertureRadius = apertureField.value ?? undefined;
  const apertureOutOfRange = !apertureRadiusInRange(apertureRadius);
  const searchFieldError = firstFieldError([
    ["Radius (arcmin)", radiusField],
    ["G limit (mag)", magLimitField],
  ]);
  const matchFieldError = firstFieldError([
    ['Match radius (")', matchRadiusField],
    ["Aperture r (px)", apertureField],
  ]);

  useEffect(() => {
    requestSeqRef.current++;
    setCone(null);
    setCross(null);
    setError(null);
    setHoverId(null);
    setSearching(false);
    setMatching(false);
  }, [filePath, wcsPath, measureKey]);

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
    if (!wcsPath) return null;
    if (searchFieldError !== null) {
      setError(searchFieldError);
      return null;
    }
    const seq = ++requestSeqRef.current;
    setSearching(true);
    setError(null);
    try {
      const result = await catalogConeSearch(wcsPath, {
        radiusArcmin,
        magLimit,
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
  }, [wcsPath, searchFieldError, radiusArcmin, magLimit]);

  const runCrossMatch = useCallback(async () => {
    if (!wcsPath || matchFieldError !== null || apertureOutOfRange || (sameList && searchFieldError !== null)) return;
    const seq = ++requestSeqRef.current;
    setMatching(true);
    setError(null);
    let matched = false;
    try {
      const result = await catalogCrossMatch(wcsPath, {
        sigma: DEFAULT_DETECTION_SIGMA,
        maxStars: DEFAULT_MAX_STARS,
        radiusArcsec: matchRadiusArcsec,
        band,
        colourTerm,
        apertureRadius,
        ...(sameList ? { coneRadiusArcmin: radiusArcmin, magLimit, maxRows: DEFAULT_MAX_ROWS } : {}),
      });
      if (requestSeqRef.current !== seq) return;
      matched = true;
      setCross(result);
      measurementLog.append(crossMatchEntry(filePath ?? wcsPath, result, { sigma: DEFAULT_DETECTION_SIGMA, maxStars: DEFAULT_MAX_STARS, colourTerm }));
    } catch (e: unknown) {
      if (requestSeqRef.current === seq) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (requestSeqRef.current === seq) setMatching(false);
    }
    if (matched && !cone) await runSearch();
  }, [filePath, wcsPath, matchFieldError, apertureOutOfRange, sameList, searchFieldError, matchRadiusArcsec, band, colourTerm, apertureRadius, radiusArcmin, magLimit, cone, runSearch]);

  const exportItems = useCallback(
    (kind: CatalogCsvKind): CatalogExportItems | null => {
      if (kind === "catalog") return cone ? { kind, rows: cone.rows } : null;
      if (kind === "sources") return cross ? { kind, rows: cross.sources } : null;
      return cross ? { kind, matches: cross.matches } : null;
    },
    [cone, cross],
  );

  const headerLinesOf = useCallback(
    (kind: CatalogCsvKind): string[] | null => {
      const context = { appVersion: APP_VERSION, exportedUtc: new Date().toISOString(), fileName: fileNameOf(wcsPath) };
      if (kind === "catalog") return cone ? coneCsvHeaderLines(cone, context) : null;
      return cross ? crossMatchCsvHeaderLines(kind, cross, context) : null;
    },
    [wcsPath, cone, cross],
  );

  const exportCsv = useCallback(
    async (kind: CatalogCsvKind) => {
      const items = exportItems(kind);
      if (!wcsPath || !items || exporting) return;
      setExporting(true);
      setError(null);
      try {
        const { save } = await import("@tauri-apps/plugin-dialog");
        const target = await save({
          defaultPath: catalogCsvFileName(wcsPath, kind),
          filters: [{ name: "CSV", extensions: ["csv"] }],
          title: `Export ${kind} CSV`,
        });
        if (!target) return;
        const result = await catalogExportCsv(wcsPath, target, items, headerLinesOf(kind) ?? undefined);
        showSavedPath(result.output_path);
      } catch (e: unknown) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setExporting(false);
      }
    },
    [wcsPath, exportItems, exporting, showSavedPath, headerLinesOf],
  );

  const copyText = useCallback((): string | null => {
    const items = exportItems(cross ? "matches" : "catalog");
    const lines = items ? headerLinesOf(items.kind) : null;
    return items && lines ? catalogueCsvText(items, lines) : null;
  }, [exportItems, headerLinesOf, cross]);

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
  const noImageReason = wcsPath ? null : NO_IMAGE_REASON;
  const zp = cross?.zero_point ?? null;
  const astrometry = cross?.astrometry ?? null;
  const warnings = [...(cone?.warnings ?? []), ...(cross?.warnings ?? [])];
  const listLine = cross ? gaiaListLine(cross) : null;
  const detection = cross ? detectionLine(cross) : null;

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
            <DecimalField
              id={radiusId}
              {...RADIUS_FIELD}
              value={radiusText}
              placeholder="auto"
              title="Cone radius in arcmin; blank uses half the image diagonal"
              onChange={setRadiusText}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={magLimitId} className="text-[9px] text-zinc-500 uppercase">
              G limit (mag)
            </label>
            <DecimalField
              id={magLimitId}
              {...MAG_LIMIT_FIELD}
              value={magLimitText}
              placeholder={String(DEFAULT_MAG_LIMIT)}
              onChange={setMagLimitText}
              className={INPUT_CLASS}
            />
          </div>
        </div>
        <div className="text-[9px] text-zinc-600">blank radius: half the image diagonal</div>
        {searchFieldError && <div className="text-[9px] text-amber-400/90" data-field-error>{searchFieldError}</div>}

        <div title={noImageReason ?? undefined}>
          <RunButton
            label="Search Gaia DR3"
            runningLabel="Querying VizieR..."
            running={searching}
            disabled={!wcsPath || busy || searchFieldError !== null}
            accent="cyan"
            icon={<Database size={12} />}
            describedBy={noImageReason ? reasonId : undefined}
            onClick={() => void runSearch()}
          />
        </div>
        {noImageReason && <div id={reasonId} data-run-blocked className="text-[9px] text-zinc-500">{noImageReason}</div>}

        <div className="grid grid-cols-2 gap-2 items-end">
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
            <DecimalField
              id={matchRadiusId}
              {...MATCH_RADIUS_FIELD}
              value={matchRadiusText}
              placeholder={String(DEFAULT_MATCH_RADIUS_ARCSEC)}
              onChange={setMatchRadiusText}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={apertureId} className="text-[9px] text-zinc-500 uppercase">
              Aperture r (px)
            </label>
            <DecimalField
              id={apertureId}
              {...APERTURE_FIELD}
              value={apertureRadiusText}
              placeholder="auto"
              title={APERTURE_TITLE}
              onChange={setApertureRadiusText}
              className={INPUT_CLASS}
            />
          </div>
          <Toggle label="Colour term" checked={colourTerm} accent="cyan" onChange={setColourTerm} />
          <div className="col-span-2">
            <Toggle label="Same Gaia list as the search" checked={sameList} accent="cyan" onChange={setSameList} />
            <div className="text-[9px] text-zinc-500" data-list-rule>{sameList ? LIST_RULE_SEARCH : LIST_RULE_FIELD}</div>
          </div>
        </div>

        {matchFieldError && <div className="text-[9px] text-amber-400/90" data-field-error>{matchFieldError}</div>}
        {apertureOutOfRange && <div className="text-[9px] text-amber-400/90">{APERTURE_RANGE_HINT}</div>}

        <div title={noImageReason ?? undefined}>
          <RunButton
            label="Cross-match detected stars"
            runningLabel="Detecting and matching..."
            running={matching}
            disabled={!wcsPath || busy || matchFieldError !== null || apertureOutOfRange || (sameList && searchFieldError !== null)}
            accent="amber"
            icon={<Crosshair size={12} />}
            describedBy={noImageReason ? reasonId : undefined}
            onClick={() => void runCrossMatch()}
          />
        </div>

        <ErrorAlert message={error} />

        {cone && (
          <div className="text-[10px] text-zinc-400 font-mono">
            {cone.n_on_image} of {cone.n_total} rows on image, r = {fmt(cone.radius_arcmin, 1)}&apos;, epoch{" "}
            {gaiaEpochText(cone.epoch_year)}
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
              {listLine && <div className="text-zinc-400 font-mono" data-gaia-list>{listLine}</div>}
              {detection && <div className="text-zinc-400 font-mono" data-detection>{detection}</div>}
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
              <div className="text-zinc-500" title={ZERO_POINT_SYSTEM_TITLE}>
                Zero point (Gaia {cross.band}, Vega){cross.photcal_present ? " - informational, image already calibrated" : ""}
              </div>
              <div className="text-cyan-300 font-mono">
                {zp
                  ? `ZP ${fmt(zp.zp, 3)} ± ${fmt(zp.zp_err, 3)}  n=${zp.n_used}, ${zp.n_rejected} rejected, ${zp.n_without_colour} without colour, rms ${fmt(zp.rms, 3)}${
                      zp.colour_coeff !== null ? `, c(BP-RP) = ${fmt(zp.colour_coeff, 4)}` : ", no colour term"
                    }`
                  : "--"}
              </div>
              {zp && cross.aperture && (
                <div className="text-zinc-400 font-mono" title={ZERO_POINT_APERTURE_TITLE}>
                  {zeroPointApertureText(cross.aperture)}
                </div>
              )}
              {zp && <div className="text-zinc-400" data-zp-use>{zeroPointUseText(cross.band, zp)}</div>}
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
            <CopyCsvButton text={copyText} title={COPY_TITLE} onError={setError} />
          </div>
        )}

        {savedPath && <div className="text-[9px] text-emerald-400/90 break-all">Saved {savedPath}</div>}

        {!cone && !cross && !error && (
          <div className="text-[10px] text-zinc-600">
            Cone search of Gaia DR3 through VizieR around the image centre with proper motions propagated to the
            observation date, full-field cross-match of detected stars with astrometric residuals and a photometric
            zero point in the Vega system of the Gaia band, with an optional BP-RP colour term. Requires a WCS.
          </div>
        )}
      </div>
    </div>
  );
}

export default memo(CatalogPanel);
