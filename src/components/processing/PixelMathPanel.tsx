import { useState, useCallback, useEffect, useId, useMemo, useRef } from "react";
import { Plus, Trash2 } from "lucide-react";
import { Toggle, RunButton, ResultGrid, CompareView, ErrorAlert, SectionHeader } from "../ui";
import { useDoneFilesContext } from "../../context/PreviewContext";
import { useRegionKey } from "../../hooks/useRegionKey";
import { runPixelMath, validatePixelMath } from "../../services/pixelmath";
import type { PixelMathResult, PixelMathSlot, PixelMathValidation } from "../../shared/types/pixelmath";
import type { ProcessedFile } from "../../shared/types/fits.types";
import {
  EXAMPLE_EXPRESSIONS,
  MAX_SLOTS,
  TARGET_SYMBOL,
  autoSlotsFromFiles,
  bindMissingSlots,
  caretLines,
  missingSlots,
  nextSlotName,
  referencedSymbols,
  slotErrors,
} from "../../utils/pixelmathSlots";

interface PixelMathPanelProps {
  selectedFile: ProcessedFile | null;
  outputDir?: string;
  chainedFrom?: string;
  onPreviewUpdate?: (url: string | null | undefined) => void;
  onProcessingDone?: (result: PixelMathResult) => void;
}

const VALIDATION_DEBOUNCE_MS = 300;
const ACCENT = "violet";
const MAX_RETAINED_PANELS = 32;

interface RetainedPanelState {
  expression: string;
  slots: PixelMathSlot[];
  truncate: boolean;
  rescale: boolean;
  outputName: string;
  slotsTouched: boolean;
  result: PixelMathResult | null;
}

const retained = new Map<string, RetainedPanelState>();

function retainedFor(key: string | null): RetainedPanelState | undefined {
  return key ? retained.get(key) : undefined;
}

function retain(key: string | null, state: RetainedPanelState): void {
  if (!key) return;
  retained.delete(key);
  retained.set(key, state);
  while (retained.size > MAX_RETAINED_PANELS) {
    const oldest = retained.keys().next().value;
    if (oldest === undefined) break;
    retained.delete(oldest);
  }
}
const SLOT_NAME_SEPARATOR = "\u0000";
const ICON = (
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="text-violet-400">
    <path d="M18 7V4H6l6 8-6 8h12v-3" />
  </svg>
);

const TEXTAREA_CLASS =
  "w-full min-h-[72px] resize-y rounded-md border border-zinc-800 bg-zinc-900/60 px-2 py-1.5 font-mono text-xs text-zinc-200 focus:ring-1 focus:ring-violet-500/40";
const INPUT_CLASS =
  "rounded-md border border-zinc-800 bg-zinc-900/60 px-2 py-1 font-mono text-xs text-zinc-200 focus:ring-1 focus:ring-violet-500/40";

function baseName(path: string): string {
  return path.split(/[/\\]/).pop() ?? path;
}

function formatValue(v: number | undefined): string {
  if (v == null || !Number.isFinite(v)) return "--";
  const abs = Math.abs(v);
  if (abs !== 0 && (abs < 1e-3 || abs >= 1e6)) return v.toExponential(3);
  return v.toFixed(4);
}

export default function PixelMathPanel({
  selectedFile,
  outputDir = "./output",
  chainedFrom,
  onPreviewUpdate,
  onProcessingDone,
}: PixelMathPanelProps) {
  const { doneFiles } = useDoneFilesContext();
  const regionKey = useRegionKey();
  const targetPath = regionKey ?? selectedFile?.path ?? null;

  const initial = retainedFor(targetPath);
  const [expression, setExpression] = useState(initial?.expression ?? "");
  const [slots, setSlots] = useState<PixelMathSlot[]>(initial?.slots ?? []);
  const [truncate, setTruncate] = useState(initial?.truncate ?? false);
  const [rescale, setRescale] = useState(initial?.rescale ?? false);
  const [outputName, setOutputName] = useState(initial?.outputName ?? "");
  const [validation, setValidation] = useState<PixelMathValidation | null>(null);
  const [isRunning, setIsRunning] = useState(false);
  const [result, setResult] = useState<PixelMathResult | null>(initial?.result ?? null);
  const [error, setError] = useState<string | null>(null);
  const validationSeq = useRef(0);
  const expressionId = useId();
  const outputNameId = useId();

  const slotsTouchedRef = useRef(initial?.slotsTouched ?? false);
  const restoredForRef = useRef(targetPath);
  const targetPathRef = useRef(targetPath);
  targetPathRef.current = targetPath;

  useEffect(() => {
    if (restoredForRef.current === targetPath) return;
    restoredForRef.current = targetPath;
    const saved = retainedFor(targetPath);
    slotsTouchedRef.current = saved?.slotsTouched ?? false;
    setExpression(saved?.expression ?? "");
    setSlots(saved?.slots ?? []);
    setTruncate(saved?.truncate ?? false);
    setRescale(saved?.rescale ?? false);
    setOutputName(saved?.outputName ?? "");
    setResult(saved?.result ?? null);
    setValidation(null);
    setError(null);
  }, [targetPath]);

  useEffect(() => {
    retain(targetPath, {
      expression,
      slots,
      truncate,
      rescale,
      outputName,
      slotsTouched: slotsTouchedRef.current,
      result,
    });
  }, [targetPath, expression, slots, truncate, rescale, outputName, result]);

  useEffect(() => {
    if (slotsTouchedRef.current || doneFiles.length === 0) return;
    setSlots((prev) => {
      const kept = prev.filter((s) => s.path !== targetPath);
      if (kept.length > 0) return kept.length === prev.length ? prev : kept;
      return autoSlotsFromFiles(doneFiles, targetPath, kept);
    });
  }, [doneFiles, targetPath]);

  const nameErrors = useMemo(() => slotErrors(slots), [slots]);
  const hasSlotErrors = nameErrors.some((e) => e !== null);
  const slotNamesKey = slots.map((s) => s.name.trim()).join(SLOT_NAME_SEPARATOR);
  const unboundSymbols = useMemo(
    () => missingSlots(expression, slots.map((s) => s.name.trim())),
    [expression, slots],
  );

  useEffect(() => {
    const seq = ++validationSeq.current;
    setValidation(null);
    if (!expression.trim() || hasSlotErrors) {
      return;
    }
    const names = slotNamesKey ? slotNamesKey.split(SLOT_NAME_SEPARATOR) : [];
    const handle = setTimeout(() => {
      validatePixelMath(expression, names)
        .then((v) => {
          if (validationSeq.current === seq) setValidation(v);
        })
        .catch(() => {
          if (validationSeq.current === seq) setValidation(null);
        });
    }, VALIDATION_DEBOUNCE_MS);
    return () => clearTimeout(handle);
  }, [expression, slotNamesKey, hasSlotErrors]);

  const fileOptions = useMemo(() => {
    const seen = new Set<string>();
    const options: { value: string; label: string }[] = [];
    if (targetPath && !seen.has(targetPath)) {
      seen.add(targetPath);
      options.push({ value: targetPath, label: `${baseName(targetPath)} (current)` });
    }
    for (const f of doneFiles) {
      if (seen.has(f.path)) continue;
      seen.add(f.path);
      options.push({ value: f.path, label: f.name || baseName(f.path) });
    }
    for (const s of slots) {
      if (s.path && !seen.has(s.path)) {
        seen.add(s.path);
        options.push({ value: s.path, label: baseName(s.path) });
      }
    }
    return options;
  }, [doneFiles, targetPath, slots]);

  const addSlot = useCallback(() => {
    slotsTouchedRef.current = true;
    setSlots((prev) => {
      if (prev.length >= MAX_SLOTS) return prev;
      const defaultPath = doneFiles[0]?.path ?? targetPath ?? "";
      return [...prev, { name: nextSlotName(prev.map((s) => s.name.trim())), path: defaultPath }];
    });
  }, [doneFiles, targetPath]);

  const updateSlot = useCallback((index: number, patch: Partial<PixelMathSlot>) => {
    slotsTouchedRef.current = true;
    setSlots((prev) => prev.map((s, i) => (i === index ? { ...s, ...patch } : s)));
  }, []);

  const removeSlot = useCallback((index: number) => {
    slotsTouchedRef.current = true;
    setSlots((prev) => prev.filter((_, i) => i !== index));
  }, []);

  const bindUnbound = useCallback(
    (expr: string) => {
      setSlots((prev) => {
        const added = bindMissingSlots(expr, doneFiles, targetPath, prev);
        return added.length ? [...prev, ...added] : prev;
      });
    },
    [doneFiles, targetPath],
  );

  const insertExample = useCallback(
    (value: string) => {
      if (!value) return;
      setExpression(value);
      bindUnbound(value);
    },
    [bindUnbound],
  );

  const handleRun = useCallback(async () => {
    if (!targetPath) return;
    const ranOn = targetPath;
    const referenced = referencedSymbols(expression);
    setIsRunning(true);
    setError(null);
    setResult(null);
    try {
      const res = await runPixelMath(ranOn, outputDir, expression, {
        slots: slots
          .map((s) => ({ name: s.name.trim(), path: s.path }))
          .filter((s) => referenced.includes(s.name)),
        truncate,
        rescale,
        name: outputName,
      });
      if (ranOn !== targetPathRef.current) return;
      setResult(res);
      onPreviewUpdate?.(res.previewUrl);
      onProcessingDone?.(res);
    } catch (err: unknown) {
      if (ranOn !== targetPathRef.current) return;
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsRunning(false);
    }
  }, [targetPath, outputDir, expression, slots, truncate, rescale, outputName, onPreviewUpdate, onProcessingDone]);

  const validationError = validation && !validation.ok ? validation : null;
  const caret =
    validationError && validationError.position != null
      ? caretLines(expression, validationError.position, validationError.length ?? 1)
      : null;
  const canRun = !!targetPath && expression.trim().length > 0 && !hasSlotErrors && !validationError;

  const originalUrl =
    selectedFile?.path === targetPath
      ? selectedFile?.result?.previewUrl
      : doneFiles.find((f) => f.path === targetPath)?.result?.previewUrl;
  const resultUrl = result?.previewUrl;
  const [targetPreviewBroken, setTargetPreviewBroken] = useState(false);
  useEffect(() => setTargetPreviewBroken(false), [originalUrl]);

  return (
    <div className="flex h-full flex-col gap-4 overflow-y-auto p-4">
      <SectionHeader icon={ICON} title="PixelMath" subtitle="Per-pixel expressions over loaded images" />

      {!targetPath && (
        <div className="px-1 text-xs italic text-zinc-500">Select a FITS file to use as {TARGET_SYMBOL}.</div>
      )}
      {targetPath && (
        <div className="truncate text-[10px] text-zinc-500" title={targetPath}>
          {TARGET_SYMBOL} = <span className="text-zinc-300">{baseName(targetPath)}</span>
        </div>
      )}
      {targetPath && chainedFrom && (
        <div className="rounded-md border border-amber-500/30 bg-amber-500/10 px-2 py-1 text-[11px] text-amber-300">
          {TARGET_SYMBOL} is the file as loaded. PixelMath does not consume the processing chain, so the
          <span className="font-medium"> {chainedFrom} </span>
          result is not part of this expression — bind it to a slot if you want it.
        </div>
      )}

      <div className="flex flex-col gap-1.5">
        <div className="flex items-center justify-between">
          <label htmlFor={expressionId} className="text-xs text-zinc-400">Expression</label>
          <select
            className="ab-select"
            value=""
            disabled={isRunning}
            onChange={(e) => insertExample(e.target.value)}
            aria-label="Examples"
          >
            <option value="">Examples...</option>
            {EXAMPLE_EXPRESSIONS.map((ex) => (
              <option key={ex.expression} value={ex.expression}>
                {ex.label}: {ex.expression}
              </option>
            ))}
          </select>
        </div>
        <textarea
          id={expressionId}
          className={TEXTAREA_CLASS}
          value={expression}
          spellCheck={false}
          disabled={isRunning}
          placeholder={`${TARGET_SYMBOL} - med(${TARGET_SYMBOL})`}
          onChange={(e) => setExpression(e.target.value)}
        />
        {caret && (
          <pre className="m-0 overflow-x-auto rounded-md bg-red-950/30 px-2 py-1 font-mono text-[11px] leading-tight text-red-300">
            {caret.line}
            {"\n"}
            {caret.marker}
            {"\n"}
            {validationError?.message}
          </pre>
        )}
        {validationError && unboundSymbols.length > 0 && (
          <button
            type="button"
            className="self-start rounded-md border border-violet-500/40 bg-violet-600/15 px-2 py-1 text-[11px] text-violet-300 hover:bg-violet-600/25"
            disabled={isRunning}
            onClick={() => bindUnbound(expression)}
          >
            Bind {unboundSymbols.join(", ")} to loaded files
          </button>
        )}
        {validationError && !caret && (
          <div className="text-[11px] text-red-300">{validationError.message}</div>
        )}
        {validation?.ok && <div className="text-[10px] text-emerald-400">Expression is valid</div>}
      </div>

      <div className="flex flex-col gap-1.5">
        <div className="flex items-center justify-between">
          <label className="text-xs text-zinc-400">Image slots</label>
          <button
            type="button"
            className="flex items-center gap-1 rounded-md bg-zinc-800/60 px-2 py-0.5 text-[10px] text-zinc-300 transition-colors hover:bg-zinc-800 disabled:opacity-40"
            onClick={addSlot}
            disabled={isRunning || slots.length >= MAX_SLOTS}
          >
            <Plus size={11} /> Add slot
          </button>
        </div>
        {slots.length === 0 && (
          <div className="text-[10px] text-zinc-600">
            Only {TARGET_SYMBOL} is bound. Add slots to reference other loaded images by name.
          </div>
        )}
        {slots.map((s, i) => (
          <div key={i} className="flex flex-col gap-0.5">
            <div className="flex items-center gap-1.5">
              <input
                className={`${INPUT_CLASS} w-16`}
                value={s.name}
                disabled={isRunning}
                aria-label={`Slot ${i + 1} name`}
                onChange={(e) => updateSlot(i, { name: e.target.value })}
              />
              <select
                className="ab-select flex-1 min-w-0"
                value={s.path}
                disabled={isRunning}
                aria-label={`Slot ${i + 1} image`}
                onChange={(e) => updateSlot(i, { path: e.target.value })}
              >
                {fileOptions.map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
              </select>
              <button
                type="button"
                className="rounded-md p-1 text-zinc-500 transition-colors hover:text-red-300"
                onClick={() => removeSlot(i)}
                disabled={isRunning}
                title="Remove slot"
              >
                <Trash2 size={12} />
              </button>
            </div>
            {nameErrors[i] && <div className="text-[10px] text-red-300">{nameErrors[i]}</div>}
            {!nameErrors[i] && s.path === targetPath && (
              <div className="text-[10px] text-amber-300">
                {s.name.trim() || `Slot ${i + 1}`} is the same image as {TARGET_SYMBOL}
              </div>
            )}
          </div>
        ))}
      </div>

      <div className="flex flex-col gap-2">
        <Toggle label="Truncate result to [0, 1]" checked={truncate} disabled={isRunning} accent={ACCENT} onChange={setTruncate} />
        <Toggle label="Rescale result to [0, 1]" checked={rescale} disabled={isRunning} accent={ACCENT} onChange={setRescale} />
        <div className="flex items-center justify-between gap-2">
          <label htmlFor={outputNameId} className="text-xs text-zinc-400">Output name</label>
          <input
            id={outputNameId}
            className={`${INPUT_CLASS} w-40`}
            value={outputName}
            disabled={isRunning}
            placeholder="pixelmath"
            onChange={(e) => setOutputName(e.target.value)}
          />
        </div>
      </div>

      <RunButton label="Run PixelMath" runningLabel="Evaluating..." running={isRunning} disabled={!canRun} accent={ACCENT} onClick={handleRun} />
      <ErrorAlert message={error} />

      {result && (
        <div className="flex flex-col gap-3 animate-fade-in">
          <ResultGrid
            columns={4}
            items={[
              { label: "Size", value: `${result.dimensions[0]}x${result.dimensions[1]}` },
              { label: "Min", value: formatValue(result.stats?.min) },
              { label: "Max", value: formatValue(result.stats?.max) },
              { label: "Mean", value: formatValue(result.stats?.mean) },
              { label: "Median", value: formatValue(result.stats?.median) },
              { label: "Non-finite", value: result.non_finite_count },
              { label: "Time", value: `${(result.elapsed_ms / 1000).toFixed(2)}s` },
              { label: "Output", value: baseName(result.fits_path) },
            ]}
          />
          {result.warnings?.map((w) => (
            <div key={w} className="rounded-md border border-amber-500/30 bg-amber-500/10 px-2 py-1 text-[11px] text-amber-300">
              {w}
            </div>
          ))}
          {!!result.cleaned_files && (
            <div className="rounded-md border border-amber-500/30 bg-amber-500/10 px-2 py-1 text-[11px] text-amber-300">
              Output cleanup removed {result.cleaned_files} older file(s) (
              {((result.cleaned_bytes ?? 0) / 1048576).toFixed(0)} MB) to stay under the size cap. Inputs of this run
              were kept.
            </div>
          )}
          {originalUrl && resultUrl && !targetPreviewBroken && (
            <>
              <img src={originalUrl} alt="" className="hidden" onError={() => setTargetPreviewBroken(true)} />
              <CompareView originalUrl={originalUrl} resultUrl={resultUrl} originalLabel="Target" resultLabel="PixelMath" accent={ACCENT} />
            </>
          )}
          {originalUrl && resultUrl && targetPreviewBroken && (
            <div className="text-[11px] text-zinc-500">
              The target preview is no longer on disk, so only the PixelMath result is shown. Reload the file to
              restore the comparison.
            </div>
          )}
        </div>
      )}
    </div>
  );
}
