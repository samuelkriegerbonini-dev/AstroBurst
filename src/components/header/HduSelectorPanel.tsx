import { useState, useCallback, useEffect, useMemo, useRef } from "react";
import { useTimedNotice } from "../../hooks/useTimedNotice";
import { Database, ChevronRight, FileText, Search, X, Copy, Check, Eye, Loader2 } from "lucide-react";
import { getFitsExtensions, getHeaderByHdu } from "../../services/header";
import type { FitsExtension } from "../../services/header";
import type { PlaneInfo } from "../../shared/types/fits.types";
import { bandButtonLabel, hduLayerName, roleChip, rowActions, shownLayerText, type ChipTone, type PendingLayer } from "../../utils/layerLabels";
import CompanionAssign from "./CompanionAssign";

const COPIED_FEEDBACK_MS = 1500;

interface HduSelectorPanelProps {
  filePath: string;
  activePath: string;
  activePlane?: PlaneInfo | null;
  sourceOfProduct?: boolean;
  onSelectHdu?: (hduIndex: number, header: Record<string, string>) => void;
  onActivate?: (ref: string) => void;
  activating?: boolean;
  activateError?: string | null;
}

type HduInfo = FitsExtension;

const TYPE_STYLES: Record<string, { bg: string; border: string; text: string }> = {
  IMAGE: { bg: "rgba(59,130,246,0.1)", border: "rgba(59,130,246,0.25)", text: "#93c5fd" },
  ARRAY: { bg: "rgba(59,130,246,0.1)", border: "rgba(59,130,246,0.25)", text: "#93c5fd" },
  TABLE: { bg: "rgba(245,158,11,0.1)", border: "rgba(245,158,11,0.25)", text: "#fcd34d" },
  BINTABLE: { bg: "rgba(245,158,11,0.1)", border: "rgba(245,158,11,0.25)", text: "#fcd34d" },
  PRIMARY: { bg: "rgba(139,92,246,0.1)", border: "rgba(139,92,246,0.25)", text: "#c4b5fd" },
};

const DEFAULT_TYPE_STYLE = { bg: "rgba(113,113,122,0.1)", border: "rgba(113,113,122,0.2)", text: "#a1a1aa" };
const SHOWN_BADGE = { text: "shown", title: "Currently displayed plane" };
const SOURCE_BADGE = { text: "source", title: "Source plane of the displayed product" };
const DQ_TAG_STYLE = { bg: "rgba(248,113,113,0.12)", border: "rgba(248,113,113,0.3)", text: "#f87171" };
const ERR_TAG_STYLE = { bg: "rgba(251,191,36,0.12)", border: "rgba(251,191,36,0.3)", text: "#fbbf24" };
const TAG_STYLES: Record<ChipTone, typeof DQ_TAG_STYLE> = { dq: DQ_TAG_STYLE, err: ERR_TAG_STYLE };
const TEAL_BADGE_STYLE = { background: "rgba(20,184,166,0.12)", border: "1px solid rgba(20,184,166,0.3)", color: "var(--ab-teal)" };
const ROW_BUTTON_CLASS =
  "text-[9px] font-medium px-1.5 py-px rounded shrink-0 transition-colors disabled:opacity-40 disabled:cursor-not-allowed hover:bg-zinc-700/40";
const ROW_BUTTON_STYLE = { color: "#a1a1aa", border: "1px solid rgba(113,113,122,0.3)" };
const PRESSED_BAND_CLASS = "text-[9px] font-medium px-1.5 py-px rounded shrink-0";
const IN_USE_CHIPS = {
  err: { text: "error layer", title: "Error layer of the shown image" },
  dq: { text: "mask", title: "Mask of the shown image" },
};

function isShownPlane(ext: HduInfo, activePath: string, activePlane: PlaneInfo | null | undefined): boolean {
  if (activePath === ext.ref) return true;
  if (!activePlane) return false;
  if (activePlane.band_plane) return false;
  if (ext.kind === "hdu") return activePlane.kind === "hdu" && activePlane.index === ext.index;
  return activePlane.kind === "array" && activePlane.key === ext.extname;
}

export default function HduSelectorPanel({
  filePath,
  activePath,
  activePlane,
  sourceOfProduct = false,
  onSelectHdu,
  onActivate,
  activating = false,
  activateError = null,
}: HduSelectorPanelProps) {
  const shownBadge = sourceOfProduct ? SOURCE_BADGE : SHOWN_BADGE;
  const [extensions, setExtensions] = useState<HduInfo[]>([]);
  const [loading, setLoading] = useState(Boolean(filePath));
  const [selectedIdx, setSelectedIdx] = useState<number | null>(null);
  const [hduHeader, setHduHeader] = useState<Record<string, string> | null>(null);
  const [headerLoading, setHeaderLoading] = useState(false);
  const [hduSearch, setHduSearch] = useState("");
  const [copiedKey, showCopiedKey] = useTimedNotice<string>(COPIED_FEEDBACK_MS);
  const [extensionsError, setExtensionsError] = useState<string | null>(null);
  const [headerError, setHeaderError] = useState<string | null>(null);
  const [loadAttempt, setLoadAttempt] = useState(0);
  const [pending, setPending] = useState<PendingLayer | null>(null);

  const headerRequestRef = useRef(0);

  useEffect(() => {
    setPending(null);
  }, [filePath, activePath]);

  useEffect(() => {
    if (!filePath) return;
    let cancelled = false;
    setLoading(true);
    setExtensions([]);
    setSelectedIdx(null);
    setHduHeader(null);
    setHduSearch("");
    setExtensionsError(null);
    setHeaderError(null);
    setHeaderLoading(false);
    headerRequestRef.current++;

    getFitsExtensions(filePath)
      .then((result) => {
        if (cancelled) return;
        setExtensions(result?.extensions ?? []);
      })
      .catch((err) => {
        if (cancelled) return;
        console.error("Failed to load FITS extensions:", err);
        setExtensionsError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (cancelled) return;
        setLoading(false);
      });

    return () => { cancelled = true; };
  }, [filePath, loadAttempt]);

  const handleSelectHdu = useCallback(
    async (idx: number) => {
      const req = ++headerRequestRef.current;
      setSelectedIdx(idx);
      setHeaderLoading(true);
      setHduHeader(null);
      setHduSearch("");
      setHeaderError(null);
      try {
        const header = await getHeaderByHdu(filePath, idx);
        if (headerRequestRef.current !== req) return;
        setHduHeader(header.index);
        onSelectHdu?.(idx, header.index);
      } catch (err) {
        if (headerRequestRef.current !== req) return;
        console.error("Failed to load HDU header:", err);
        setHeaderError(err instanceof Error ? err.message : String(err));
      } finally {
        if (headerRequestRef.current === req) setHeaderLoading(false);
      }
    },
    [filePath, onSelectHdu],
  );

  const handleCopyKey = useCallback((key: string, val: string) => {
    navigator.clipboard?.writeText(`${key} = ${val}`);
    showCopiedKey(key);
  }, [showCopiedKey]);

  const filteredHduEntries = useMemo(() => {
    if (!hduHeader || typeof hduHeader !== "object") return [];
    const all = Object.entries(hduHeader);
    const q = hduSearch.toLowerCase().trim();
    if (!q) return all;
    return all.filter(
      ([k, v]) => k.toLowerCase().includes(q) || String(v).toLowerCase().includes(q),
    );
  }, [hduHeader, hduSearch]);

  const isAsdf = extensions.length > 0 && extensions[0].kind === "array";
  const panelTitle = isAsdf ? "ASDF Arrays" : "FITS Extensions";
  const countLabel = isAsdf ? `${extensions.length} arrays` : `${extensions.length} HDUs`;
  const assignablePlane = activePlane && shownLayerText(activePlane) === null ? activePlane : null;
  const layersBlock = activePlane ? (
    <CompanionAssign imagePath={activePath} plane={activePlane} pending={pending} onPendingChange={setPending} />
  ) : null;

  if (loading) {
    return (
      <div className="ab-panel flex items-center justify-center py-5">
        <div
          className="w-5 h-5 rounded-full animate-spin"
          style={{ border: "2px solid transparent", borderTopColor: "var(--ab-rose)", borderRightColor: "rgba(244,63,94,0.3)" }}
        />
      </div>
    );
  }

  if (extensionsError) {
    return (
      <div className="ab-panel overflow-hidden">
        <div className="ab-panel-header">
          <div className="flex items-center gap-2">
            <Database size={12} style={{ color: "var(--ab-rose)" }} />
            <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">
              FITS Extensions
            </span>
          </div>
        </div>
        <div className="flex flex-col items-center gap-2 py-4 px-3">
          <p role="alert" className="text-[10px] text-red-400/80 text-center select-text">
            Failed to read extensions: {extensionsError}
          </p>
          <button
            onClick={() => setLoadAttempt((a) => a + 1)}
            className="text-[10px] px-2 py-1 rounded transition-colors"
            style={{ color: "var(--ab-rose)", border: "1px solid rgba(244,63,94,0.25)", background: "rgba(244,63,94,0.06)" }}
          >
            Retry
          </button>
        </div>
      </div>
    );
  }

  if (extensions.length <= 1) {
    if (!layersBlock) return null;
    return (
      <div className="ab-panel overflow-hidden">
        <div className="ab-panel-header">
          <div className="flex items-center gap-2">
            <Database size={12} style={{ color: "var(--ab-rose)" }} />
            <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">
              Layers
            </span>
          </div>
        </div>
        {layersBlock}
      </div>
    );
  }

  return (
    <div className="ab-panel overflow-hidden">
      <div className="ab-panel-header">
        <div className="flex items-center gap-2">
          <Database size={12} style={{ color: "var(--ab-rose)" }} />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">
            {panelTitle}
          </span>
        </div>
        <span className="text-[10px] font-mono text-zinc-500">
          {countLabel}
        </span>
      </div>

      {layersBlock}

      <div className="max-h-[240px] overflow-y-auto">
        {extensions.map((ext, i) => {
          const idx = ext.index ?? i;
          const isHdu = ext.kind === "hdu";
          const isSelected = isHdu && selectedIdx === idx;
          const isShown = isShownPlane(ext, activePath, activePlane);
          const dimParts = [ext.naxis1, ext.naxis2, ext.naxis3]
            .slice(0, Math.max(0, ext.naxis))
            .filter((d) => d > 0);
          const dims = dimParts.length > 0 ? dimParts.join(" × ") : "";
          const typeLabel = !isHdu ? "ARRAY" : idx === 0 ? "PRIMARY" : ext.has_data ? "IMAGE" : "EXT";
          const ts = TYPE_STYLES[typeLabel] || DEFAULT_TYPE_STYLE;
          const chip = roleChip(ext);
          const actions = rowActions(ext, assignablePlane, isShown);
          const layerName = hduLayerName(ext);
          const bands = ext.bands ?? [];
          const shownBand = activePlane?.band_plane?.hdu === ext.index ? activePlane.band_plane.plane : null;
          const nameLabel = ext.extname
            ? ext.extver !== null && ext.extver !== undefined && isHdu
              ? `${ext.extname},${ext.extver}`
              : ext.extname
            : "(unnamed)";

          return (
            <div
              key={ext.ref || i}
              className="px-3 py-1.5 transition-all"
              style={{
                background: isSelected ? "rgba(244,63,94,0.06)" : isShown ? "rgba(20,184,166,0.05)" : "transparent",
                borderBottom: "1px solid rgba(63,63,70,0.12)",
              }}
            >
              <div className="flex items-center gap-2">
                <button
                  onClick={() => { if (isHdu) handleSelectHdu(idx); }}
                  disabled={!isHdu}
                  className="flex items-center gap-2.5 flex-1 min-w-0 text-left disabled:cursor-default"
                  title={isHdu ? "Show this HDU's header" : ext.extname ?? undefined}
                >
                  <span
                    className="transition-transform shrink-0"
                    style={{ transform: isSelected ? "rotate(90deg)" : "rotate(0deg)", visibility: isHdu ? "visible" : "hidden" }}
                  >
                    <ChevronRight size={10} style={{ color: isSelected ? "var(--ab-rose)" : "var(--ab-text-4)" }} />
                  </span>
                  <span className="text-[10px] font-mono shrink-0 w-[24px]" style={{ color: "var(--ab-text-4)" }}>
                    #{idx}
                  </span>
                  <span
                    className="text-[9px] font-bold px-2 py-0.5 rounded shrink-0"
                    style={{ background: ts.bg, border: `1px solid ${ts.border}`, color: ts.text }}
                  >
                    {typeLabel}
                  </span>
                  <span
                    className="text-[11px] truncate flex-1"
                    style={{ color: isSelected || isShown ? "#e4e4e7" : "var(--ab-text-3)" }}
                    title={ext.extname ?? undefined}
                  >
                    {nameLabel}
                  </span>
                  {chip && (
                    <span
                      className="text-[9px] font-bold px-1.5 py-px rounded shrink-0"
                      style={{ background: TAG_STYLES[chip.tone].bg, border: `1px solid ${TAG_STYLES[chip.tone].border}`, color: TAG_STYLES[chip.tone].text }}
                      title={chip.title}
                    >
                      {chip.label}
                    </span>
                  )}
                  {dims && (
                    <span className="text-[10px] font-mono min-w-0 truncate" style={{ color: "var(--ab-text-4)" }} title={dims}>
                      {dims}
                    </span>
                  )}
                  {ext.bunit && (
                    <span className="text-[10px] font-mono min-w-0 truncate" style={{ color: "var(--ab-text-4)" }} title={`BUNIT ${ext.bunit}`}>
                      {ext.bunit}
                    </span>
                  )}
                </button>
                {actions.inUse && (
                  <span
                    className="text-[9px] font-medium px-1.5 py-px rounded shrink-0"
                    style={TEAL_BADGE_STYLE}
                    title={IN_USE_CHIPS[actions.inUse].title}
                  >
                    {IN_USE_CHIPS[actions.inUse].text}
                  </span>
                )}
                {actions.asErr && (
                  <button
                    type="button"
                    onClick={() => setPending({ role: "err", source: ext.ref, name: layerName })}
                    className={ROW_BUTTON_CLASS}
                    style={ROW_BUTTON_STYLE}
                    title={`Use ${layerName} as the error layer of the shown image`}
                  >
                    Use as ERR
                  </button>
                )}
                {actions.asDq && (
                  <button
                    type="button"
                    onClick={() => setPending({ role: "dq", source: ext.ref, name: layerName })}
                    className={ROW_BUTTON_CLASS}
                    style={ROW_BUTTON_STYLE}
                    title={`Use ${layerName} as the mask of the shown image`}
                  >
                    Use as DQ
                  </button>
                )}
                {ext.has_data && (
                  isShown ? (
                    <span
                      className="text-[9px] font-medium px-1.5 py-px rounded shrink-0 flex items-center gap-1"
                      style={TEAL_BADGE_STYLE}
                      title={shownBadge.title}
                    >
                      {!sourceOfProduct && <Eye size={9} />}
                      {shownBadge.text}
                    </span>
                  ) : (
                    <button
                      onClick={() => onActivate?.(ext.ref)}
                      disabled={activating || !onActivate}
                      className={ROW_BUTTON_CLASS}
                      style={ROW_BUTTON_STYLE}
                      title={`Display ${ext.ref}`}
                    >
                      {activating ? <Loader2 size={9} className="animate-spin" /> : "Display"}
                    </button>
                  )
                )}
              </div>
              {bands.length > 0 && (
                <div role="group" aria-label={`Bands of HDU ${idx}`} className="flex flex-wrap items-center gap-1 mt-1 pl-[34px]">
                  {bands.map((band) => {
                    const label = bandButtonLabel(band);
                    const pressed = shownBand === band.plane;
                    return (
                      <button
                        key={band.key}
                        type="button"
                        aria-pressed={pressed}
                        disabled={pressed || activating || !onActivate}
                        onClick={() => onActivate?.(band.key)}
                        className={pressed ? PRESSED_BAND_CLASS : ROW_BUTTON_CLASS}
                        style={pressed ? TEAL_BADGE_STYLE : ROW_BUTTON_STYLE}
                        title={`Display band ${label} (plane ${band.plane} of HDU ${idx})`}
                      >
                        {label}
                      </button>
                    );
                  })}
                </div>
              )}
            </div>
          );
        })}
      </div>

      {activateError && (
        <div className="flex items-center gap-2 px-3 py-2" style={{ borderTop: "1px solid var(--ab-border)" }}>
          <p role="alert" className="text-[10px] text-red-400/80 flex-1 truncate select-text" title={activateError}>
            Failed to display plane: {activateError}
          </p>
        </div>
      )}

      {headerLoading && (
        <div className="flex items-center justify-center py-4" style={{ borderTop: "1px solid var(--ab-border)" }}>
          <div
            className="w-4 h-4 rounded-full animate-spin"
            style={{ border: "2px solid transparent", borderTopColor: "var(--ab-rose)", borderRightColor: "rgba(244,63,94,0.3)" }}
          />
        </div>
      )}

      {headerError && !headerLoading && (
        <div className="flex items-center gap-2 px-3 py-2" style={{ borderTop: "1px solid var(--ab-border)" }}>
          <p role="alert" className="text-[10px] text-red-400/80 flex-1 truncate select-text" title={headerError}>
            Failed to read HDU header: {headerError}
          </p>
          <button
            onClick={() => { if (selectedIdx !== null) handleSelectHdu(selectedIdx); }}
            className="text-[10px] shrink-0 px-2 py-0.5 rounded transition-colors"
            style={{ color: "var(--ab-rose)", border: "1px solid rgba(244,63,94,0.25)", background: "rgba(244,63,94,0.06)" }}
          >
            Retry
          </button>
        </div>
      )}

      {hduHeader && !headerLoading && (
        <div style={{ borderTop: "1px solid var(--ab-border)" }}>
          <div className="flex items-center gap-2 px-3 py-1.5" style={{ background: "rgba(244,63,94,0.03)" }}>
            <FileText size={10} style={{ color: "var(--ab-rose)", opacity: 0.6 }} />
            <span className="text-[10px] font-medium text-zinc-400">
              HDU #{selectedIdx}
            </span>
            <span className="text-[10px] font-mono text-zinc-600 ml-auto">
              {filteredHduEntries.length} keys
            </span>
          </div>

          {Object.keys(hduHeader).length > 8 && (
            <div className="px-3 py-1.5" style={{ borderBottom: "1px solid rgba(63,63,70,0.12)" }}>
              <div className="relative">
                <Search size={10} className="absolute left-2 top-1/2 -translate-y-1/2 text-zinc-600 pointer-events-none" />
                <input
                  type="text"
                  value={hduSearch}
                  onChange={(e) => setHduSearch(e.target.value)}
                  placeholder="Filter HDU keys..."
                  aria-label="Filter HDU keys"
                  className="w-full pl-6 pr-6 py-1 text-[10px] text-zinc-300 rounded placeholder:text-zinc-600 transition-colors"
                  style={{
                    background: "rgba(24,24,32,0.6)",
                    border: hduSearch ? "1px solid rgba(244,63,94,0.2)" : "1px solid rgba(63,63,70,0.3)",
                  }}
                />
                {hduSearch && (
                  <button
                    type="button"
                    aria-label="Clear filter"
                    onClick={() => setHduSearch("")}
                    className="absolute right-2 top-1/2 -translate-y-1/2 text-zinc-500 hover:text-zinc-300"
                  >
                    <X size={10} />
                  </button>
                )}
              </div>
            </div>
          )}

          <div className="max-h-[220px] overflow-y-auto px-1 py-1">
            {filteredHduEntries.map(([key, val]) => (
              <div
                key={key}
                className="group flex items-center gap-2 px-2 py-[3px] rounded transition-colors"
                onMouseOver={(e) => (e.currentTarget.style.background = "rgba(63,63,70,0.2)")}
                onMouseOut={(e) => (e.currentTarget.style.background = "")}
              >
                <span className="text-[10px] font-mono w-[80px] shrink-0 truncate text-zinc-500" title={key}>
                  {key}
                </span>
                <span className="text-[10px] font-mono text-zinc-300 truncate flex-1 select-text" title={String(val)}>
                  {String(val)}
                </span>
                <button
                  type="button"
                  aria-label="Copy key"
                  onClick={() => handleCopyKey(key, String(val))}
                  className="opacity-0 group-hover:opacity-100 focus-visible:opacity-100 p-0.5 rounded transition-all hover:bg-zinc-700/40"
                >
                  {copiedKey === key
                    ? <Check size={9} style={{ color: "var(--ab-emerald)" }} />
                    : <Copy size={9} className="text-zinc-600" />}
                </button>
              </div>
            ))}
            {filteredHduEntries.length === 0 && hduSearch && (
              <div className="text-[10px] text-zinc-600 text-center py-3">
                No keys match "{hduSearch}"
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
