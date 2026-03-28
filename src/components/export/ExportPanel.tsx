import { useState, useCallback, useEffect } from "react";
import { Save, FileDown, FolderOpen, Crosshair, Loader2, ImageIcon } from "lucide-react";
import { Toggle, RunButton, ResultGrid, SectionHeader } from "../ui";
import { getExportDir } from "../../infrastructure/tauri";
import { exportAlignedChannels, exportPng, exportRgbPng } from "../../services/export.service";
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

interface CompositeStf {
  r: StfParams;
  g: StfParams;
  b: StfParams;
}

interface ExportPanelProps {
  filePath: string | null;
  stfParams: StfParams | null;
  onExport: (filePath: string, outputPath: string, options: ExportOptions) => Promise<void>;
  onExportRgb?: (r: string | null, g: string | null, b: string | null, outputPath: string, options: RgbExportOptions) => Promise<void>;
  rgbChannels?: RgbChannels | null;
  compositeStf?: CompositeStf | null;
  alignMethod?: string;
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
                                      compositeStf,
                                      alignMethod,
                                      isLoading = false,
                                      lastResult = null,
                                    }: ExportPanelProps) {
  const [applyStf, setApplyStf] = useState(false);
  const [copyWcs, setCopyWcs] = useState(true);
  const [copyMetadata, setCopyMetadata] = useState(true);
  const [bitpix, setBitpix] = useState(-32);
  const [exportDone, setExportDone] = useState(false);
  const [savedPath, setSavedPath] = useState<string | null>(null);
  const [alignedExporting, setAlignedExporting] = useState(false);
  const [alignedResult, setAlignedResult] = useState<any>(null);
  const [alignedMethod, setAlignedMethod] = useState(alignMethod ?? "phase_correlation");
  const [pngBitDepth, setPngBitDepth] = useState(16);
  const [pngApplyStf, setPngApplyStf] = useState(false);
  const [pngExporting, setPngExporting] = useState(false);
  const [pngExported, setPngExported] = useState(false);

  useEffect(() => {
    if (alignMethod) setAlignedMethod(alignMethod);
  }, [alignMethod]);

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

  const handleExportAligned = useCallback(async () => {
    if (!rgbChannels) return;
    setAlignedExporting(true);
    setAlignedResult(null);
    try {
      const dir = await getExportDir();
      const result = await exportAlignedChannels(
        rgbChannels.r, rgbChannels.g, rgbChannels.b, dir,
        { alignMethod: alignedMethod, copyWcs, copyMetadata },
      );
      setAlignedResult(result);
      const firstPath = result?.channels?.[0]?.path;
      if (firstPath) {
        setSavedPath(firstPath.replace(/[/\\][^/\\]+$/, ""));
        setTimeout(() => setSavedPath(null), 8000);
      }
    } catch (e) {
      console.error("Aligned export failed:", e);
    } finally {
      setAlignedExporting(false);
    }
  }, [rgbChannels, alignedMethod, copyWcs, copyMetadata]);

  const handleExportPng = useCallback(async () => {
    if (!filePath) return;
    setPngExporting(true);
    try {
      const dir = await getExportDir();
      const stem = filePath
        .split(/[/\\]/)
        .pop()
        ?.replace(/\.(fits?|zip)$/i, "") || "output";
      const suffix = pngApplyStf ? "_stf" : "";
      const outputPath = `${dir}/${stem}${suffix}.png`;
      await exportPng(filePath, outputPath, {
        bitDepth: pngBitDepth,
        applyStfStretch: pngApplyStf,
        shadow: stfParams?.shadow,
        midtone: stfParams?.midtone,
        highlight: stfParams?.highlight,
      });
      setPngExported(true);
      setSavedPath(outputPath);
      setTimeout(() => {
        setPngExported(false);
        setSavedPath(null);
      }, 8000);
    } catch (e) {
      console.error("PNG export failed:", e);
    } finally {
      setPngExporting(false);
    }
  }, [filePath, pngBitDepth, pngApplyStf, stfParams]);

  const handleExportRgbPng = useCallback(async () => {
    if (!rgbChannels) return;
    setPngExporting(true);
    try {
      const dir = await getExportDir();
      const hasComposite = !!compositeStf;
      const effectiveStf = hasComposite || pngApplyStf;
      const suffix = effectiveStf ? "_stf" : "";
      const outputPath = `${dir}/rgb_composite${suffix}_${pngBitDepth}bit.png`;
      await exportRgbPng(rgbChannels.r, rgbChannels.g, rgbChannels.b, outputPath, {
        bitDepth: pngBitDepth,
        applyStfStretch: effectiveStf,
        shadowR: hasComposite ? compositeStf!.r.shadow : undefined,
        midtoneR: hasComposite ? compositeStf!.r.midtone : undefined,
        highlightR: hasComposite ? compositeStf!.r.highlight : undefined,
        shadowG: hasComposite ? compositeStf!.g.shadow : undefined,
        midtoneG: hasComposite ? compositeStf!.g.midtone : undefined,
        highlightG: hasComposite ? compositeStf!.g.highlight : undefined,
        shadowB: hasComposite ? compositeStf!.b.shadow : undefined,
        midtoneB: hasComposite ? compositeStf!.b.midtone : undefined,
        highlightB: hasComposite ? compositeStf!.b.highlight : undefined,
      });
      setPngExported(true);
      setSavedPath(outputPath);
      setTimeout(() => {
        setPngExported(false);
        setSavedPath(null);
      }, 8000);
    } catch (e) {
      console.error("RGB PNG export failed:", e);
    } finally {
      setPngExporting(false);
    }
  }, [rgbChannels, pngBitDepth, pngApplyStf, compositeStf]);

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

      <div className="flex flex-col gap-2 border-t border-zinc-800/50 pt-3">
        <SectionHeader icon={<ImageIcon size={14} className="text-sky-400" />} title="Export PNG" />
        <div className="flex items-center justify-between">
          <label className="text-xs text-zinc-400">Bit Depth</label>
          <select value={pngBitDepth} onChange={(e) => setPngBitDepth(Number(e.target.value))} className="ab-select">
            <option value={16}>16-bit</option>
            <option value={8}>8-bit</option>
          </select>
        </div>
        <Toggle label="Apply current STF stretch" checked={pngApplyStf} accent="sky" onChange={setPngApplyStf} />
        <RunButton
          label={pngExported ? "Saved!" : "Export as PNG"}
          runningLabel="Exporting..."
          running={pngExporting}
          disabled={!filePath || pngExported}
          accent="sky"
          onClick={handleExportPng}
        />
        {hasRgb && (
          <>
            <button
              onClick={handleExportRgbPng}
              disabled={pngExporting}
              className="w-full flex items-center justify-center gap-2 bg-sky-600/15 hover:bg-sky-600/25 text-sky-300 border border-sky-600/25 rounded px-3 py-1.5 text-xs font-medium transition-colors disabled:opacity-50"
            >
              {pngExporting ? <Loader2 size={12} className="animate-spin" /> : <ImageIcon size={12} />}
              Export RGB as PNG ({pngBitDepth}-bit)
            </button>
            {compositeStf && (
              <p className="text-[10px] text-sky-400/60 px-1">
                STF stretch auto-applied from composite preview
              </p>
            )}
          </>
        )}
      </div>

      {hasRgb && (
        <div className="flex flex-col gap-2 border-t border-zinc-800/50 pt-3">
          <div className="flex items-center justify-between">
            <label className="text-xs text-zinc-400">Align Method</label>
            <select
              value={alignedMethod}
              onChange={(e) => setAlignedMethod(e.target.value)}
              className="ab-select"
              disabled={alignedExporting}
            >
              <option value="phase_correlation">Phase Correlation</option>
              <option value="affine">Star-based Affine</option>
            </select>
          </div>
          <button
            onClick={handleExportAligned}
            disabled={alignedExporting}
            className="w-full flex items-center justify-center gap-2 bg-teal-600/15 hover:bg-teal-600/25 text-teal-300 border border-teal-600/25 rounded px-3 py-1.5 text-xs font-medium transition-colors disabled:opacity-50"
          >
            {alignedExporting ? <Loader2 size={12} className="animate-spin" /> : <Crosshair size={12} />}
            {alignedExporting ? "Aligning..." : "Export Aligned Channels (FITS)"}
          </button>
        </div>
      )}

      {alignedResult?.channels && (
        <div className="flex flex-col gap-1 px-1">
          {alignedResult.channels.map((ch: any) => (
            <div key={ch.channel} className="flex items-center justify-between text-[10px] text-zinc-400">
              <span className="text-teal-300">{ch.channel}</span>
              <span className="truncate ml-2">{ch.path?.split(/[/\\]/).pop()}</span>
              <span className="ml-auto pl-2 text-zinc-600">{ch.file_size_bytes ? `${(ch.file_size_bytes / 1024).toFixed(0)} KB` : ""}</span>
            </div>
          ))}
          <div className="text-[10px] text-zinc-600">{alignedResult.elapsed_ms} ms</div>
        </div>
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
