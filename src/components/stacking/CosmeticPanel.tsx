import { useState, useCallback, useId, useMemo } from "react";
import { Wand2, Layers, AlertTriangle, CheckCircle2 } from "lucide-react";
import { Slider, Toggle, RunButton, ResultGrid, CompareView, ErrorAlert, SectionHeader } from "../ui";
import { cosmeticCorrect, cosmeticCorrectBatch } from "../../services/cosmetic";
import { useDoneFilesContext } from "../../context/PreviewContext";
import { parseDefectList, formatDefectError } from "../../utils/defectList";
import { withVersionParam } from "../../utils/processingChain";
import type { ProcessedFile } from "../../shared/types/fits.types";
import type {
  CosmeticBatchResult,
  CosmeticConfig,
  CosmeticOptions,
  CosmeticReplacement,
  CosmeticResult,
} from "../../shared/types/cosmetic";
import type { RunTarget } from "./StackingTab";

interface CosmeticPanelProps {
  selectedFile: ProcessedFile | null;
  outputDir?: string;
  runTarget?: RunTarget | null;
  onProcessingDone?: (result: CosmeticResult, target: RunTarget | null) => void;
  onBatchDone?: (result: CosmeticBatchResult) => void | Promise<void>;
}

const ACCENT = "violet";
const MAX_LISTED_ERRORS = 4;
const MAX_BATCH_ERRORS = 4;
const DEFECT_PLACEHOLDER = "Point x y\nCol x [y0 y1]\nRow y [x0 x1]\n# 0-based, one entry per line";

const ICON = <Wand2 size={14} className="text-violet-400" />;

function fileLabel(path: string | undefined): string {
  return path?.split(/[/\\]/).pop() ?? "";
}

export default function CosmeticPanel({ selectedFile, outputDir = "./output", runTarget = null, onProcessingDone, onBatchDone }: CosmeticPanelProps) {
  const { doneFiles } = useDoneFilesContext();

  const [useMasterDark, setUseMasterDark] = useState(true);
  const [masterDarkPath, setMasterDarkPath] = useState("");
  const [darkHotEnabled, setDarkHotEnabled] = useState(true);
  const [darkHotSigma, setDarkHotSigma] = useState(3.0);
  const [darkColdEnabled, setDarkColdEnabled] = useState(false);
  const [darkColdSigma, setDarkColdSigma] = useState(3.0);

  const [autoEnabled, setAutoEnabled] = useState(false);
  const [autoHotEnabled, setAutoHotEnabled] = useState(true);
  const [autoHotSigma, setAutoHotSigma] = useState(3.0);
  const [autoColdEnabled, setAutoColdEnabled] = useState(false);
  const [autoColdSigma, setAutoColdSigma] = useState(3.0);

  const [listEnabled, setListEnabled] = useState(false);
  const [defectText, setDefectText] = useState("");

  const [cfa, setCfa] = useState(false);
  const [replacement, setReplacement] = useState<CosmeticReplacement>("median");
  const [amount, setAmount] = useState(1.0);

  const [isRunning, setIsRunning] = useState(false);
  const [isBatchRunning, setIsBatchRunning] = useState(false);
  const [lastResult, setResult] = useState<CosmeticResult | null>(null);
  const [batchResult, setBatchResult] = useState<CosmeticBatchResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const masterDarkId = useId();
  const replacementId = useId();

  const parsedList = useMemo(() => parseDefectList(defectText), [defectText]);
  const listErrors = listEnabled ? parsedList.errors : [];

  const lights = useMemo(
    () => doneFiles.filter((f) => f.path && f.path !== masterDarkPath),
    [doneFiles, masterDarkPath],
  );

  const darkReady = !useMasterDark || masterDarkPath !== "";
  const darkActive = useMasterDark && (darkHotEnabled || darkColdEnabled);
  const autoActive = autoEnabled && (autoHotEnabled || autoColdEnabled);
  const listActive = listEnabled && parsedList.defects.length > 0;
  const anyMethod = darkActive || autoActive || listActive;
  const canRun = darkReady && anyMethod && listErrors.length === 0;
  const busy = isRunning || isBatchRunning;

  const buildOptions = useCallback((): CosmeticOptions => {
    const config: CosmeticConfig = {
      use_master_dark: useMasterDark,
      dark_hot_sigma: useMasterDark && darkHotEnabled ? darkHotSigma : null,
      dark_cold_sigma: useMasterDark && darkColdEnabled ? darkColdSigma : null,
      auto_hot_sigma: autoEnabled && autoHotEnabled ? autoHotSigma : null,
      auto_cold_sigma: autoEnabled && autoColdEnabled ? autoColdSigma : null,
      defects: [],
      cfa,
      amount,
      replacement,
    };
    return {
      masterDarkPath: useMasterDark ? masterDarkPath : null,
      config,
      defectListText: listEnabled ? defectText : null,
    };
  }, [
    useMasterDark, masterDarkPath, darkHotEnabled, darkHotSigma, darkColdEnabled, darkColdSigma,
    autoEnabled, autoHotEnabled, autoHotSigma, autoColdEnabled, autoColdSigma,
    listEnabled, defectText, cfa, amount, replacement,
  ]);

  const handleRun = useCallback(async () => {
    if (!selectedFile?.path || !canRun) return;
    const target = runTarget;
    setIsRunning(true);
    setError(null);
    setResult(null);
    setBatchResult(null);
    try {
      const res = await cosmeticCorrect(selectedFile.path, outputDir, buildOptions());
      setResult(res.previewUrl ? { ...res, previewUrl: withVersionParam(res.previewUrl, Date.now()) } : res);
      onProcessingDone?.(res, target);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setIsRunning(false);
    }
  }, [selectedFile, canRun, outputDir, buildOptions, runTarget, onProcessingDone]);

  const handleBatch = useCallback(async () => {
    if (lights.length === 0 || !canRun) return;
    setIsBatchRunning(true);
    setError(null);
    setResult(null);
    setBatchResult(null);
    try {
      const res = await cosmeticCorrectBatch(lights.map((f) => f.path), outputDir, buildOptions());
      setBatchResult(res);
      await onBatchDone?.(res);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setIsBatchRunning(false);
    }
  }, [lights, canRun, outputDir, buildOptions, onBatchDone]);

  const result = lastResult && selectedFile?.path === lastResult.path ? lastResult : null;
  const originalUrl = selectedFile?.result?.previewUrl;
  const resultUrl = result?.previewUrl;
  const batchWarnings = batchResult?.results.filter((r) => r.dq_present).length ?? 0;

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <SectionHeader icon={ICON} title="Cosmetic Correction" subtitle="Hot / cold pixel repair before flat division" />

      {!selectedFile && (
        <div className="text-xs text-zinc-500 italic px-1">Select a light frame to enable cosmetic correction.</div>
      )}

      <div className="flex flex-col gap-2">
        <Toggle label="Use master dark" checked={useMasterDark} disabled={busy} accent={ACCENT} onChange={setUseMasterDark} />
        {useMasterDark && (
          <div className="flex flex-col gap-2 pl-2 border-l border-zinc-800">
            <div className="flex items-center justify-between gap-2">
              <label htmlFor={masterDarkId} className="text-xs text-zinc-400 shrink-0">Master dark</label>
              <select
                id={masterDarkId}
                value={masterDarkPath}
                onChange={(e) => setMasterDarkPath(e.target.value)}
                className="ab-select max-w-[60%] truncate"
                disabled={busy}
              >
                <option value="">Select a loaded frame...</option>
                {doneFiles.map((f) => (
                  <option key={f.path} value={f.path}>{f.name || fileLabel(f.path)}</option>
                ))}
              </select>
            </div>
            <Toggle label="Hot pixels" checked={darkHotEnabled} disabled={busy} accent={ACCENT} onChange={setDarkHotEnabled} />
            {darkHotEnabled && (
              <Slider label="Hot sigma" value={darkHotSigma} min={0.5} max={20} step={0.1} disabled={busy} accent={ACCENT} format={(v) => v.toFixed(1)} onChange={setDarkHotSigma} />
            )}
            <Toggle label="Cold pixels" checked={darkColdEnabled} disabled={busy} accent={ACCENT} onChange={setDarkColdEnabled} />
            {darkColdEnabled && (
              <Slider label="Cold sigma" value={darkColdSigma} min={0.5} max={20} step={0.1} disabled={busy} accent={ACCENT} format={(v) => v.toFixed(1)} onChange={setDarkColdSigma} />
            )}
          </div>
        )}

        <Toggle label="Auto detect" checked={autoEnabled} disabled={busy} accent={ACCENT} onChange={setAutoEnabled} />
        {autoEnabled && (
          <div className="flex flex-col gap-2 pl-2 border-l border-zinc-800">
            <Toggle label="Hot pixels" checked={autoHotEnabled} disabled={busy} accent={ACCENT} onChange={setAutoHotEnabled} />
            {autoHotEnabled && (
              <Slider label="Hot sigma" value={autoHotSigma} min={0.5} max={20} step={0.1} disabled={busy} accent={ACCENT} format={(v) => v.toFixed(1)} onChange={setAutoHotSigma} />
            )}
            <Toggle label="Cold pixels" checked={autoColdEnabled} disabled={busy} accent={ACCENT} onChange={setAutoColdEnabled} />
            {autoColdEnabled && (
              <Slider label="Cold sigma" value={autoColdSigma} min={0.5} max={20} step={0.1} disabled={busy} accent={ACCENT} format={(v) => v.toFixed(1)} onChange={setAutoColdSigma} />
            )}
          </div>
        )}

        <Toggle label="Defect list" checked={listEnabled} disabled={busy} accent={ACCENT} onChange={setListEnabled} />
        {listEnabled && (
          <div className="flex flex-col gap-1 pl-2 border-l border-zinc-800">
            <textarea
              value={defectText}
              onChange={(e) => setDefectText(e.target.value)}
              placeholder={DEFECT_PLACEHOLDER}
              spellCheck={false}
              disabled={busy}
              aria-label="Defect list (one entry per line, 0-based pixel coordinates)"
              className="w-full h-24 resize-y bg-zinc-900/80 border border-zinc-700/50 rounded px-2 py-1 text-[11px] font-mono text-zinc-200 placeholder:text-zinc-600 focus:border-violet-400"
            />
            <div className="text-[10px] text-zinc-500">
              {parsedList.defects.length} entr{parsedList.defects.length === 1 ? "y" : "ies"}
              {parsedList.errors.length > 0 && <span className="text-amber-400"> - {parsedList.errors.length} invalid line{parsedList.errors.length === 1 ? "" : "s"}</span>}
            </div>
            {parsedList.errors.slice(0, MAX_LISTED_ERRORS).map((err) => (
              <div key={err.line} className="text-[10px] text-amber-400/90 font-mono">{formatDefectError(err)}</div>
            ))}
          </div>
        )}

        <Toggle label="CFA (Bayer) data" checked={cfa} disabled={busy} accent={ACCENT} onChange={setCfa} />

        <div className="flex items-center justify-between">
          <label htmlFor={replacementId} className="text-xs text-zinc-400">Replacement</label>
          <select
            id={replacementId}
            value={replacement}
            onChange={(e) => setReplacement(e.target.value as CosmeticReplacement)}
            className="ab-select"
            disabled={busy}
          >
            <option value="median">Median of neighbours</option>
            <option value="mean">Mean of neighbours</option>
          </select>
        </div>

        <Slider label="Amount" value={amount} min={0} max={1} step={0.05} disabled={busy} accent={ACCENT} format={(v) => v.toFixed(2)} onChange={setAmount} />
      </div>

      {!anyMethod && (
        <div className="text-[10px] text-zinc-500 italic">Enable at least one detection method.</div>
      )}
      {useMasterDark && masterDarkPath === "" && (
        <div className="text-[10px] text-zinc-500 italic">Pick a master dark from the loaded files, or disable the master dark method.</div>
      )}

      <RunButton
        label="Run Cosmetic Correction"
        runningLabel="Correcting..."
        running={isRunning}
        disabled={!selectedFile?.path || !canRun || isBatchRunning}
        accent={ACCENT}
        onClick={handleRun}
      />
      <RunButton
        label={`Apply to All Loaded Lights (${lights.length})`}
        runningLabel="Correcting batch..."
        running={isBatchRunning}
        disabled={lights.length === 0 || !canRun || isRunning}
        accent={ACCENT}
        small
        icon={<Layers size={12} />}
        onClick={handleBatch}
      />
      <ErrorAlert message={error} />

      {result && (
        <div className="flex flex-col gap-3 animate-fade-in">
          {result.dq_present && result.warning && (
            <div className="flex items-start gap-2 text-[10px] text-amber-300 bg-amber-500/10 border border-amber-500/20 rounded-lg px-3 py-2">
              <AlertTriangle size={12} className="shrink-0 mt-0.5" />
              <span>{result.warning}</span>
            </div>
          )}
          <ResultGrid items={[
            { label: "Flagged", value: result.counts.flagged },
            { label: "Replaced", value: result.counts.replaced },
            { label: "Hot", value: result.counts.hot },
            { label: "Cold", value: result.counts.cold },
            { label: "Listed", value: result.counts.listed },
            { label: "Time", value: `${((result.elapsed_ms ?? 0) / 1000).toFixed(1)}s` },
          ]} />
          {result.fits_path && (
            <div className="text-[10px] text-zinc-500 truncate" title={result.fits_path}>
              FITS: <span className="text-zinc-300">{fileLabel(result.fits_path)}</span>
            </div>
          )}
          {originalUrl && resultUrl && (
            <CompareView originalUrl={originalUrl} resultUrl={resultUrl} originalLabel="Original" resultLabel="Corrected" accent={ACCENT} />
          )}
        </div>
      )}

      {batchResult && (
        <div className="flex flex-col gap-2 animate-fade-in bg-emerald-500/10 border border-emerald-500/20 rounded-lg px-3 py-2.5">
          <div className="flex items-center gap-1.5 text-xs text-emerald-300 font-medium">
            <CheckCircle2 size={12} />
            Batch Complete - {batchResult.succeeded} ok, {batchResult.failed} failed
          </div>
          {batchWarnings > 0 && (
            <div className="flex items-center gap-1.5 text-[10px] text-amber-300">
              <AlertTriangle size={11} />
              {batchWarnings} frame{batchWarnings === 1 ? "" : "s"} carr{batchWarnings === 1 ? "ies" : "y"} a DQ plane
            </div>
          )}
          {batchResult.results.filter((r) => r.error).slice(0, MAX_BATCH_ERRORS).map((r) => (
            <div key={r.path} className="text-[9px] text-amber-400/90 truncate" title={`${r.path}: ${r.error}`}>
              {fileLabel(r.path)}: {r.error}
            </div>
          ))}
          <div className="text-[9px] text-zinc-600">
            Corrected FITS files were written to the output folder as &lt;frame&gt;_cosmetic.fits.
          </div>
        </div>
      )}
    </div>
  );
}
