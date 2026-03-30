import { useState, useCallback } from "react";
import { Download, Archive, Loader2, Check, FolderOpen } from "lucide-react";
import type { WizardState } from "../wizard.types";
import { exportRgbPng, exportFitsRgb } from "../../../services/export.service";
import { restretchComposite } from "../../../services/compose.service";
import { getExportDir } from "../../../infrastructure/tauri";
import { RunButton } from "../../ui";

interface ExportStepProps {
  state: WizardState;
}

function resolveChannelPath(state: WizardState, binId: string): string | null {
  if (state.alignedPaths[binId]) return state.alignedPaths[binId];
  if (state.backgroundPaths[binId]) return state.backgroundPaths[binId];
  if (state.stackedPaths[binId]) return state.stackedPaths[binId];
  const bin = state.bins.find((b) => b.id === binId);
  if (bin && bin.files.length > 0) return bin.files[0];
  return null;
}

function resolveRgbPaths(state: WizardState): { r: string | null; g: string | null; b: string | null } {
  const activeBins = state.bins.filter((b) => b.files.length > 0);

  const rCandidates = ["r", "sii", "ha"];
  const gCandidates = ["g", "ha", "oiii"];
  const bCandidates = ["b", "oiii", "sii"];

  const usedIds = new Set<string>();

  const findBest = (candidates: string[], allowReuse = false): string | null => {
    for (const cid of candidates) {
      if (!allowReuse && usedIds.has(cid)) continue;
      const bin = activeBins.find((b) => b.id === cid);
      if (bin) {
        usedIds.add(cid);
        return resolveChannelPath(state, cid);
      }
    }
    return null;
  };

  const r = findBest(rCandidates);
  const g = findBest(gCandidates);
  let b = findBest(bCandidates);
  if (!b) b = findBest(bCandidates, true);

  return { r, g, b };
}

async function revealInExplorer(path: string) {
  try {
    const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
    await revealItemInDir(path);
  } catch {
    try {
      const { open } = await import("@tauri-apps/plugin-shell");
      const dir = path.replace(/[/\\][^/\\]+$/, "");
      await open(dir);
    } catch {}
  }
}

export default function ExportStep({ state }: ExportStepProps) {
  const [format, setFormat] = useState<"png" | "fits">("png");
  const [bitDepth, setBitDepth] = useState(16);
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<any>(null);
  const [error, setError] = useState("");
  const [savedPath, setSavedPath] = useState<string | null>(null);

  const [zipLoading, setZipLoading] = useState(false);
  const [zipDone, setZipDone] = useState(false);
  const [zipError, setZipError] = useState("");

  const handleExport = useCallback(async () => {
    setLoading(true);
    setError("");
    setSavedPath(null);
    try {
      const ts = Date.now();
      let dir: string;
      try {
        dir = await getExportDir();
      } catch {
        dir = "./output";
      }

      if (state.compositeReady) {
        const outputPath = `${dir}/astroburst_composite_${ts}.${format === "png" ? "png" : "fits"}`;

        if (format === "png") {
          const stfDefault = { shadow: 0, midtone: 0.5, highlight: 1 };
          const restretchRes = await restretchComposite(dir, stfDefault, stfDefault, stfDefault);

          if (restretchRes?.png_path) {
            try {
              const { copyFile } = await import("@tauri-apps/plugin-fs");
              await copyFile(restretchRes.png_path, outputPath);
              setResult({ output_path: outputPath, elapsed_ms: restretchRes.elapsed_ms });
              setSavedPath(outputPath);
            } catch {
              setResult({ output_path: restretchRes.png_path, elapsed_ms: restretchRes.elapsed_ms });
              setSavedPath(restretchRes.png_path);
            }
          } else {
            const { r, g, b } = resolveRgbPaths(state);
            const res = await exportRgbPng(r, g, b, outputPath, { bitDepth });
            setResult(res);
            setSavedPath(outputPath);
          }
        } else {
          const { r, g, b } = resolveRgbPaths(state);
          const res = await exportFitsRgb(r, g, b, outputPath);
          setResult(res);
          setSavedPath(outputPath);
        }
        return;
      }

      const { r, g, b } = resolveRgbPaths(state);

      if (!r && !g && !b) {
        throw new Error("No channel paths resolved for export");
      }

      if (format === "png") {
        const outputPath = `${dir}/astroburst_rgb_${ts}.png`;
        const res = await exportRgbPng(r, g, b, outputPath, { bitDepth });
        setResult(res);
        setSavedPath(outputPath);
      } else {
        const outputPath = `${dir}/astroburst_rgb_${ts}.fits`;
        const res = await exportFitsRgb(r, g, b, outputPath);
        setResult(res);
        setSavedPath(outputPath);
      }
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setLoading(false);
    }
  }, [state, format, bitDepth]);

  const handleZipExport = useCallback(async () => {
    setZipLoading(true);
    setZipError("");
    setZipDone(false);
    try {
      const ts = Date.now();
      let dir: string;
      try {
        dir = await getExportDir();
      } catch {
        dir = "./output";
      }

      const filesToZip: { name: string; path: string }[] = [];

      const { r, g, b } = resolveRgbPaths(state);

      if (r) {
        const pngPath = `${dir}/channel_r_${ts}.png`;
        try {
          await exportRgbPng(r, null, null, pngPath, { bitDepth: 16 });
          filesToZip.push({ name: "channel_r.png", path: pngPath });
        } catch {}
      }
      if (g) {
        const pngPath = `${dir}/channel_g_${ts}.png`;
        try {
          await exportRgbPng(null, g, null, pngPath, { bitDepth: 16 });
          filesToZip.push({ name: "channel_g.png", path: pngPath });
        } catch {}
      }
      if (b) {
        const pngPath = `${dir}/channel_b_${ts}.png`;
        try {
          await exportRgbPng(null, null, b, pngPath, { bitDepth: 16 });
          filesToZip.push({ name: "channel_b.png", path: pngPath });
        } catch {}
      }

      if (state.compositeReady) {
        const compositePath = `${dir}/composite_${ts}.png`;
        try {
          const stfDefault = { shadow: 0, midtone: 0.5, highlight: 1 };
          const res = await restretchComposite(dir, stfDefault, stfDefault, stfDefault);
          if (res?.png_path) {
            try {
              const { copyFile } = await import("@tauri-apps/plugin-fs");
              await copyFile(res.png_path, compositePath);
              filesToZip.push({ name: "composite_rgb.png", path: compositePath });
            } catch {
              filesToZip.push({ name: "composite_rgb.png", path: res.png_path });
            }
          }
        } catch {}
      }

      if (filesToZip.length === 0) {
        throw new Error("No files to zip");
      }

      const JSZip = (await import("jszip")).default;
      const { saveAs } = await import("file-saver");
      const { readFile } = await import("@tauri-apps/plugin-fs");

      const zip = new JSZip();
      for (const f of filesToZip) {
        try {
          const data = await readFile(f.path);
          zip.file(f.name, data);
        } catch (err) {
          console.error(`[AstroBurst] Failed to read ${f.path}:`, err);
        }
      }

      const blob = await zip.generateAsync({ type: "blob", compression: "STORE" });
      saveAs(blob, `astroburst-compose-${ts}.zip`);
      setZipDone(true);
      setTimeout(() => setZipDone(false), 3000);
    } catch (e: any) {
      setZipError(e?.message ?? String(e));
    } finally {
      setZipLoading(false);
    }
  }, [state]);

  const activeBins = state.bins.filter((b) => b.files.length > 0);

  return (
    <div className="flex flex-col gap-3 p-3">
      {state.compositeReady && (
        <div className="text-[10px] text-emerald-400/70 bg-emerald-500/5 border border-emerald-500/10 rounded-md px-2 py-1.5">
          Exporting from blended composite cache (includes stretch/SCNR if applied).
        </div>
      )}

      <div className="flex items-center justify-between">
        <label className="text-xs text-zinc-400">Format</label>
        <select value={format} onChange={(e) => setFormat(e.target.value as "png" | "fits")} className="ab-select">
          <option value="png">PNG</option>
          <option value="fits">FITS (RGB)</option>
        </select>
      </div>

      {format === "png" && (
        <div className="flex items-center justify-between">
          <label className="text-xs text-zinc-400">Bit Depth</label>
          <select value={bitDepth} onChange={(e) => setBitDepth(Number(e.target.value))} className="ab-select">
            <option value={8}>8-bit</option>
            <option value={16}>16-bit</option>
          </select>
        </div>
      )}

      <RunButton
        label={`Export ${format.toUpperCase()}`}
        runningLabel="Exporting..."
        running={loading}
        accent="teal"
        onClick={handleExport}
        icon={<Download size={12} />}
      />

      {result && (
        <div className="flex items-center gap-2 p-2 rounded-lg bg-teal-600/10 border border-teal-500/20">
          <Check size={12} className="text-teal-400 shrink-0" />
          <div className="flex flex-col min-w-0">
            <span className="text-[10px] text-teal-300">Export complete</span>
            {result.file_size_bytes && (
              <span className="text-[9px] text-zinc-600">
                {(result.file_size_bytes / 1024).toFixed(0)} KB, {result.elapsed_ms}ms
              </span>
            )}
          </div>
        </div>
      )}

      {savedPath && (
        <button
          onClick={() => revealInExplorer(savedPath)}
          className="w-full flex items-center gap-2 px-2.5 py-1.5 rounded bg-emerald-900/25 border border-emerald-600/20 text-left transition-colors hover:bg-emerald-900/40 group"
        >
          <FolderOpen size={10} className="text-emerald-400 shrink-0" />
          <span className="text-[9px] text-emerald-400/70 truncate group-hover:text-emerald-300/90">
            {savedPath.split(/[/\\]/).pop()}
          </span>
        </button>
      )}

      <div className="border-t border-zinc-800/30 pt-3 mt-1">
        <button
          onClick={handleZipExport}
          disabled={zipLoading || activeBins.length === 0}
          className="w-full flex items-center justify-center gap-2 px-3 py-2 rounded-lg text-xs font-medium transition-all disabled:opacity-40"
          style={{
            background: zipDone ? "rgba(16,185,129,0.15)" : "rgba(139,92,246,0.12)",
            border: zipDone ? "1px solid rgba(16,185,129,0.3)" : "1px solid rgba(139,92,246,0.25)",
            color: zipDone ? "#6ee7b7" : "#c4b5fd",
          }}
        >
          {zipLoading ? (
            <>
              <Loader2 size={13} className="animate-spin" />
              Creating ZIP...
            </>
          ) : zipDone ? (
            <>
              <Check size={13} />
              ZIP Downloaded
            </>
          ) : (
            <>
              <Archive size={13} />
              Download ZIP (all channels + composite)
            </>
          )}
        </button>
        {zipError && <div className="text-[9px] text-red-400 mt-1">{zipError}</div>}
      </div>

      {error && <div className="text-[9px] text-red-400">{error}</div>}
    </div>
  );
}
