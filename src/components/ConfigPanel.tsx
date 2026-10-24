import { useState, useCallback, useEffect, useRef, useId } from "react";
import { Settings, Key, Save, Loader2, CheckCircle2, AlertCircle, RefreshCw, HardDrive, Trash2, Download } from "lucide-react";
import { getConfig, updateConfig, saveApiKey, getApiKey, getOutputDirInfo, cleanupStorage, summarizeCleanup } from "../services/config";
import type { AppConfig, OutputDirInfo } from "../services/config";
import { getHistoryDir, getOutputDir, onCommandOutputs } from "../infrastructure/tauri";
import { listRenderRecords, useRenderActions } from "../context/PreviewContext";
import { useCompositePreview } from "../context/CompositeContext";
import { useComposeWizardContext } from "../context/ComposeWizardContext";
import { fileStore } from "../hooks/useFileStore";
import { compositeChainStore } from "../utils/compositeChainStore";
import { outputKeepList } from "../utils/outputKeep";
import { historyStore } from "../utils/historyStore";
import { watchStorage } from "../utils/storageRefresh";
import { decimalDraftText, decimalFieldState } from "../utils/decimalText";
import CatalogsSettings from "./CatalogsSettings";

const DEFAULT_MAST_QUOTA_GB = 20;
const QUOTA_LABEL = "MAST download quota";
export const QUOTA_POSITIVE_ERROR = `${QUOTA_LABEL}: Use a value greater than 0`;
const NAVIGATOR_PLATFORM = typeof navigator === "undefined" ? "" : navigator.platform ?? "";

function defaultDataFolder(platform: string): string {
  if (/^win/i.test(platform)) return "%LOCALAPPDATA%\\astroburst";
  if (/mac/i.test(platform)) return "~/Library/Application Support/astroburst";
  return "~/.local/share/astroburst";
}

function dataDirValue(draft: string): string | null {
  return draft.trim() || null;
}

type QuotaCommit = { value: number | null; error: null } | { value: null; error: string };

function quotaCommit(draft: string): QuotaCommit {
  const { value, error } = decimalFieldState(draft, { optional: true });
  if (error) return { value: null, error: `${QUOTA_LABEL}: ${error}` };
  if (value !== null && value <= 0) return { value: null, error: QUOTA_POSITIVE_ERROR };
  return { value, error: null };
}

export interface ArchiveDownloadsFieldsProps {
  dataDirId: string;
  quotaId: string;
  platform: string;
  dataDirDraft: string;
  quotaDraft: string;
  savedDataDir: string | null;
  savedQuota: number | null;
  onDataDirDraftChange: (text: string) => void;
  onQuotaDraftChange: (text: string) => void;
  onCommit: (field: "data_dir_override" | "mast_quota_gb", value: string | number | null) => void;
  onError: (message: string) => void;
}

export function ArchiveDownloadsFields({
  dataDirId,
  quotaId,
  platform,
  dataDirDraft,
  quotaDraft,
  savedDataDir,
  savedQuota,
  onDataDirDraftChange,
  onQuotaDraftChange,
  onCommit,
  onError,
}: ArchiveDownloadsFieldsProps) {
  const dataDirPending = dataDirValue(dataDirDraft) !== savedDataDir;

  const commitDataDir = () => {
    if (dataDirPending) onCommit("data_dir_override", dataDirValue(dataDirDraft));
  };

  const commitQuota = () => {
    const { value, error } = quotaCommit(quotaDraft);
    if (error !== null) {
      onError(error);
      return;
    }
    if (value !== savedQuota) onCommit("mast_quota_gb", value);
  };

  return (
    <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 p-4 space-y-3">
      <h4 className="text-xs font-semibold text-zinc-400 uppercase tracking-wider flex items-center gap-1.5">
        <Download size={12} />
        Archive Downloads
      </h4>

      <div>
        <label htmlFor={dataDirId} className="text-[10px] text-zinc-400 block mb-1">Data folder</label>
        <input
          id={dataDirId}
          type="text"
          value={dataDirDraft}
          onChange={(e) => onDataDirDraftChange(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter") commitDataDir(); }}
          placeholder={`Default: ${defaultDataFolder(platform)}`}
          className="w-full bg-zinc-900 border border-zinc-700/50 rounded-md px-3 py-1.5 text-[11px] font-mono text-zinc-300 focus:border-teal-500/50"
        />
        <p className="text-[10px] text-zinc-600 mt-1">
          MAST downloads are stored in its mast subfolder. Use an absolute path outside the app&apos;s own data folders, or
          leave empty for the default.
          {dataDirPending && <span className="text-amber-400"> Press Enter to apply.</span>}
        </p>
      </div>

      <div>
        <label htmlFor={quotaId} className="text-[10px] text-zinc-400 block mb-1">{QUOTA_LABEL} (GB)</label>
        <input
          id={quotaId}
          type="text"
          inputMode="decimal"
          value={quotaDraft}
          onChange={(e) => onQuotaDraftChange(e.target.value)}
          onBlur={commitQuota}
          onKeyDown={(e) => { if (e.key === "Enter") e.currentTarget.blur(); }}
          placeholder={String(DEFAULT_MAST_QUOTA_GB)}
          className="w-full bg-zinc-900 border border-zinc-700/50 rounded-md px-3 py-1.5 text-[11px] font-mono text-zinc-300 focus:border-teal-500/50"
        />
        <p className="text-[10px] text-zinc-600 mt-1">
          Only files downloaded from MAST count. Leave empty for {DEFAULT_MAST_QUOTA_GB} GB.
        </p>
      </div>
    </div>
  );
}

function formatMb(bytes: number): string {
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function storageLine(info: OutputDirInfo | null): string {
  if (!info) return "--";
  return `${formatMb(info.total_size)}${info.max_size ? ` of ${formatMb(info.max_size)}` : ""} · ${info.file_count} files`;
}

function isOverCap(info: OutputDirInfo | null): boolean {
  return info?.max_size != null && info.total_size > info.max_size;
}

function isUnderCap(info: OutputDirInfo | null): boolean {
  return info?.max_size != null && info.total_size <= info.max_size;
}

export default function ConfigPanel() {

  const apiUrlId = useId();
  const timeoutId = useId();
  const dataDirId = useId();
  const quotaId = useId();
  const { forgetOutputs } = useRenderActions();
  const { compositePreviewUrl } = useCompositePreview();
  const { state: wizardState } = useComposeWizardContext();
  const keepInputRef = useRef({ compositePreviewUrl, wizardState });
  keepInputRef.current = { compositePreviewUrl, wizardState };

  const [config, setConfig] = useState<AppConfig | null>(null);
  const [apiUrlDraft, setApiUrlDraft] = useState("");
  const [dataDirDraft, setDataDirDraft] = useState("");
  const [quotaDraft, setQuotaDraft] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [apiKeyMasked, setApiKeyMasked] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [saveStatus, setSaveStatus] = useState<"idle" | "success" | "error">("idle");
  const [error, setError] = useState<string | null>(null);

  const loadConfig = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [cfg, keyResult] = await Promise.all([getConfig(), getApiKey()]);
      setConfig(cfg);
      setApiUrlDraft(cfg.astrometry_api_url ?? "");
      setDataDirDraft(cfg.data_dir_override ?? "");
      setQuotaDraft(decimalDraftText(cfg.mast_quota_gb));
      if (keyResult?.key) {
        setApiKeyMasked(keyResult.key.slice(0, 4) + "..." + keyResult.key.slice(-4));
        setApiKey("");
      } else {
        setApiKeyMasked(null);
      }
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadConfig();
  }, [loadConfig]);

  const handleSaveApiKey = useCallback(async () => {
    if (!apiKey.trim()) return;
    setSaving(true);
    setSaveStatus("idle");
    setError(null);
    try {
      await saveApiKey(apiKey.trim(), "astrometry");
      setApiKeyMasked(apiKey.slice(0, 4) + "..." + apiKey.slice(-4));
      setApiKey("");
      setSaveStatus("success");
      setTimeout(() => setSaveStatus("idle"), 2000);
    } catch (e: unknown) {
      setSaveStatus("error");
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }, [apiKey]);

  const [storageInfo, setStorageInfo] = useState<OutputDirInfo | null>(null);
  const [historyInfo, setHistoryInfo] = useState<OutputDirInfo | null>(null);
  const [storageBusy, setStorageBusy] = useState(false);
  const [cleanResult, setCleanResult] = useState<string | null>(null);
  const [cleanArmed, setCleanArmed] = useState(false);

  const refreshStorage = useCallback(async () => {
    setStorageBusy(true);
    try {
      const [dir, historyDir] = await Promise.all([getOutputDir(), getHistoryDir()]);
      const [output, history] = await Promise.all([getOutputDirInfo(dir), getOutputDirInfo(historyDir)]);
      setStorageInfo(output);
      setHistoryInfo(history);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setStorageBusy(false);
    }
  }, []);

  useEffect(() => {
    refreshStorage();
  }, [refreshStorage]);

  useEffect(() => watchStorage(() => void refreshStorage(), [onCommandOutputs, historyStore.subscribe]), [refreshStorage]);

  const handleCleanup = useCallback(async () => {
    if (!cleanArmed) {
      setCleanArmed(true);
      setCleanResult(null);
      return;
    }
    setCleanArmed(false);
    setStorageBusy(true);
    setCleanResult(null);
    try {
      const [dir, historyDir] = await Promise.all([getOutputDir(), getHistoryDir()]);
      const live = keepInputRef.current;
      const keep = outputKeepList({
        files: fileStore.getFiles(),
        records: listRenderRecords(),
        wizard: live.wizardState,
        compositeChain: compositeChainStore.get(),
        previewUrls: [live.compositePreviewUrl],
        historyDir,
      });
      const result = await cleanupStorage(dir, historyDir, keep);
      const summary = summarizeCleanup(result);
      if (summary.cleanedPaths.length) forgetOutputs(summary.cleanedPaths);
      if (result.output.ok) {
        const output = result.output.result;
        setStorageInfo((prev) => ({
          output_dir: output.output_dir,
          total_size: output.total_size,
          max_size: prev?.max_size,
          file_count: output.file_count,
        }));
      }
      if (result.history.ok) {
        const history = result.history.result;
        setHistoryInfo((prev) => ({
          output_dir: history.history_root,
          total_size: history.total_size,
          max_size: prev?.max_size,
          file_count: history.file_count,
        }));
      }
      if (summary.errors.length) setError(summary.errors.join(" "));
      const replayable = summary.replayableNodes > 0 ? `, ${summary.replayableNodes} history steps now replayable` : "";
      if (summary.cleanedFiles > 0) {
        setCleanResult(`${summary.cleanedFiles} files removed (${formatMb(summary.cleanedBytes)})${replayable}`);
      } else if (!summary.errors.length) {
        setCleanResult("Nothing removed: the outputs and history steps over the cap are still in use");
      }
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setStorageBusy(false);
    }
  }, [cleanArmed, forgetOutputs]);

  const pendingSavesRef = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());
  const reqSeqRef = useRef(0);

  const persistField = useCallback(async (field: string, value: unknown) => {
    const timers = pendingSavesRef.current;
    const seq = ++reqSeqRef.current;
    try {
      const updated = await updateConfig(field, value);
      if (seq === reqSeqRef.current && timers.size === 0) setConfig(updated);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  const handleUpdateField = useCallback(
    (field: string, value: unknown) => {
      setError(null);
      setConfig((prev) => (prev ? { ...prev, [field]: value } : prev));
      const timers = pendingSavesRef.current;
      const existing = timers.get(field);
      if (existing) clearTimeout(existing);
      timers.set(field, setTimeout(() => {
        timers.delete(field);
        void persistField(field, value);
      }, 300));
    },
    [persistField],
  );

  const commitField = useCallback(
    (field: string, value: unknown) => {
      setError(null);
      const timers = pendingSavesRef.current;
      const existing = timers.get(field);
      if (existing) {
        clearTimeout(existing);
        timers.delete(field);
      }
      void persistField(field, value);
    },
    [persistField],
  );

  const commitApiUrl = useCallback(() => {
    const next = apiUrlDraft.trim();
    if (!config || next === config.astrometry_api_url) return;
    commitField("astrometry_api_url", next);
  }, [apiUrlDraft, config, commitField]);

  const underCap = isUnderCap(storageInfo) && (historyInfo === null || isUnderCap(historyInfo));

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 size={20} className="animate-spin text-zinc-500" />
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 p-4">
        <div className="flex items-center justify-between mb-3">
          <h4 className="text-xs font-semibold text-teal-400 uppercase tracking-wider flex items-center gap-1.5">
            <Key size={12} />
            Astrometry.net API Key
          </h4>
          {apiKeyMasked && (
            <span className="text-[10px] font-mono text-emerald-400/60">{apiKeyMasked}</span>
          )}
        </div>
        <div className="flex gap-2">
          <input
            type="password"
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") handleSaveApiKey(); }}
            placeholder={apiKeyMasked ? "Enter new key to replace..." : "Paste your API key..."}
            aria-label="Astrometry.net API key"
            className="flex-1 min-w-0 bg-zinc-900 border border-zinc-700/50 rounded-md px-3 py-2 text-xs text-zinc-200 focus:border-teal-500/50 placeholder:text-zinc-600"
          />
          <button
            onClick={handleSaveApiKey}
            disabled={!apiKey.trim() || saving}
            className="flex items-center gap-1.5 px-3 py-2 rounded-md text-xs font-medium transition-all disabled:opacity-30 disabled:cursor-not-allowed"
            style={{
              background: "rgba(20,184,166,0.12)",
              color: "#5eead4",
              border: "1px solid rgba(20,184,166,0.2)",
            }}
          >
            {saving ? <Loader2 size={12} className="animate-spin" /> : saveStatus === "success" ? <CheckCircle2 size={12} /> : <Save size={12} />}
            {saveStatus === "success" ? "Saved" : "Save"}
          </button>
        </div>
        <p className="text-[10px] text-zinc-600 mt-2">
          Only for the optional astrometry.net fallback after an offline solve fails. Get one at nova.astrometry.net
        </p>
      </div>

      {config && (
        <>
          <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 p-4 space-y-3">
            <h4 className="text-xs font-semibold text-zinc-400 uppercase tracking-wider flex items-center gap-1.5">
              <Settings size={12} />
              Plate Solving
            </h4>

            <div>
              <label htmlFor={apiUrlId} className="text-[10px] text-zinc-400 block mb-1">API URL</label>
              <input
                id={apiUrlId}
                type="text"
                value={apiUrlDraft}
                onChange={(e) => setApiUrlDraft(e.target.value)}
                onBlur={commitApiUrl}
                onKeyDown={(e) => { if (e.key === "Enter") e.currentTarget.blur(); }}
                placeholder="https://nova.astrometry.net"
                className="w-full bg-zinc-900 border border-zinc-700/50 rounded-md px-3 py-1.5 text-[11px] font-mono text-zinc-300 focus:border-teal-500/50"
              />
              <p className="text-[10px] text-zinc-600 mt-1">
                Must be an https:// URL — the API key is sent to this host.
              </p>
            </div>

            <div>
              <div className="flex items-center justify-between mb-1">
                <label htmlFor={timeoutId} className="text-[10px] text-zinc-400">Timeout (seconds)</label>
                <span className="text-[10px] font-mono text-zinc-500">
                  {config.plate_solve_timeout_secs}s
                </span>
              </div>
              <input
                id={timeoutId}
                type="range"
                min={30}
                max={600}
                step={30}
                value={config.plate_solve_timeout_secs}
                onChange={(e) => handleUpdateField("plate_solve_timeout_secs", parseInt(e.target.value))}
                className="w-full accent-teal-500"
              />
            </div>
          </div>

          <CatalogsSettings />

          <ArchiveDownloadsFields
            dataDirId={dataDirId}
            quotaId={quotaId}
            platform={NAVIGATOR_PLATFORM}
            dataDirDraft={dataDirDraft}
            quotaDraft={quotaDraft}
            savedDataDir={config.data_dir_override}
            savedQuota={config.mast_quota_gb}
            onDataDirDraftChange={setDataDirDraft}
            onQuotaDraftChange={setQuotaDraft}
            onCommit={commitField}
            onError={setError}
          />
        </>
      )}

      <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 p-4">
        <div className="flex items-center justify-between mb-3">
          <h4 className="text-xs font-semibold text-teal-400 uppercase tracking-wider flex items-center gap-1.5">
            <HardDrive size={12} />
            Output Storage
          </h4>
          <button
            onClick={refreshStorage}
            disabled={storageBusy}
            className="text-zinc-500 hover:text-zinc-300 transition-colors disabled:opacity-30"
            title="Refresh storage info"
            aria-label="Refresh storage info"
          >
            <RefreshCw size={12} className={storageBusy ? "animate-spin" : ""} />
          </button>
        </div>
        <div className="flex items-center justify-between gap-3">
          <div className="flex flex-col gap-0.5 min-w-0">
            <span className="text-xs text-zinc-300 font-mono">{storageLine(storageInfo)}</span>
            <span className="text-[10px] text-zinc-400 font-mono" title={historyInfo?.output_dir}>
              History {storageLine(historyInfo)}
            </span>
            {isOverCap(historyInfo) && (
              <span className="text-[10px] text-amber-400">
                History is over the size limit. Clean up removes abandoned branches first, then older steps of the least
                recently edited images; the latest image keeps its current step. Removed steps are rebuilt by replay.
              </span>
            )}
            {isOverCap(storageInfo) && (
              <span className="text-[10px] text-amber-400">
                Over the size limit. The limit is not enforced automatically — Clean up removes the oldest outputs that
                no loaded file, processed result or wizard step still uses.
              </span>
            )}
            {storageInfo && (
              <span className="text-[10px] text-zinc-600 font-mono truncate" title={storageInfo.output_dir}>
                {storageInfo.output_dir}
              </span>
            )}
            {cleanResult && <span className="text-[10px] text-emerald-400/80">{cleanResult}</span>}
          </div>
          <button
            onClick={handleCleanup}
            onBlur={() => setCleanArmed(false)}
            disabled={storageBusy || underCap}
            title={underCap ? "under the size cap, nothing to remove" : undefined}
            className="flex items-center gap-1.5 px-3 py-2 rounded-md text-xs font-medium transition-all disabled:opacity-30 disabled:cursor-not-allowed shrink-0"
            style={{
              background: cleanArmed ? "rgba(244,63,94,0.25)" : "rgba(244,63,94,0.1)",
              color: cleanArmed ? "#fecdd3" : "#fda4af",
              border: cleanArmed ? "1px solid rgba(244,63,94,0.5)" : "1px solid rgba(244,63,94,0.2)",
            }}
          >
            <Trash2 size={12} />
            {cleanArmed ? "Confirm — deletes oldest unused outputs" : "Clean up"}
          </button>
        </div>
      </div>

      <button
        onClick={loadConfig}
        className="flex items-center justify-center gap-2 text-xs text-zinc-500 hover:text-zinc-300 transition-colors py-2"
      >
        <RefreshCw size={12} />
        Reload Config
      </button>

      {error && (
        <div className="flex items-start gap-2 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2 text-xs text-red-300">
          <AlertCircle size={14} className="shrink-0 mt-0.5" />
          {error}
        </div>
      )}
    </div>
  );
}
