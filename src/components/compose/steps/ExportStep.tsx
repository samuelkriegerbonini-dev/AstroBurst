import { useState, useCallback } from "react";
import { Download, Loader2, Check, FolderOpen, Archive } from "lucide-react";
import type { WizardState } from "../wizard";
import { resolveRgbPaths } from "../../../utils/wizard";
import { exportRgbPng, exportFitsRgb } from "../../../services/export";
import { restretchComposite } from "../../../services/compose";
import { getExportDir, getOutputDir } from "../../../infrastructure/tauri";
import { useCompositeContext } from "../../../context/CompositeContext";
import { RunButton } from "../../ui";

interface ExportStepProps {
  state: WizardState;
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

function resolveHeaderSourcePath(state: WizardState): string | null {
  const { r, g, b } = resolveRgbPaths(state);
  return r ?? g ?? b ?? null;
}

export default function ExportStep({ state }: ExportStepProps) {
  const { compositeStfR, compositeStfG, compositeStfB } = useCompositeContext();

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
        dir = await getOutputDir();
      }

      if (state.compositeReady) {
        if (format === "png") {
          const restretchRes = await restretchComposite(
            dir,
            compositeStfR,
            compositeStfG,
            compositeStfB
          );

          if (restretchRes?.png_path) {
            setResult(restretchRes);
            setSavedPath(restretchRes.png_path);
          } else {
            throw new Error("Composite stretch failed. Re-run Blend and try again.");
          }
        } else {
          const outputPath = `${dir}/astroburst_composite_${ts}.fits`;
          const headerSource = resolveHeaderSourcePath(state);
          const res = await exportFitsRgb(
            headerSource,
            null,
            null,
            outputPath
          );
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
  }, [state, format, bitDepth, compositeStfR, compositeStfG, compositeStfB]);

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
        dir = await getOutputDir();
      }

      const filesToZip: { name: string; path: string }[] = [];
      const { r, g, b } = resolveRgbPaths(state);

      if (r) {
        const path = `${dir}/channel_r_${ts}.png`;
        await exportRgbPng(r, null, null, path, { bitDepth: 16 });
        filesToZip.push({ name: "channel_r.png", path });
      }

      if (g) {
        const path = `${dir}/channel_g_${ts}.png`;
        await exportRgbPng(null, g, null, path, { bitDepth: 16 });
        filesToZip.push({ name: "channel_g.png", path });
      }

      if (b) {
        const path = `${dir}/channel_b_${ts}.png`;
        await exportRgbPng(null, null, b, path, { bitDepth: 16 });
        filesToZip.push({ name: "channel_b.png", path });
      }

      if (state.compositeReady) {
        const res = await restretchComposite(
          dir,
          compositeStfR,
          compositeStfG,
          compositeStfB
        );

        if (res?.png_path) {
          filesToZip.push({ name: "composite_rgb.png", path: res.png_path });
        }
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
          console.error("Failed to read:", f.path, err);
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
  }, [state, compositeStfR, compositeStfG, compositeStfB]);

  const activeBins = state.bins.filter((b) => b.files.length > 0);

  return (
    <div className="flex flex-col gap-3 p-3">

      {state.compositeReady && (
        <div className="text-[10px] text-emerald-400/70 bg-emerald-500/5 border border-emerald-500/10 rounded-md px-2 py-1.5">
          Exporting calibrated composite (WB + SCNR applied, linear). PNG applies STF stretch.
        </div>
      )}

      <div className="flex items-center justify-between">
        <label className="text-xs text-zinc-400">Format</label>
        <select
          value={format}
          onChange={(e) => setFormat(e.target.value as "png" | "fits")}
          className="ab-select"
        >
          <option value="png">PNG</option>
          <option value="fits">FITS (RGB)</option>
        </select>
      </div>

      {format === "png" && (
        <div className="flex items-center justify-between">
          <label className="text-xs text-zinc-400">Bit Depth</label>
          <select
            value={bitDepth}
            onChange={(e) => setBitDepth(Number(e.target.value))}
            className="ab-select"
          >
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
          <Check size={12} className="text-teal-400" />
          <div className="flex flex-col">
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
          className="flex items-center gap-2 px-2 py-1 rounded bg-emerald-900/25 border border-emerald-600/20"
        >
          <FolderOpen size={10} />
          <span className="text-[9px] truncate">
            {savedPath.split(/[/\\]/).pop()}
          </span>
        </button>
      )}

      <button
        onClick={handleZipExport}
        disabled={zipLoading || activeBins.length === 0}
        className="flex items-center justify-center gap-2 px-3 py-2 rounded-lg text-xs"
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
            Download ZIP
          </>
        )}
      </button>

      {zipError && <div className="text-[9px] text-red-400">{zipError}</div>}
      {error && <div className="text-[9px] text-red-400">{error}</div>}
    </div>
  );
}
