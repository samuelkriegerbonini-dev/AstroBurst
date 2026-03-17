import { useState, useCallback } from "react";
import { Save, FileDown, FolderOpen } from "lucide-react";
import { Toggle, RunButton, ResultGrid, SectionHeader } from "../ui";
import { getExportDir } from "../../infrastructure/tauri";
import type { StfParams } from "../../shared/types";

interface BitpixOption {
  value: number;
  label: string;
}

const BITPIX_OPTIONS: BitpixOption[] = [
  { value: -32, label: "Float32 (BITPIX -32)" },
  { value: 16, label: "Int16 (BITPIX 16)" },
  { value: -64, label: "Float64 (BITPIX -64)" },
];

interface ExportOptions {
  applyStfStretch: boolean;
  shadow?: number;
  midtone?: number;
  highlight?: number;
  copyWcs: boolean;
  copyMetadata: boolean;
  bitpix: number;
}

interface RgbExportOptions {
  copyWcs: boolean;
  copyMetadata: boolean;
}

interface ExportResult {
  output_path?: string;
  file_size_bytes?: number;
  elapsed_ms?: number;
}

interface RgbChannels {
  r: string | null;
  g: string | null;
  b: string | null;
}

interface ExportPanelProps {
  filePath: string | null;
  stfParams: StfParams | null;
  onExport: (filePath: string, outputPath: string, options: ExportOptions) => Promise<void>;
  onExportRgb?: (r: string | null, g: string | null, b: string | null, outputPath: string, options: RgbExportOptions) => Promise<void>;
  rgbChannels?: RgbChannels | null;
  isLoading?: boolean;
  lastResult?: ExportResult | null;
}

const ICON = <Save size={14} className="text-amber-400" />;

async function revealInExplorer(path: string) {
  try {
    const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
    await revealItemInDir(path);
  } catch {
    try {
      const { open } = await import("@tauri-apps/plugin-shell");
      const dir = path.replace(/[/\\][^/\\]+$/, "");
      await open(dir);
    } catch {
      /* noop */
    }
  }
}

export default function ExportPanel({
                                      filePath,
                                      stfParams,
                                      onExport,
                                      onExportRgb,
                                      rgbChannels,
                                      isLoading = false,
                                      lastResult = null,
                                    }: ExportPanelProps) {
  const [applyStf, setApplyStf] = useState(false);
  const [copyWcs, setCopyWcs] = useState(true);
  const [copyMetadata, setCopyMetadata] = useState(true);
  const [bitpix, setBitpix] = useState(-32);
  const [exportDone, setExportDone] = useState(false);
  const [savedPath, setSavedPath] = useState<string | null>(null);

  const handleExport = useCallback(async () => {
    if (!filePath || !onExport) return;

    const dir = await getExportDir();
    const stem = filePath
      .split(/[/\\]/)
      .pop()
      ?.replace(/\.(fits?|zip)$/i, "") || "output";
    const suffix = applyStf ? "_stf" : "_proc";
    const outputPath = `${dir}/${stem}${suffix}.fits`;

    try {
      await onExport(filePath, outputPath, {
        applyStfStretch: applyStf,
        shadow: stfParams?.shadow,
        midtone: stfParams?.midtone,
        highlight: stfParams?.highlight,
        copyWcs,
        copyMetadata,
        bitpix,
      });
      setExportDone(true);
      setSavedPath(outputPath);
      setTimeout(() => {
        setExportDone(false);
        setSavedPath(null);
      }, 8000);
    } catch (e) {
      console.error("Export failed:", e);
    }
  }, [filePath, applyStf, stfParams, copyWcs, copyMetadata, bitpix, onExport]);

  const handleExportRgb = useCallback(async () => {
    if (!rgbChannels || !onExportRgb) return;
    const dir = await getExportDir();
    const outputPath = `${dir}/rgb_composite.fits`;
    try {
      await onExportRgb(rgbChannels.r, rgbChannels.g, rgbChannels.b, outputPath, {
        copyWcs,
        copyMetadata,
      });
      setExportDone(true);
      setSavedPath(outputPath);
      setTimeout(() => {
        setExportDone(false);
        setSavedPath(null);
      }, 8000);
    } catch (e) {
      console.error("RGB FITS export failed:", e);
    }
  }, [rgbChannels, copyWcs, copyMetadata, onExportRgb]);

  const hasRgb = rgbChannels && (rgbChannels.r || rgbChannels.g || rgbChannels.b);

  const exportLabel = exportDone ? "Saved!" : "Export as FITS";

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <SectionHeader icon={ICON} title="Export FITS" />

      <div className="flex flex-col gap-1.5">
        <Toggle label="Apply current STF stretch" checked={applyStf} accent="amber" onChange={setApplyStf} />
        <Toggle label="Copy WCS (coordinates)" checked={copyWcs} accent="amber" onChange={setCopyWcs} />
        <Toggle label="Copy observation metadata" checked={copyMetadata} accent="amber" onChange={setCopyMetadata} />
      </div>

      <div className="flex items-center justify-between">
        <label className="text-xs text-zinc-400">BITPIX</label>
        <select value={bitpix} onChange={(e) => setBitpix(Number(e.target.value))} className="ab-select">
          {BITPIX_OPTIONS.map((opt) => (
            <option key={opt.value} value={opt.value}>{opt.label}</option>
          ))}
        </select>
      </div>

      <RunButton
        label={exportLabel}
        runningLabel="Exporting..."
        running={isLoading}
        disabled={!filePath || exportDone}
        accent="amber"
        onClick={handleExport}
      />

      {hasRgb && (
        <button
          onClick={handleExportRgb}
          disabled={isLoading}
          className="w-full flex items-center justify-center gap-2 bg-pink-600/15 hover:bg-pink-600/25 text-pink-300 border border-pink-600/25 rounded px-3 py-1.5 text-xs font-medium transition-colors disabled:opacity-50"
        >
          <FileDown size={12} />
          Export RGB as FITS cube
        </button>
      )}

      {savedPath && (
        <button
          onClick={() => revealInExplorer(savedPath)}
          className="w-full flex items-center gap-2 px-3 py-2 rounded bg-emerald-900/25 border border-emerald-600/20 text-left transition-colors hover:bg-emerald-900/40 group"
        >
          <FolderOpen size={12} className="text-emerald-400 shrink-0" />
          <div className="flex flex-col min-w-0">
            <span className="text-[10px] font-semibold text-emerald-300">Saved to Downloads</span>
            <span className="text-[9px] text-emerald-400/70 truncate group-hover:text-emerald-300/90">
              {savedPath}
            </span>
          </div>
        </button>
      )}

      {lastResult && !savedPath && (
        <ResultGrid columns={3} items={[
          { label: "Output", value: lastResult.output_path?.split(/[/\\]/).pop() },
          { label: "Size", value: lastResult.file_size_bytes != null ? `${(lastResult.file_size_bytes / 1024).toFixed(0)} KB` : "--" },
          { label: "Time", value: lastResult.elapsed_ms != null ? `${lastResult.elapsed_ms} ms` : "--" },
        ]} />
      )}
    </div>
  );
}
