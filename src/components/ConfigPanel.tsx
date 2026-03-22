import { useState, useCallback, useEffect } from "react";
import { Settings, Key, Save, Loader2, CheckCircle2, AlertCircle, RefreshCw } from "lucide-react";
import { getConfig, updateConfig, saveApiKey, getApiKey } from "../services/config.service";
import type { AppConfig } from "../services/config.service";

export default function ConfigPanel() {

  const [config, setConfig] = useState<AppConfig | null>(null);
  const [apiKey, setApiKey] = useState("");
  const [apiKeyMasked, setApiKeyMasked] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [saveStatus, setSaveStatus] = useState<"idle" | "success" | "error">("idle");
  const [error, setError] = useState<string | null>(null);

  const loadConfig = useCallback(async () => {
    setLoading(true);
    try {
      const [cfg, keyResult] = await Promise.all([getConfig(), getApiKey()]);
      setConfig(cfg);
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

  const handleUpdateField = useCallback(
    async (field: string, value: unknown) => {
      try {
        const updated = await updateConfig(field, value);
        setConfig(updated);
      } catch (e: unknown) {
        setError(e instanceof Error ? e.message : String(e));
      }
    },
    [],
  );

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
            placeholder={apiKeyMasked ? "Enter new key to replace..." : "Paste your API key..."}
            className="flex-1 bg-zinc-900 border border-zinc-700/50 rounded-md px-3 py-2 text-xs text-zinc-200 outline-none focus:border-teal-500/50 placeholder:text-zinc-600"
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
          Required for plate solving. Get one at nova.astrometry.net
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
              <label className="text-[10px] text-zinc-400 block mb-1">API URL</label>
              <input
                type="text"
                value={config.astrometry_api_url || ""}
                onChange={(e) => handleUpdateField("astrometry_api_url", e.target.value)}
                className="w-full bg-zinc-900 border border-zinc-700/50 rounded-md px-3 py-1.5 text-[11px] font-mono text-zinc-300 outline-none focus:border-teal-500/50"
              />
            </div>

            <div>
              <div className="flex items-center justify-between mb-1">
                <label className="text-[10px] text-zinc-400">Timeout (seconds)</label>
                <span className="text-[10px] font-mono text-zinc-500">
                  {config.plate_solve_timeout_secs}s
                </span>
              </div>
              <input
                type="range"
                min={30}
                max={600}
                step={30}
                value={config.plate_solve_timeout_secs}
                onChange={(e) => handleUpdateField("plate_solve_timeout_secs", parseInt(e.target.value))}
                className="w-full accent-teal-500"
              />
            </div>

            <div>
              <div className="flex items-center justify-between mb-1">
                <label className="text-[10px] text-zinc-400">Max Stars</label>
                <span className="text-[10px] font-mono text-zinc-500">
                  {config.plate_solve_max_stars}
                </span>
              </div>
              <input
                type="range"
                min={20}
                max={500}
                step={10}
                value={config.plate_solve_max_stars}
                onChange={(e) => handleUpdateField("plate_solve_max_stars", parseInt(e.target.value))}
                className="w-full accent-teal-500"
              />
            </div>
          </div>

          <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 p-4 space-y-3">
            <h4 className="text-xs font-semibold text-zinc-400 uppercase tracking-wider">
              Auto-Stretch (STF)
            </h4>

            <div>
              <div className="flex items-center justify-between mb-1">
                <label className="text-[10px] text-zinc-400">Target Background</label>
                <span className="text-[10px] font-mono text-zinc-500">
                  {config.auto_stretch_target_bg?.toFixed(2)}
                </span>
              </div>
              <input
                type="range"
                min={0.1}
                max={0.5}
                step={0.01}
                value={config.auto_stretch_target_bg}
                onChange={(e) => handleUpdateField("auto_stretch_target_bg", parseFloat(e.target.value))}
                className="w-full accent-teal-500"
              />
            </div>

            <div>
              <div className="flex items-center justify-between mb-1">
                <label className="text-[10px] text-zinc-400">Shadow Clipping (K)</label>
                <span className="text-[10px] font-mono text-zinc-500">
                  {config.auto_stretch_shadow_k?.toFixed(1)}
                </span>
              </div>
              <input
                type="range"
                min={-5.0}
                max={-0.5}
                step={0.1}
                value={config.auto_stretch_shadow_k}
                onChange={(e) => handleUpdateField("auto_stretch_shadow_k", parseFloat(e.target.value))}
                className="w-full accent-teal-500"
              />
            </div>
          </div>
        </>
      )}

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
