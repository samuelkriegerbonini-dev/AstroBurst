import { memo, useCallback, useEffect, useMemo, useState, useSyncExternalStore, type ReactNode } from "react";
import { Download, Eye, EyeOff, FolderOpen, RefreshCw, RotateCcw, Tag, Trash2, VenetianMask } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { DecimalField, ErrorAlert, RunButton, SectionHeader, Slider, Toggle } from "../ui";
import { CONFIRM_DELETE_LABEL, useArmedConfirm } from "../../hooks/useArmedConfirm";
import { HISTORY_BUSY_MESSAGE, loadHistory, useHistoryRender, useHistorySnapshot, useHistoryTarget, type HistoryTarget } from "../../hooks/useHistory";
import { useCompositeMode } from "../../hooks/useCompositeMode";
import { runExclusive, useRunLocked } from "../../hooks/useProcessingRun";
import { useRegionDoc } from "../../hooks/useRegionStore";
import { useDisplayedImage, useFileContext } from "../../context/PreviewContext";
import { useCompositePreview } from "../../context/CompositeContext";
import { useWizardCompositeDims } from "../../context/ComposeWizardContext";
import { getOutputDir } from "../../infrastructure/tauri";
import { COMPOSITE_MASK_TARGET, createMask, deleteMask, exportMask, type MaskTarget } from "../../services/masks";
import { MARK_MODES, type MarkMode, type MaskCreateResult, type MaskExportKind, type MaskKind, type MaskSpec, type MaskTint } from "../../shared/types/masks";
import type { HistoryDoc } from "../../shared/types/ops";
import { COMPOSITE_RUN_KEY } from "../../utils/compositeChain";
import type { HistorySnapshot } from "../../utils/historyStore";
import { maskStore, type MaskEntry } from "../../utils/maskStore";
import { FITS_FILE_EXTENSIONS } from "../../utils/validation";
import { DEFAULT_MASK_FORM, maskRequestOf, regionSplit, type MaskForm } from "./maskForm";
import { forgetMaskPreviews, hideMaskLayer, maskInputPath, showMaskLayer } from "./maskOverlay";
import {
  DISPLAY_HINT,
  FEATHER_HINT,
  HISTORY_LOADING_NOTE,
  LUMINANCE_HINT,
  MARK_MODE_HINTS,
  MARK_MODE_LABELS,
  MASK_FILE_HINT,
  MASK_KIND_LABELS,
  MASKS_COMPOSITE_NOTE,
  NO_MASKS_NOTE,
  RANGE_HINT,
  REGIONS_ON_COMPOSITE_NOTE,
  STARS_HINT,
  deletedNotice,
  entryCoverageLine,
  exportedNotice,
  fileNameOf,
  maskResultLine,
  recipeSummary,
  regionsSummary,
  restoredNotice,
} from "./maskText";

const MASKS_RUN_PANEL = "masks";
const KIND_ORDER: readonly MaskKind[] = ["luminance", "range", "stars", "regions", "file"];
const NO_ENTRIES: readonly MaskEntry[] = Object.freeze([]);
const ICON_BUTTON = "p-1 rounded text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800/60 disabled:opacity-30 disabled:cursor-not-allowed";
const TEXT_BUTTON = "flex items-center gap-1 text-[10px] px-2 py-0.5 rounded text-zinc-400 hover:text-zinc-200 disabled:opacity-30 disabled:cursor-not-allowed";
const FIELD = "w-full bg-zinc-900 border border-zinc-700/60 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:outline-none focus:border-amber-500/60";
const FIELD_LABEL = "flex flex-col gap-0.5 text-[10px] text-zinc-500";
const NOTE = "text-[10px] text-amber-300/90 bg-amber-900/15 border border-amber-700/25 rounded px-2 py-1.5";
const HINT = "text-[10px] text-zinc-500 px-1";

export interface MasksToolProps {
  fileName: string;
  compositeMode: boolean;
  target: HistoryTarget | null;
  snapshot: HistorySnapshot;
  imageDims: readonly [number, number] | null;
  compositeRevision: string;
  runLocked: boolean;
  onHistoryChanged: () => Promise<unknown> | void;
}

interface Keyed<T> {
  key: string;
  value: T;
}

interface RestoreOffer {
  maskId: string;
  recipe: MaskSpec;
}

interface MaskNotice {
  text: string;
  title?: string;
  restore?: RestoreOffer;
}

type MaskTask = (key: string, target: MaskTarget) => Promise<void>;

function messageOf(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function tintHex(tint: MaskTint): string {
  return `#${tint.map((c) => c.toString(16).padStart(2, "0")).join("")}`;
}

function tintOf(hex: string): MaskTint | null {
  const match = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex);
  return match ? [parseInt(match[1], 16), parseInt(match[2], 16), parseInt(match[3], 16)] : null;
}

function blockReasonOf(compositeMode: boolean, own: HistorySnapshot | null, runLocked: boolean): string | null {
  if (!compositeMode) {
    if (!own || own.pending) return HISTORY_LOADING_NOTE;
    if (own.state?.read_only) return own.state.read_only;
  }
  return runLocked ? HISTORY_BUSY_MESSAGE : null;
}

async function exclusively(lockKey: string, task: () => Promise<void>): Promise<void> {
  const outcome = await runExclusive(MASKS_RUN_PANEL, lockKey, task);
  if (!outcome.ran) throw new Error(HISTORY_BUSY_MESSAGE);
}

function keyed<T>(value: Keyed<T> | null, key: string | null): T | null {
  return value !== null && value.key === key ? value.value : null;
}

export function MasksTool({ fileName, compositeMode, target, snapshot, imageDims, compositeRevision, runLocked, onHistoryChanged }: MasksToolProps) {
  const [form, setForm] = useState<MaskForm>(DEFAULT_MASK_FORM);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<Keyed<string> | null>(null);
  const [notice, setNotice] = useState<Keyed<MaskNotice> | null>(null);
  const [lastResult, setLastResult] = useState<Keyed<MaskCreateResult> | null>(null);
  const masks = useSyncExternalStore(maskStore.subscribe, maskStore.get, maskStore.get);
  const { armedKey, press, blur } = useArmedConfirm();

  const own = !compositeMode && target !== null && snapshot.fileKey === target.fileKey ? snapshot : null;
  const doc: HistoryDoc | null = own?.state?.read_only === null ? own.state.doc : null;
  const inputPath = own ? maskInputPath(own) : null;
  const imageKey = compositeMode ? COMPOSITE_RUN_KEY : (target?.scope.imageKey ?? null);
  const basePath = compositeMode ? null : (target?.scope.basePath ?? null);
  const lockKey = compositeMode ? COMPOSITE_RUN_KEY : (target?.fileKey ?? null);
  const revision = compositeMode ? compositeRevision : (inputPath ?? "");
  const drawReady = compositeMode || (own !== null && (own.state !== null || !own.pending));
  const maskTarget = useMemo<MaskTarget | null>(() => {
    if (compositeMode) return COMPOSITE_MASK_TARGET;
    return imageKey === null ? null : { imageKey, basePath, inputPath };
  }, [compositeMode, imageKey, basePath, inputPath]);

  const regionDoc = useRegionDoc(compositeMode ? null : imageKey);
  const request = useMemo(
    () => maskRequestOf(form, { mark: masks.mark, regions: regionDoc.regions, compositeMode }),
    [form, masks.mark, regionDoc.regions, compositeMode],
  );
  const split = useMemo(() => regionSplit(regionDoc.regions), [regionDoc.regions]);

  const entries = imageKey === null ? NO_ENTRIES : (masks.lists[imageKey] ?? NO_ENTRIES);
  const activeId = imageKey === null ? null : (masks.active[imageKey] ?? null);
  const activeEntry = entries.find((entry) => entry.id === activeId) ?? null;
  const layerCols = activeEntry?.dims?.[0] ?? imageDims?.[0] ?? 0;
  const layerRows = activeEntry?.dims?.[1] ?? imageDims?.[1] ?? 0;
  const style = masks.style;

  useEffect(() => {
    if (imageKey === null || maskTarget === null || !drawReady) return;
    if (!activeEntry || layerCols === 0 || layerRows === 0) {
      hideMaskLayer(imageKey);
      return;
    }
    const key = imageKey;
    showMaskLayer({ key, target: maskTarget, entry: activeEntry, style, dims: [layerCols, layerRows], revision }).catch((e: unknown) =>
      setError({ key, value: messageOf(e) }),
    );
  }, [imageKey, maskTarget, drawReady, activeEntry, style, layerCols, layerRows, revision]);

  const blockReason = blockReasonOf(compositeMode, own, runLocked && busy === null);
  const locked = busy !== null || blockReason !== null;
  const createReason = blockReason ?? (request.ok ? null : request.reason);

  const runAction = useCallback(
    async (action: string, exclusive: boolean, task: MaskTask) => {
      if (imageKey === null || maskTarget === null || lockKey === null) return;
      const key = imageKey;
      setBusy(action);
      setError(null);
      try {
        if (exclusive) {
          await exclusively(lockKey, () => task(key, maskTarget));
          if (!compositeMode) await onHistoryChanged();
        } else {
          await task(key, maskTarget);
        }
      } catch (e) {
        setError({ key, value: messageOf(e) });
      } finally {
        setBusy(null);
      }
    },
    [imageKey, maskTarget, lockKey, compositeMode, onHistoryChanged],
  );

  const create = useCallback(() => {
    if (!request.ok) return;
    const { recipe, mods, shapesSystem } = request;
    void runAction("create", true, async (key, maskTarget) => {
      const result = await createMask(maskTarget, recipe, shapesSystem ? { shapesSystem, mods } : { mods });
      maskStore.recordCreated(key, result);
      maskStore.setActive(key, result.mask_id);
      setLastResult({ key, value: result });
      setNotice(null);
    });
  }, [request, runAction]);

  const measure = useCallback(
    (entry: MaskEntry) =>
      void runAction(`measure:${entry.id}`, true, async (key, maskTarget) => {
        const result = await createMask(maskTarget, entry.recipe);
        maskStore.recordCreated(key, result);
        setLastResult({ key, value: result });
      }),
    [runAction],
  );

  const remove = useCallback(
    (entry: MaskEntry) =>
      void runAction(`delete:${entry.id}`, true, async (key, maskTarget) => {
        const result = await deleteMask(maskTarget, entry.id);
        maskStore.recordDeleted(key, result);
        forgetMaskPreviews(key, entry.id);
        const restore = result.kept_for_replay ? undefined : { maskId: result.mask_id, recipe: result.recipe };
        setNotice({ key, value: { text: deletedNotice(result, doc), restore } });
      }),
    [runAction, doc],
  );

  const restore = useCallback(
    (offer: RestoreOffer) =>
      void runAction(`restore:${offer.maskId}`, true, async (key, maskTarget) => {
        const result = await createMask(maskTarget, offer.recipe);
        maskStore.recordCreated(key, result);
        maskStore.setActive(key, result.mask_id);
        setLastResult({ key, value: result });
        setNotice({ key, value: { text: restoredNotice(offer.maskId, result.mask_id) } });
      }),
    [runAction],
  );

  const exportAs = useCallback(
    (entry: MaskEntry, kind: MaskExportKind) =>
      void runAction(`export:${entry.id}`, false, async (key, maskTarget) => {
        const result = await exportMask(maskTarget, entry.id, await getOutputDir(), kind);
        maskStore.recordExported(result);
        setNotice({ key, value: { text: exportedNotice(result), title: result.fits_path } });
      }),
    [runAction],
  );

  const pickFile = useCallback(async () => {
    try {
      const picked = await open({ multiple: false, directory: false, title: "Mask FITS file", filters: [{ name: "FITS", extensions: [...FITS_FILE_EXTENSIONS] }] });
      if (typeof picked === "string") setForm((current) => ({ ...current, filePath: picked }));
    } catch (e) {
      if (imageKey !== null) setError({ key: imageKey, value: messageOf(e) });
    }
  }, [imageKey]);

  const patch = useCallback((next: Partial<MaskForm>) => setForm((current) => ({ ...current, ...next })), []);

  if (imageKey === null) {
    return <div className="p-4 text-xs text-zinc-500 italic">Select an image to make masks for it.</div>;
  }

  const shownError = keyed(error, imageKey);
  const shownNotice = keyed(notice, imageKey);
  const shownResult = keyed(lastResult, imageKey);
  const subject = compositeMode ? "Colour composite" : fileName;

  const kindFields = (): ReactNode => {
    switch (form.kind) {
      case "luminance":
        return (
          <>
            <Toggle label="Auto STF" checked={form.autoStf} disabled={locked} accent="amber" onChange={(autoStf) => patch({ autoStf })} />
            <div className={HINT}>{LUMINANCE_HINT}</div>
          </>
        );
      case "range":
        return (
          <>
            <div className="grid grid-cols-3 gap-2">
              <label className={FIELD_LABEL}>
                Low
                <DecimalField aria-label="Low" value={form.lo} onChange={(lo) => patch({ lo })} disabled={locked} className={FIELD} />
              </label>
              <label className={FIELD_LABEL}>
                High
                <DecimalField aria-label="High" value={form.hi} onChange={(hi) => patch({ hi })} disabled={locked} className={FIELD} />
              </label>
              <label className={FIELD_LABEL}>
                Fuzz
                <DecimalField aria-label="Fuzz" value={form.fuzz} onChange={(fuzz) => patch({ fuzz })} min={0} optional disabled={locked} placeholder="0" className={FIELD} />
              </label>
            </div>
            <div className={HINT}>{RANGE_HINT}</div>
          </>
        );
      case "stars":
        return (
          <>
            <Slider label="Detection Sigma" value={form.sigma} min={2} max={15} step={0.5} disabled={locked} accent="amber" format={(v) => `${v.toFixed(1)}σ`} hint="higher = only strong stars" onChange={(sigma) => patch({ sigma })} />
            <Slider label="Mask Growth" value={form.growth} min={0.5} max={10} step={0.1} disabled={locked} accent="amber" format={(v) => `${v.toFixed(1)}x FWHM`} onChange={(growth) => patch({ growth })} />
            <Slider label="Mask Softness" value={form.softness} min={0} max={20} step={0.5} disabled={locked} accent="amber" format={(v) => `${v.toFixed(1)}px`} onChange={(softness) => patch({ softness })} />
            <Toggle label="Luminance Protection" checked={form.luminanceProtect} disabled={locked} accent="amber" onChange={(luminanceProtect) => patch({ luminanceProtect })} />
            {form.luminanceProtect && (
              <Slider label="Luminance Ceiling" value={form.luminanceCeiling} min={0.5} max={0.99} step={0.01} disabled={locked} accent="amber" format={(v) => v.toFixed(2)} hint="pixels brighter than this are masked too" onChange={(luminanceCeiling) => patch({ luminanceCeiling })} />
            )}
            <details className="text-[10px] text-zinc-500">
              <summary className="cursor-pointer select-none text-zinc-400">Star filters</summary>
              <div className="mt-1 flex flex-col gap-2">
                <Slider label="Min FWHM" value={form.minFwhm} min={0.5} max={10} step={0.5} disabled={locked} accent="amber" format={(v) => `${v.toFixed(1)}px`} onChange={(minFwhm) => patch({ minFwhm })} />
                <Slider label="Max FWHM" value={form.maxFwhm} min={5} max={60} step={1} disabled={locked} accent="amber" format={(v) => `${v.toFixed(0)}px`} onChange={(maxFwhm) => patch({ maxFwhm })} />
                <Slider label="Max Eccentricity" value={form.maxEccentricity} min={0.5} max={1} step={0.01} disabled={locked} accent="amber" format={(v) => v.toFixed(2)} hint="lower = reject elongated blobs" onChange={(maxEccentricity) => patch({ maxEccentricity })} />
              </div>
            </details>
            <div className={HINT}>{STARS_HINT}</div>
          </>
        );
      case "regions":
        return (
          <div className={HINT}>{regionsSummary(split.shapes.length, split.excludeShapes.length, split.skipped)}</div>
        );
      case "file":
        return (
          <>
            <div className="flex items-center gap-2">
              <button type="button" data-mask-pick-file="" className={TEXT_BUTTON} style={{ border: "1px solid var(--ab-border)" }} onClick={() => void pickFile()} disabled={locked} title={form.filePath ?? undefined}>
                <FolderOpen size={11} />
                {form.filePath ? fileNameOf(form.filePath) : "Pick FITS"}
              </button>
              <select
                aria-label="Mark mode"
                value={masks.mark}
                disabled={locked}
                onChange={(e) => maskStore.setMark(e.target.value as MarkMode)}
                className="bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300"
              >
                {MARK_MODES.map((mode) => (
                  <option key={mode} value={mode}>
                    {MARK_MODE_LABELS[mode]}
                  </option>
                ))}
              </select>
            </div>
            {masks.mark === "range" && (
              <div className="grid grid-cols-2 gap-2">
                <label className={FIELD_LABEL}>
                  Mark Low
                  <DecimalField aria-label="Mark Low" value={form.markLo} onChange={(markLo) => patch({ markLo })} disabled={locked} className={FIELD} />
                </label>
                <label className={FIELD_LABEL}>
                  Mark High
                  <DecimalField aria-label="Mark High" value={form.markHi} onChange={(markHi) => patch({ markHi })} disabled={locked} className={FIELD} />
                </label>
              </div>
            )}
            <div className={HINT}>
              {MARK_MODE_HINTS[masks.mark]} {MASK_FILE_HINT}
            </div>
          </>
        );
    }
  };

  const maskRow = (entry: MaskEntry): ReactNode => {
    const active = entry.id === activeId;
    const armed = armedKey === entry.id;
    return (
      <li
        key={entry.id}
        data-mask-row={entry.id}
        data-mask-active={active}
        data-mask-coverage={entry.coverage ?? undefined}
        data-mask-stars={entry.stars ?? undefined}
        className={`flex items-start gap-2 rounded px-2 py-1.5 ${active ? "bg-amber-500/10 ring-1 ring-amber-500/30" : "bg-zinc-800/40"}`}
      >
        <button
          type="button"
          className={ICON_BUTTON}
          aria-pressed={active}
          aria-label={active ? `Hide ${entry.id}` : `Show ${entry.id} over the image`}
          title={active ? "Hide this mask" : "Show this mask over the image"}
          onClick={() => maskStore.setActive(imageKey, active ? null : entry.id)}
        >
          {active ? <Eye size={12} className="text-amber-300" /> : <EyeOff size={12} />}
        </button>
        <div className="min-w-0 flex-1">
          <div className="text-[11px] text-zinc-200 font-mono">
            {entry.id} · {MASK_KIND_LABELS[entry.kind]}
          </div>
          <div className="text-[10px] text-zinc-500 truncate" title={recipeSummary(entry.recipe)}>
            {recipeSummary(entry.recipe)}
          </div>
          <div className="text-[10px] text-zinc-400 font-mono">{entryCoverageLine(entry)}</div>
        </div>
        <span className="flex items-center gap-0.5">
          {entry.coverage === null && (
            <button type="button" className={ICON_BUTTON} disabled={locked} aria-label={`Measure the coverage of ${entry.id}`} title="Measure the coverage (builds the mask once)" onClick={() => measure(entry)}>
              <RefreshCw size={11} />
            </button>
          )}
          <button type="button" className={ICON_BUTTON} disabled={busy !== null || !drawReady} aria-label={`Export ${entry.id} as a float FITS`} title="Export as a float FITS (0 to 1) into the output folder" onClick={() => exportAs(entry, "float")}>
            <Download size={11} />
          </button>
          {entry.kind === "regions" && (
            <button type="button" className={ICON_BUTTON} disabled={busy !== null || !drawReady} aria-label={`Export ${entry.id} as a label FITS`} title="Export as a label FITS (region n = n, outside = 0) into the output folder" onClick={() => exportAs(entry, "label")}>
              <Tag size={11} />
            </button>
          )}
          <button
            type="button"
            className={`${ICON_BUTTON} ${armed ? "text-red-300" : ""}`}
            disabled={locked}
            aria-label={`Delete ${entry.id}`}
            title={armed ? CONFIRM_DELETE_LABEL : "Delete this mask; a History step that used it keeps its recipe"}
            onClick={() => press(entry.id, () => remove(entry))}
            onBlur={() => blur(entry.id)}
          >
            {armed ? <span className="text-[9px]">{CONFIRM_DELETE_LABEL}</span> : <Trash2 size={11} />}
          </button>
        </span>
      </li>
    );
  };

  return (
    <div className="flex flex-col gap-3 p-3" data-masks-tool="">
      <SectionHeader icon={<VenetianMask size={14} className="text-amber-400" />} title="Masks" subtitle={subject} truncateSubtitle />
      {compositeMode && (
        <div role="note" data-mask-composite-note="" className={NOTE}>
          {MASKS_COMPOSITE_NOTE}
        </div>
      )}

      <div role="group" aria-label="Mask kind" className="flex gap-1.5">
        {KIND_ORDER.map((kind) => {
          const unavailable = compositeMode && kind === "regions";
          const chosen = form.kind === kind;
          return (
            <button
              key={kind}
              type="button"
              data-mask-kind={kind}
              aria-pressed={chosen}
              disabled={locked || unavailable}
              title={unavailable ? REGIONS_ON_COMPOSITE_NOTE : undefined}
              onClick={() => patch({ kind })}
              className={`flex-1 py-1 rounded-md text-[10px] font-medium transition-all duration-150 disabled:opacity-40 ${
                chosen ? "bg-amber-500/20 text-amber-300 ring-1 ring-amber-500/30" : "bg-zinc-800/50 text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800"
              }`}
            >
              {MASK_KIND_LABELS[kind]}
            </button>
          );
        })}
      </div>

      <div className="flex flex-col gap-2">{kindFields()}</div>

      <div className="flex flex-col gap-2">
        <Slider label="Feather" value={form.featherPx} min={0} max={50} step={0.5} disabled={locked} accent="amber" format={(v) => `σ ${v.toFixed(1)} px`} hint={FEATHER_HINT} onChange={(featherPx) => patch({ featherPx })} />
        <Toggle label="Invert" checked={form.invert} disabled={locked} accent="amber" onChange={(invert) => patch({ invert })} />
      </div>

      <div title={createReason ?? undefined}>
        <RunButton label="Create mask" runningLabel="Creating mask…" running={busy === "create"} disabled={locked || !request.ok} accent="amber" onClick={create} />
      </div>
      {createReason && busy === null && (
        <div data-mask-why="" className={HINT}>
          {createReason}
        </div>
      )}
      <ErrorAlert message={shownError} />
      {shownResult && (
        <div role="status" data-mask-result={shownResult.mask_id} className="text-[10px] text-zinc-300 font-mono px-1">
          {maskResultLine(shownResult)}
        </div>
      )}
      {shownNotice && (
        <div role="status" data-mask-notice="" className={`${NOTE} flex items-start gap-2`} title={shownNotice.title}>
          <span className="flex-1">{shownNotice.text}</span>
          {shownNotice.restore && (
            <button
              type="button"
              className={TEXT_BUTTON}
              style={{ border: "1px solid var(--ab-border)" }}
              disabled={locked}
              aria-label={`Restore ${shownNotice.restore.maskId}`}
              title="Create the mask again from its recipe; it may come back under a new id"
              onClick={() => shownNotice.restore && restore(shownNotice.restore)}
            >
              <RotateCcw size={11} />
              Restore
            </button>
          )}
        </div>
      )}

      <div className="flex flex-col gap-1.5">
        <div className="text-[10px] uppercase tracking-wide text-zinc-500 px-1">{compositeMode ? "Masks of the composite" : "Masks of this image"}</div>
        {entries.length === 0 ? <div className="text-xs text-zinc-500 italic px-1">{NO_MASKS_NOTE}</div> : <ul className="flex flex-col gap-1">{entries.map(maskRow)}</ul>}
      </div>

      <div className="flex flex-col gap-2">
        <div className="text-[10px] uppercase tracking-wide text-zinc-500 px-1">Display</div>
        <Slider label="Opacity" value={style.opacity} min={0} max={1} step={0.05} accent="amber" format={(v) => `${Math.round(v * 100)}%`} onCommit={(opacity) => maskStore.setStyle({ opacity })} />
        <label className="flex items-center justify-between text-xs text-zinc-400">
          Tint
          <input
            type="color"
            aria-label="Tint"
            value={tintHex(style.tint)}
            onChange={(e) => {
              const tint = tintOf(e.target.value);
              if (tint) maskStore.setStyle({ tint });
            }}
            className="h-5 w-8 cursor-pointer rounded border border-zinc-700 bg-transparent"
          />
        </label>
        <Toggle label="Display Invert" checked={style.displayInvert} accent="amber" onChange={(displayInvert) => maskStore.setStyle({ displayInvert })} />
        <div className={HINT}>{DISPLAY_HINT}</div>
      </div>
    </div>
  );
}

function MasksPanel() {
  const { file } = useFileContext();
  const compositeMode = useCompositeMode();
  const target = useHistoryTarget();
  const render = useHistoryRender();
  const snapshot = useHistorySnapshot();
  const displayed = useDisplayedImage();
  const compositeDims = useWizardCompositeDims();
  const { compositePreviewUrl } = useCompositePreview();
  const runLocked = useRunLocked(compositeMode ? COMPOSITE_RUN_KEY : (target?.fileKey ?? ""));
  const onHistoryChanged = useCallback(() => (target ? loadHistory(target, render) : undefined), [target, render]);
  return (
    <MasksTool
      fileName={file?.name ?? ""}
      compositeMode={compositeMode}
      target={target}
      snapshot={snapshot}
      imageDims={compositeMode ? compositeDims : displayed.dimensions}
      compositeRevision={compositePreviewUrl ?? ""}
      runLocked={runLocked}
      onHistoryChanged={onHistoryChanged}
    />
  );
}

export default memo(MasksPanel);
