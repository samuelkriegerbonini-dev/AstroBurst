import { useState, useCallback, useEffect, useMemo, useRef } from "react";
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
  caretLines,
  nextSlotName,
  slotErrors,
} from "../../utils/pixelmathSlots";

interface PixelMathPanelProps {
  selectedFile: ProcessedFile | null;
  outputDir?: string;
  onPreviewUpdate?: (url: string | null | undefined) => void;
  onProcessingDone?: (result: PixelMathResult) => void;
}

const VALIDATION_DEBOUNCE_MS = 300;
const ACCENT = "violet";
const SLOT_NAME_SEPARATOR = "\u0000";
const ICON = (
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="text-violet-400">
    <path d="M18 7V4H6l6 8-6 8h12v-3" />
  </svg>
);

const TEXTAREA_CLASS =
  "w-full min-h-[72px] resize-y rounded-md border border-zinc-800 bg-zinc-900/60 px-2 py-1.5 font-mono text-xs text-zinc-200 focus:outline-none focus:ring-1 focus:ring-violet-500/40";
const INPUT_CLASS =
  "rounded-md border border-zinc-800 bg-zinc-900/60 px-2 py-1 font-mono text-xs text-zinc-200 focus:outline-none focus:ring-1 focus:ring-violet-500/40";

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
  onPreviewUpdate,
  onProcessingDone,
}: PixelMathPanelProps) {
  const { doneFiles } = useDoneFilesContext();
  const regionKey = useRegionKey();
  const targetPath = regionKey ?? selectedFile?.path ?? null;

  const [expression, setExpression] = useState("");
  const [slots, setSlots] = useState<PixelMathSlot[]>([]);
  const [truncate, setTruncate] = useState(false);
  const [rescale, setRescale] = useState(false);
  const [outputName, setOutputName] = useState("");
  const [validation, setValidation] = useState<PixelMathValidation | null>(null);
  const [isRunning, setIsRunning] = useState(false);
  const [result, setResult] = useState<PixelMathResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const validationSeq = useRef(0);

  const nameErrors = useMemo(() => slotErrors(slots), [slots]);
  const hasSlotErrors = nameErrors.some((e) => e !== null);
  const slotNamesKey = slots.map((s) => s.name.trim()).join(SLOT_NAME_SEPARATOR);

  useEffect(() => {
    if (!expression.trim() || hasSlotErrors) {
      setValidation(null);
      return;
    }
    const seq = ++validationSeq.current;
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
    setSlots((prev) => {
      if (prev.length >= MAX_SLOTS) return prev;
      const defaultPath = doneFiles[0]?.path ?? targetPath ?? "";
      return [...prev, { name: nextSlotName(prev.map((s) => s.name.trim())), path: defaultPath }];
    });
  }, [doneFiles, targetPath]);

  const updateSlot = useCallback((index: number, patch: Partial<PixelMathSlot>) => {
    setSlots((prev) => prev.map((s, i) => (i === index ? { ...s, ...patch } : s)));
  }, []);

  const removeSlot = useCallback((index: number) => {
    setSlots((prev) => prev.filter((_, i) => i !== index));
  }, []);

  const insertExample = useCallback((value: string) => {
    if (value) setExpression(value);
  }, []);

  const handleRun = useCallback(async () => {
    if (!targetPath) return;
    setIsRunning(true);
    setError(null);
    setResult(null);
    try {
      const res = await runPixelMath(targetPath, outputDir, expression, {
        slots: slots.map((s) => ({ name: s.name.trim(), path: s.path })),
        truncate,
        rescale,
        name: outputName,
      });
      setResult(res);
      onPreviewUpdate?.(res.previewUrl);
      onProcessingDone?.(res);
    } catch (err: unknown) {
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

  const originalUrl = selectedFile?.result?.previewUrl;
  const resultUrl = result?.previewUrl;

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

      <div className="flex flex-col gap-1.5">
        <div className="flex items-center justify-between">
          <label className="text-xs text-zinc-400">Expression</label>
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
          </div>
        ))}
      </div>

      <div className="flex flex-col gap-2">
        <Toggle label="Truncate result to [0, 1]" checked={truncate} disabled={isRunning} accent={ACCENT} onChange={setTruncate} />
        <Toggle label="Rescale result to [0, 1]" checked={rescale} disabled={isRunning} accent={ACCENT} onChange={setRescale} />
        <div className="flex items-center justify-between gap-2">
          <label className="text-xs text-zinc-400">Output name</label>
          <input
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
          {originalUrl && resultUrl && (
            <CompareView originalUrl={originalUrl} resultUrl={resultUrl} originalLabel="Target" resultLabel="PixelMath" accent={ACCENT} />
          )}
        </div>
      )}
    </div>
  );
}
