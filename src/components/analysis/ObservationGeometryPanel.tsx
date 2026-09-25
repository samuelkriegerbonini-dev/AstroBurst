import { memo, useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { ClipboardCopy, Compass, Loader2 } from "lucide-react";
import { getObservationGeometry } from "../../services/geometry";
import type { ObservationGeometryResult } from "../../shared/types/geometry";
import {
  NO_SITE_HINT,
  NO_TARGET_HINT,
  NO_TIME_HINT,
  geometryRows,
  geometryText,
  loadSiteOverride,
  overridesFor,
  parseSiteInputs,
  parseTargetInputs,
  saveSiteOverride,
  type GeometryRow,
  type SiteOverride,
  type StorageLike,
  type TargetOverride,
} from "../../utils/observationGeometry";
import { ErrorAlert } from "../ui";

interface ObservationGeometryPanelProps {
  filePath: string | null;
}

const EMPTY_HINT = "Load a file to compute its observation geometry.";
const SITE_MEMORY_HINT =
  "A site typed here is remembered for every file and replaces the header site (OBSGEO-X/Y/Z, OBSGEO-L/B/H or SITELAT/SITELONG-style cards, read east-positive); the Notes name the header site it replaced.";
const TARGET_SESSION_HINT = "A target typed here is used for this file only.";
const TIME_ROW_KEYS = ["jd_utc", "jd_tt", "jd_tdb", "bjd_tdb", "hjd_utc"];
const TARGET_PLACEHOLDER_DIGITS = 6;
const SITE_PLACEHOLDER_DIGITS = 4;

const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-amber-500/50 w-full";
const SMALL_BUTTON_CLASS =
  "flex items-center gap-1 px-2 py-1 rounded text-[10px] border border-zinc-700/60 text-zinc-300 hover:bg-zinc-800/80 disabled:opacity-40 disabled:cursor-not-allowed";
const SECTION_CLASS = "text-[9px] text-zinc-500 uppercase tracking-wider";
const CHIP_CLASS = "inline-block rounded px-1 py-px text-[8px] leading-tight ml-1 bg-zinc-800 text-zinc-400";
const HINT_CLASS = "text-[9px] text-zinc-600";
const WARN_CLASS = "text-[9px] text-amber-400/90";
const SOURCE_CLASS = "text-[9px] text-zinc-500 truncate";

function browserStorage(): StorageLike | null {
  try {
    return typeof window !== "undefined" ? window.localStorage : null;
  } catch {
    return null;
  }
}

function placeholderOf(value: number | null | undefined, digits: number): string {
  return typeof value === "number" && Number.isFinite(value) ? value.toFixed(digits) : "";
}

function RowLine({ row, chip }: { row: GeometryRow; chip?: string | null }) {
  return (
    <div className="grid grid-cols-[1fr_auto] items-center gap-2 text-[10px]">
      <span className="text-zinc-500 truncate">
        {row.label}
        {chip && <span className={CHIP_CLASS}>{chip}</span>}
      </span>
      <span className="text-zinc-300 font-mono" title={row.hint}>
        {row.value}
        {row.unit ? ` ${row.unit}` : ""}
      </span>
    </div>
  );
}

function ObservationGeometryPanel({ filePath }: ObservationGeometryPanelProps) {
  const raId = useId();
  const decId = useId();
  const latId = useId();
  const lonId = useId();
  const heightId = useId();
  const [siteOverride, setSiteOverride] = useState<SiteOverride | null>(() => loadSiteOverride(browserStorage()));
  const [latText, setLatText] = useState(() => (siteOverride ? String(siteOverride.lat) : ""));
  const [lonText, setLonText] = useState(() => (siteOverride ? String(siteOverride.lon) : ""));
  const [heightText, setHeightText] = useState(() => (siteOverride ? String(siteOverride.height) : ""));
  const [siteError, setSiteError] = useState<string | null>(null);
  const [targetOverride, setTargetOverride] = useState<TargetOverride | null>(null);
  const [raText, setRaText] = useState("");
  const [decText, setDecText] = useState("");
  const [targetError, setTargetError] = useState<string | null>(null);
  const [result, setResult] = useState<ObservationGeometryResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const seqRef = useRef(0);

  useEffect(() => {
    seqRef.current++;
    setResult(null);
    setError(null);
    setLoading(false);
    setTargetOverride(null);
    setRaText("");
    setDecText("");
    setTargetError(null);
  }, [filePath]);

  const overrides = useMemo(() => overridesFor(targetOverride, siteOverride), [targetOverride, siteOverride]);

  useEffect(() => {
    if (!filePath) return;
    const seq = ++seqRef.current;
    setLoading(true);
    setError(null);
    void (async () => {
      try {
        const res = await getObservationGeometry(filePath, overrides);
        if (seq !== seqRef.current) return;
        setResult(res);
      } catch (e: unknown) {
        if (seq === seqRef.current) setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (seq === seqRef.current) setLoading(false);
      }
    })();
  }, [filePath, overrides]);

  const useTarget = useCallback(() => {
    const parsed = parseTargetInputs(raText, decText);
    setTargetError(parsed.error);
    if (parsed.error === null) setTargetOverride(parsed.target);
  }, [raText, decText]);

  const resetTarget = useCallback(() => {
    setRaText("");
    setDecText("");
    setTargetError(null);
    setTargetOverride(null);
  }, []);

  const useSite = useCallback(() => {
    const parsed = parseSiteInputs(latText, lonText, heightText);
    setSiteError(parsed.error);
    if (parsed.error !== null) return;
    saveSiteOverride(browserStorage(), parsed.site);
    setSiteOverride(parsed.site);
  }, [latText, lonText, heightText]);

  const forgetSite = useCallback(() => {
    saveSiteOverride(browserStorage(), null);
    setSiteOverride(null);
    setLatText("");
    setLonText("");
    setHeightText("");
    setSiteError(null);
  }, []);

  const copy = useCallback(async () => {
    if (!result) return;
    try {
      await navigator.clipboard.writeText(geometryText(result));
    } catch (e: unknown) {
      setError(`Clipboard copy failed: ${e instanceof Error ? e.message : String(e)}`);
    }
  }, [result]);

  const rows = useMemo(() => (result ? geometryRows(result) : []), [result]);
  const timeRows = rows.filter((r) => TIME_ROW_KEYS.includes(r.key));
  const skyRows = rows.filter((r) => !TIME_ROW_KEYS.includes(r.key));
  const hasTime = result !== null && result.geometry.jd_utc !== null;
  const bjdSource = result?.geometry.bjd_source ?? null;
  const notes = result ? [...result.notes, ...result.geometry.time_scale_notes] : [];

  return (
    <div className="ab-panel overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <Compass size={12} className="text-amber-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">Observation geometry</span>
        </div>
        {loading && <Loader2 size={12} className="animate-spin text-amber-400/70" />}
      </div>

      <div className="px-3 py-2 space-y-2">
        {!filePath ? (
          <div className="text-[10px] text-zinc-600">{EMPTY_HINT}</div>
        ) : (
          <>
            <div className={SECTION_CLASS}>Target</div>
            <div className="grid grid-cols-2 gap-1.5">
              <div className="flex flex-col gap-0.5">
                <label htmlFor={raId} className="text-[9px] text-zinc-500">
                  RA deg
                </label>
                <input
                  id={raId}
                  type="text"
                  inputMode="decimal"
                  value={raText}
                  placeholder={targetOverride ? "" : placeholderOf(result?.target?.ra_deg, TARGET_PLACEHOLDER_DIGITS)}
                  onChange={(e) => setRaText(e.target.value)}
                  className={INPUT_CLASS}
                />
              </div>
              <div className="flex flex-col gap-0.5">
                <label htmlFor={decId} className="text-[9px] text-zinc-500">
                  Dec deg
                </label>
                <input
                  id={decId}
                  type="text"
                  inputMode="decimal"
                  value={decText}
                  placeholder={targetOverride ? "" : placeholderOf(result?.target?.dec_deg, TARGET_PLACEHOLDER_DIGITS)}
                  onChange={(e) => setDecText(e.target.value)}
                  className={INPUT_CLASS}
                />
              </div>
            </div>
            {targetError && <div className={WARN_CLASS}>{targetError}</div>}
            {result &&
              (result.target ? (
                <div className={SOURCE_CLASS} title={result.target.source}>
                  from {result.target.source}
                </div>
              ) : (
                <div className={WARN_CLASS}>{NO_TARGET_HINT}</div>
              ))}
            <div className="flex items-center gap-1.5">
              <button type="button" onClick={useTarget} className={SMALL_BUTTON_CLASS}>
                Use target
              </button>
              <button type="button" onClick={resetTarget} disabled={!targetOverride && raText === "" && decText === ""} className={SMALL_BUTTON_CLASS}>
                Reset
              </button>
              <span className={HINT_CLASS}>{TARGET_SESSION_HINT}</span>
            </div>

            <div className={SECTION_CLASS}>Site</div>
            <div className="grid grid-cols-3 gap-1.5">
              <div className="flex flex-col gap-0.5">
                <label htmlFor={latId} className="text-[9px] text-zinc-500">
                  lat deg
                </label>
                <input
                  id={latId}
                  type="text"
                  inputMode="decimal"
                  value={latText}
                  placeholder={placeholderOf(result?.site?.lat_deg, SITE_PLACEHOLDER_DIGITS)}
                  onChange={(e) => setLatText(e.target.value)}
                  className={INPUT_CLASS}
                />
              </div>
              <div className="flex flex-col gap-0.5">
                <label htmlFor={lonId} className="text-[9px] text-zinc-500">
                  lon deg (E+)
                </label>
                <input
                  id={lonId}
                  type="text"
                  inputMode="decimal"
                  value={lonText}
                  placeholder={placeholderOf(result?.site?.lon_deg, SITE_PLACEHOLDER_DIGITS)}
                  onChange={(e) => setLonText(e.target.value)}
                  className={INPUT_CLASS}
                />
              </div>
              <div className="flex flex-col gap-0.5">
                <label htmlFor={heightId} className="text-[9px] text-zinc-500">
                  height m
                </label>
                <input
                  id={heightId}
                  type="text"
                  inputMode="decimal"
                  value={heightText}
                  placeholder={placeholderOf(result?.site?.height_m, 0)}
                  onChange={(e) => setHeightText(e.target.value)}
                  className={INPUT_CLASS}
                />
              </div>
            </div>
            {siteError && <div className={WARN_CLASS}>{siteError}</div>}
            {result &&
              (result.site ? (
                <div className={SOURCE_CLASS} title={result.site.source}>
                  from {result.site.source}
                </div>
              ) : (
                <div className={WARN_CLASS}>{NO_SITE_HINT}</div>
              ))}
            <div className="flex items-center gap-1.5">
              <button type="button" onClick={useSite} className={SMALL_BUTTON_CLASS}>
                Use site
              </button>
              <button type="button" onClick={forgetSite} disabled={!siteOverride} className={SMALL_BUTTON_CLASS}>
                Forget site
              </button>
            </div>
            <div className={HINT_CLASS}>{SITE_MEMORY_HINT}</div>

            <ErrorAlert message={error} />

            {result && (
              <>
                <div className={SECTION_CLASS}>Time</div>
                {hasTime ? (
                  <div className="space-y-0.5">
                    {timeRows.map((row) => (
                      <RowLine key={row.key} row={row} chip={row.key === "bjd_tdb" ? bjdSource : null} />
                    ))}
                  </div>
                ) : (
                  <div className={WARN_CLASS}>{NO_TIME_HINT}</div>
                )}
                {result.time_source && (
                  <div className={SOURCE_CLASS} title={result.time_source}>
                    time: {result.time_source}
                  </div>
                )}

                <div className={SECTION_CLASS}>Sky</div>
                <div className="space-y-0.5">
                  {skyRows.map((row) => (
                    <RowLine key={row.key} row={row} />
                  ))}
                </div>

                {notes.length > 0 && (
                  <details className="text-[9px] text-zinc-500">
                    <summary className="cursor-pointer select-none">Notes</summary>
                    <ul className="mt-1 space-y-0.5 list-disc pl-4">
                      {notes.map((note) => (
                        <li key={note}>{note}</li>
                      ))}
                    </ul>
                  </details>
                )}

                <div className="flex flex-wrap items-center gap-1.5">
                  <button type="button" onClick={() => void copy()} className={SMALL_BUTTON_CLASS}>
                    <ClipboardCopy size={10} />
                    Copy
                  </button>
                  <span className="text-[9px] text-zinc-500 font-mono ml-auto">{result.elapsed_ms} ms</span>
                </div>
              </>
            )}
          </>
        )}
      </div>
    </div>
  );
}

export default memo(ObservationGeometryPanel);
