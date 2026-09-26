import { useState, useCallback, useId } from "react";
import { Download, Loader2, Check, FolderOpen, Archive } from "lucide-react";
import type { WizardState } from "../wizard";
import {
  channelExportHistory,
  compositeHistoryLines,
  exportBlockedReason,
  exportWcsWarning,
  resolveExportRgbPaths,
  resolveRgbPaths,
  wizardHeaderSourcePath,
  wizardZipChannels,
} from "../../../utils/wizard";
import { exportRgbPng, exportFitsRgbWithHeader } from "../../../services/export";
import { compositeRgbPngStf } from "../../../utils/exportSources";
import { clearCompositeCache } from "../../../services/compose";
import { getExportDir } from "../../../infrastructure/tauri";
import { useCompositeStf } from "../../../context/CompositeContext";
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

function resolveCompositeChannelPath(state: WizardState): string | null {
  const { r, g, b } = resolveRgbPaths(state);
  return r ?? g ?? b ?? null;
}

function buildHistory(state: WizardState, exported: (string | null)[]): string[] {
  return state.compositeReady
    ? compositeHistoryLines(state.compositeHistory)
    : channelExportHistory(state, exported);
}

export default function ExportStep({ state }: ExportStepProps) {
  const { compositeStfR, compositeStfG, compositeStfB, compositeStfLinked } = useCompositeStf();

  const formatId = useId();
  const bitDepthId = useId();
  const bitpixId = useId();
  const [format, setFormat] = useState<"png" | "fits">("png");
  const [bitDepth, setBitDepth] = useState(16);
  const [bitpix, setBitpix] = useState(-32);
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<{ file_size_bytes?: number; elapsed_ms?: number; bitpix?: number } | null>(null);
  const [error, setError] = useState("");
  const [headerWarning, setHeaderWarning] = useState<string | null>(null);
  const [savedPath, setSavedPath] = useState<string | null>(null);

  const [zipLoading, setZipLoading] = useState(false);
  const [zipDone, setZipDone] = useState(false);
  const [zipError, setZipError] = useState("");

  const handleExport = useCallback(async () => {
    setLoading(true);
    setError("");
    setResult(null);
    setHeaderWarning(null);
    setSavedPath(null);

    try {
      const ts = Date.now();
      const dir = await getExportDir();

      if (state.compositeReady) {
        if (format === "png") {
          const outputPath = `${dir}/astroburst_composite_${ts}.png`;
          const res = await exportRgbPng(null, null, null, outputPath, {
            bitDepth,
            ...compositeRgbPngStf({ r: compositeStfR, g: compositeStfG, b: compositeStfB }, true, compositeStfLinked),
          });
          setResult(res);
          setSavedPath(outputPath);
        } else {
          const outputPath = `${dir}/astroburst_composite_${ts}.fits`;
          const exported = await exportFitsRgbWithHeader(
            resolveCompositeChannelPath(state),
            null,
            null,
            outputPath,
            { bitpix, history: buildHistory(state, []), headerPath: wizardHeaderSourcePath(state, []) },
          );
          setResult(exported.result);
          setHeaderWarning(exportWcsWarning(exported.result, exported.headerWarning));
          setSavedPath(outputPath);
        }

        return;
      }

      const { r, g, b, monoBinId } = resolveExportRgbPaths(state);

      if (!r && !g && !b) {
        throw new Error("No channel paths resolved for export");
      }

      await clearCompositeCache().catch(() => {});

      if (format === "png") {
        const outputPath = `${dir}/astroburst_rgb_${ts}.png`;
        const res = await exportRgbPng(r, g, b, outputPath, { bitDepth, linked: true });
        setResult(res);
        setSavedPath(outputPath);
      } else {
        const outputPath = `${dir}/astroburst_rgb_${ts}.fits`;
        const exported = await exportFitsRgbWithHeader(r, g, b, outputPath, {
          bitpix,
          history: buildHistory(state, [r, g, b]),
          headerPath: wizardHeaderSourcePath(state, [r, g, b], monoBinId),
        });
        setResult(exported.result);
        setHeaderWarning(exportWcsWarning(exported.result, exported.headerWarning));
        setSavedPath(outputPath);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [state, format, bitDepth, bitpix, compositeStfR, compositeStfG, compositeStfB, compositeStfLinked]);

  const handleZipExport = useCallback(async () => {
    setZipLoading(true);
    setZipError("");
    setZipDone(false);

    try {
      const ts = Date.now();
      const dir = await getExportDir();

      const filesToZip: { name: string; path: string }[] = [];
      for (const { name, path: source } of wizardZipChannels(state)) {
        const path = `${dir}/${name}_${ts}.png`;
        await exportRgbPng(source, source, source, path, { bitDepth: 16, linked: true });
        filesToZip.push({ name: `${name}.png`, path });
      }

      if (state.compositeReady) {
        const path = `${dir}/composite_rgb_${ts}.png`;
        await exportRgbPng(null, null, null, path, {
          bitDepth: 16,
          ...compositeRgbPngStf({ r: compositeStfR, g: compositeStfG, b: compositeStfB }, true, compositeStfLinked),
        });
        filesToZip.push({ name: "composite_rgb.png", path });
      }

      if (filesToZip.length === 0) {
        throw new Error("No files to zip");
      }

      const JSZip = (await import("jszip")).default;
      const { saveAs } = await import("file-saver");
      const { readFile } = await import("@tauri-apps/plugin-fs");

      const zip = new JSZip();

      const missing: string[] = [];
      for (const f of filesToZip) {
        try {
          const data = await readFile(f.path);
          zip.file(f.name, data);
        } catch (err) {
          console.error("Failed to read:", f.path, err);
          missing.push(f.name);
        }
      }

      if (missing.length === filesToZip.length) {
        throw new Error(`ZIP failed — none of the ${filesToZip.length} files could be read`);
      }

      const blob = await zip.generateAsync({ type: "blob", compression: "STORE" });
      saveAs(blob, `astroburst-compose-${ts}.zip`);

      if (missing.length > 0) {
        setZipError(`ZIP created, but ${missing.length} of ${filesToZip.length} files could not be added: ${missing.join(", ")}`);
      }
      setZipDone(true);
      setTimeout(() => setZipDone(false), 3000);
    } catch (e) {
      setZipError(e instanceof Error ? e.message : String(e));
    } finally {
      setZipLoading(false);
    }
  }, [state, compositeStfR, compositeStfG, compositeStfB, compositeStfLinked]);

  const activeBins = state.bins.filter((b) => b.files.length > 0);
  const blockedReason = exportBlockedReason(state);
  const monoBinId = state.compositeReady ? null : resolveExportRgbPaths(state).monoBinId;
  const monoLabel = monoBinId ? state.bins.find((b) => b.id === monoBinId)?.shortLabel ?? monoBinId : null;

  return (
    <div className="flex flex-col gap-3 p-3">

      {state.compositeReady && (
        <div className="text-[10px] text-emerald-400/70 bg-emerald-500/5 border border-emerald-500/10 rounded-md px-2 py-1.5">
          PNG exports the composite as processed (stretch + curves when applied, else STF). FITS stays linear (WB + SCNR only).
        </div>
      )}

      {monoLabel && (
        <div className="text-[10px] text-amber-400/70 bg-amber-500/5 border border-amber-500/10 rounded-md px-2 py-1.5">
          {monoLabel} was processed on its own, so PNG, FITS and ZIP export that channel alone, as shown on screen. Blend the channels to export colour.
        </div>
      )}

      <div className="flex items-center justify-between">
        <label htmlFor={formatId} className="text-xs text-zinc-400">Format</label>
        <select
          id={formatId}
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
          <label htmlFor={bitDepthId} className="text-xs text-zinc-400">Bit Depth</label>
          <select
            id={bitDepthId}
            value={bitDepth}
            onChange={(e) => setBitDepth(Number(e.target.value))}
            className="ab-select"
          >
            <option value={8}>8-bit</option>
            <option value={16}>16-bit</option>
          </select>
        </div>
      )}

      {format === "fits" && (
        <div className="flex items-center justify-between">
          <label htmlFor={bitpixId} className="text-xs text-zinc-400">BITPIX</label>
          <select
            id={bitpixId}
            value={bitpix}
            onChange={(e) => setBitpix(Number(e.target.value))}
            className="ab-select"
          >
            <option value={-32}>Float32 (-32)</option>
            <option value={16}>Int16 (16)</option>
            <option value={-64}>Float64 (-64)</option>
          </select>
        </div>
      )}

      <RunButton
        label={`Export ${format.toUpperCase()}`}
        runningLabel="Exporting..."
        running={loading}
        disabled={blockedReason !== null}
        accent="teal"
        onClick={handleExport}
        icon={<Download size={12} />}
      />

      {blockedReason && <div className="text-[9px] text-zinc-500">{blockedReason}</div>}

      {result && (
        <div className="flex items-center gap-2 p-2 rounded-lg bg-teal-600/10 border border-teal-500/20">
          <Check size={12} className="text-teal-400" />
          <div className="flex flex-col">
            <span className="text-[10px] text-teal-300">Export complete</span>
            {result.file_size_bytes && (
              <span className="text-[9px] text-zinc-600">
                {(result.file_size_bytes / 1024).toFixed(0)} KB, {result.elapsed_ms}ms
                {result.bitpix && format === "fits" && ` BITPIX=${result.bitpix}`}
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
        className="flex items-center justify-center gap-2 px-3 py-2 rounded-lg text-xs disabled:opacity-40"
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

      {headerWarning && <div className="text-[9px] text-amber-400">{headerWarning}</div>}
      {zipError && <div className="text-[9px] text-red-400">{zipError}</div>}
      {error && <div className="text-[9px] text-red-400">{error}</div>}
    </div>
  );
}
