import { useState, useRef, useEffect, memo } from "react";
import { AlertTriangle, ChevronDown, Loader2, RefreshCw } from "lucide-react";
import { useDqContext, useFileContext, type DqTarget } from "../../context/PreviewContext";
import type { DqFlagTableResult } from "../../services/display";
import type { DqOverlaySettings } from "../../shared/types/dq";
import type { ProcessedFile } from "../../shared/types";
import { setFlagExclusion } from "../../services/companions";
import { applyLayerChange } from "../../hooks/applyLayerChange";
import { useMeasurementProvenance } from "../../hooks/useMeasurementLog";
import { measurementLog } from "../../utils/measurementLog";
import { layerAssignmentEntry } from "../../utils/measurementLogLayers";
import {
  emptyFlagsText,
  EXCLUDED_FLAG_TAG,
  EXCLUDED_FLAG_TITLE,
  exclusionLabel,
  exclusionSummary,
  exclusionTitle,
  FLAG_LIST_LEGEND,
  hasBit,
  isNamedFlagSet,
  listedDqFlags,
  toggleBit,
  UNNAMED_FLAGS_TITLE,
} from "../../utils/dqFlags";

const BTN_CLASS = "flex items-center gap-1 text-[10px] px-2 py-0.5 rounded transition-all duration-200 disabled:opacity-30 disabled:cursor-not-allowed";
const ON_STYLE: React.CSSProperties = { background: "rgba(248,113,113,0.15)", color: "#f87171", border: "1px solid rgba(248,113,113,0.35)" };
const OFF_STYLE: React.CSSProperties = { color: "var(--ab-text-3)", border: "1px solid transparent" };
const EDIT_BTN_CLASS = "px-1.5 py-0.5 rounded text-[9px] text-zinc-300 hover:text-zinc-100 transition-colors disabled:opacity-30 disabled:cursor-not-allowed";
const EDIT_BTN_STYLE: React.CSSProperties = { border: "1px solid var(--ab-border)" };
const APPLY_EXCLUSION_TITLE = "Tick flags in the list (they show on the image), then make them the excluded set";

interface ExclusionEditor {
  busy: boolean;
  error: string | null;
  onApply: (mask: number) => void;
  onDefault: () => void;
}

interface DqFlagMenuProps {
  flagTable: DqFlagTableResult | null;
  overlay: DqOverlaySettings;
  setOverlay: (patch: Partial<DqOverlaySettings>) => void;
  excludeDq: boolean;
  setExcludeDq: (v: boolean) => void;
  dqMaskError: string | null;
  unavailable?: string | null;
  exclusion?: ExclusionEditor | null;
}

export function DqFlagMenu({ flagTable, overlay, setOverlay, excludeDq, setExcludeDq, dqMaskError, unavailable = null, exclusion = null }: DqFlagMenuProps) {
  const flags = flagTable ? listedDqFlags(flagTable) : [];
  const emptyText = flagTable ? emptyFlagsText(flagTable) : null;
  const overlayMask = overlay.mask >>> 0;
  return (
    <div
      className="absolute right-0 top-full mt-1 z-50 w-60 rounded-md shadow-lg flex flex-col"
      style={{ background: "rgba(9,9,20,0.97)", border: "1px solid var(--ab-border)" }}
    >
      <div className="flex items-center justify-between px-2.5 py-1.5" style={{ borderBottom: "1px solid var(--ab-border)" }}>
        <span className="text-[10px] font-semibold text-zinc-300 uppercase tracking-wider">DQ flags</span>
        {flagTable && (
          <span
            className="text-[9px] font-mono text-zinc-500 truncate max-w-[120px]"
            title={isNamedFlagSet(flagTable) ? flagTable.label : UNNAMED_FLAGS_TITLE}
          >
            {flagTable.label}
          </span>
        )}
      </div>

      {unavailable !== null ? (
        <div className="flex flex-col gap-0.5 px-2.5 py-1.5 text-[9px] break-words" style={{ color: "var(--ab-text-3)" }}>
          <span>{`Mask layer not used: ${unavailable}`}</span>
          <span>Nothing is excluded from histogram, statistics and photometry.</span>
        </div>
      ) : (
        <>
          {typeof flagTable?.dq_origin === "string" && (
            <div
              className="px-2.5 py-1 text-[9px] truncate"
              style={{ color: "var(--ab-text-3)", borderBottom: "1px solid var(--ab-border)" }}
              title={flagTable.dq_origin}
            >
              {`Mask: ${flagTable.dq_origin}`}
            </div>
          )}

          {dqMaskError && (
            <div className="px-2.5 py-1.5 text-[9px] text-red-400 break-words" style={{ borderBottom: "1px solid var(--ab-border)" }}>
              overlay unavailable: {dqMaskError}
            </div>
          )}

          {!flagTable ? (
            <div className="flex items-center justify-center py-3">
              <Loader2 size={12} className="animate-spin text-zinc-600" />
            </div>
          ) : (
            <>
              <div className="flex items-center gap-2 px-2.5 py-1 text-[9px]">
                <button
                  onClick={() => setOverlay({ mask: flagTable.default_mask })}
                  className="text-zinc-400 hover:text-zinc-200 transition-colors"
                >
                  default
                </button>
                <button
                  onClick={() => setOverlay({ mask: 0 })}
                  className="text-zinc-400 hover:text-zinc-200 transition-colors"
                >
                  none
                </button>
                <span className="ml-auto font-mono text-zinc-600">mask {overlayMask}</span>
              </div>
              <div className="px-2.5 pb-1 text-[9px] break-words" style={{ color: "var(--ab-text-3)" }}>{FLAG_LIST_LEGEND}</div>
              <div className="max-h-[220px] overflow-y-auto px-1.5 pb-1">
                {flags.map((f) => (
                  <label
                    key={f.bit}
                    className="flex items-center gap-2 px-1 py-[2px] rounded cursor-pointer hover:bg-zinc-800/40"
                  >
                    <input
                      type="checkbox"
                      className="accent-red-400"
                      checked={hasBit(overlay.mask, f.bit)}
                      onChange={() => setOverlay({ mask: toggleBit(overlay.mask, f.bit) })}
                    />
                    <span className="text-[10px] font-mono text-zinc-300 truncate">{f.name}</span>
                    <span className="ml-auto text-[9px] font-mono text-zinc-600">{f.bit}</span>
                    {hasBit(flagTable.exclusion_mask, f.bit) && (
                      <span className="text-[9px] font-mono text-red-400" title={EXCLUDED_FLAG_TITLE}>{EXCLUDED_FLAG_TAG}</span>
                    )}
                  </label>
                ))}
                {emptyText !== null && <div className="px-1 py-1 text-[9px] text-zinc-500">{emptyText}</div>}
              </div>
            </>
          )}

          <label
            className="flex items-center gap-2 px-2.5 py-1.5 cursor-pointer"
            style={{ borderTop: "1px solid var(--ab-border)" }}
            title={flagTable ? exclusionTitle(flagTable) : undefined}
          >
            <input
              type="checkbox"
              className="accent-red-400"
              checked={excludeDq}
              onChange={(e) => setExcludeDq(e.target.checked)}
            />
            <span className="text-[10px] text-zinc-300">{flagTable ? exclusionLabel(flagTable) : "exclude flags from stats"}</span>
          </label>

          {exclusion && flagTable && (
            <div className="flex flex-col gap-1 px-2.5 pb-1.5 text-[9px]">
              <span className="break-words" style={{ color: "var(--ab-text-3)" }}>{exclusionSummary(flagTable)}</span>
              <div className="flex items-center gap-1.5">
                <button
                  type="button"
                  title={APPLY_EXCLUSION_TITLE}
                  disabled={exclusion.busy || overlayMask === flagTable.exclusion_mask >>> 0}
                  onClick={() => exclusion.onApply(overlayMask)}
                  className={EDIT_BTN_CLASS}
                  style={EDIT_BTN_STYLE}
                >
                  Exclude the checked flags
                </button>
                {flagTable.exclusion_edited === true && (
                  <button
                    type="button"
                    disabled={exclusion.busy}
                    onClick={() => exclusion.onDefault()}
                    className={EDIT_BTN_CLASS}
                    style={EDIT_BTN_STYLE}
                  >
                    Default exclusion
                  </button>
                )}
              </div>
              {exclusion.error !== null && (
                <div role="alert" className="text-red-400 break-words">{`Exclusion not changed: ${exclusion.error}`}</div>
              )}
            </div>
          )}
        </>
      )}
    </div>
  );
}

function noDqTitle(target: DqTarget, flagTableLoading: boolean): string {
  if (target.kind === "composite") return "No DQ overlay for the colour composite";
  if (target.kind === "file") return "No DQ plane in this file";
  return flagTableLoading ? `Reading DQ of ${target.label}…` : `No DQ plane in this product (${target.label})`;
}

const exclusionKeyOf = (file: Pick<ProcessedFile, "id" | "path"> | null) => (file ? `${file.id}|${file.path}` : null);

function DqControlsInner() {
  const {
    dqRef, dqTarget, flagTable, flagTableLoading, flagTableError, retryFlagTable,
    overlay, setOverlay, excludeDq, setExcludeDq, dqMaskLoading, dqMaskError,
  } = useDqContext();
  const { file } = useFileContext();
  const provenance = useMeasurementProvenance();
  const [open, setOpen] = useState(false);
  const [exclusionBusy, setExclusionBusy] = useState(false);
  const [exclusionError, setExclusionError] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const fileKey = exclusionKeyOf(file);
  const activeFileKey = useRef(fileKey);
  const hasDq = !!dqRef;
  const tableFailed = flagTableError !== null;
  const plane = dqTarget.kind === "file" ? file?.result?.plane ?? null : null;
  const unusable = !hasDq && !tableFailed ? plane?.dq_unavailable ?? null : null;
  const menuAvailable = (hasDq || unusable !== null) && !tableFailed;

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [open]);

  useEffect(() => {
    if (!menuAvailable) setOpen(false);
  }, [menuAvailable]);

  useEffect(() => {
    activeFileKey.current = fileKey;
    setExclusionBusy(false);
    setExclusionError(null);
  }, [fileKey]);

  const changeExclusion = (target: ProcessedFile, mask: number | null) => {
    const requestedFor = exclusionKeyOf(target);
    setExclusionBusy(true);
    setFlagExclusion(target.path, mask).then(
      (changed) => {
        if (activeFileKey.current !== requestedFor) return;
        applyLayerChange(target, changed);
        measurementLog.append(
          layerAssignmentEntry(provenance, mask === null ? { role: "exclusion", action: "default", mask: null } : { role: "exclusion", action: "choose", mask }, changed),
        );
        setExclusionError(null);
        setExclusionBusy(false);
      },
      (e: unknown) => {
        if (activeFileKey.current !== requestedFor) return;
        setExclusionError(e instanceof Error ? e.message : String(e));
        setExclusionBusy(false);
      },
    );
  };

  const editable = dqTarget.kind === "file" && hasDq && file?.result ? file : null;
  const exclusion: ExclusionEditor | null = editable
    ? {
        busy: exclusionBusy,
        error: exclusionError,
        onApply: (mask) => changeExclusion(editable, mask),
        onDefault: () => changeExclusion(editable, null),
      }
    : null;

  const unavailableTitle = tableFailed
    ? `DQ table unavailable: ${flagTableError}`
    : unusable !== null
      ? `Mask layer not used: ${unusable}`
      : noDqTitle(dqTarget, flagTableLoading);
  const toggleTitle = tableFailed || !hasDq
    ? unavailableTitle
    : dqMaskError
      ? `DQ overlay failed: ${dqMaskError}`
      : overlay.enabled
        ? "DQ overlay shown — click to hide"
        : "Show flagged DQ pixels over the image";
  const pending = dqMaskLoading || (!hasDq && flagTableLoading);

  return (
    <div ref={rootRef} className="relative flex items-center gap-0.5">
      <button
        onClick={() => { if (hasDq) setOverlay({ enabled: !overlay.enabled }); }}
        disabled={!hasDq}
        title={toggleTitle}
        aria-pressed={overlay.enabled && hasDq}
        className={BTN_CLASS}
        style={overlay.enabled && hasDq ? ON_STYLE : OFF_STYLE}
      >
        {pending ? <Loader2 size={10} className="animate-spin" /> : <AlertTriangle size={10} />}
        DQ
      </button>
      {tableFailed ? (
        <button
          type="button"
          onClick={retryFlagTable}
          title={unavailableTitle}
          aria-label="Retry reading the DQ table"
          className={BTN_CLASS}
          style={OFF_STYLE}
        >
          <RefreshCw size={10} />
          Retry
        </button>
      ) : (
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          disabled={!hasDq && unusable === null}
          title={hasDq ? "DQ flags and masking options" : unavailableTitle}
          aria-label="DQ flag options"
          aria-expanded={open && menuAvailable}
          className="p-1 rounded text-zinc-500 hover:text-zinc-300 transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
        >
          <ChevronDown size={10} />
        </button>
      )}

      {open && menuAvailable && (
        <DqFlagMenu
          flagTable={flagTable}
          overlay={overlay}
          setOverlay={setOverlay}
          excludeDq={excludeDq}
          setExcludeDq={setExcludeDq}
          dqMaskError={dqMaskError}
          unavailable={unusable}
          exclusion={exclusion}
        />
      )}
    </div>
  );
}

export default memo(DqControlsInner);
