import { useState, useCallback, useEffect, useMemo, useId } from "react";
import { Save, FileDown, FolderOpen, Crosshair, Loader2, ImageIcon, Scissors, Archive } from "lucide-react";
import { Toggle, RunButton, ResultGrid, SectionHeader, ErrorAlert } from "../ui";
import { getExportDir } from "../../infrastructure/tauri";
import {
  compressMef,
  exportAlignedChannels,
  exportPng,
  exportRgbPng,
  parseExtnameList,
  riceSupportsBitpix,
  DEFAULT_QUANTIZE_LEVEL,
  type CompressMefResult,
  type FitsCompression,
} from "../../services/export";
import { exportStem, parseImageRef } from "../../utils/imageRef";
import { getWcsInfo } from "../../services/astrometry";
import {
  cutoutDefaultFileName,
  describeCutout,
  exportCutout,
  manualCutoutBox,
  selectedBoxRegion,
} from "../../services/cutout";
import { useRegionKey } from "../../hooks/useRegionKey";
import { useRegionDoc } from "../../hooks/useRegionStore";
import type { CutoutExportResult, CutoutSizeUnit } from "../../shared/types/cutout";
import type { StfParams } from "../../shared/types";

interface AlignedExportState {
  channels?: AlignedChannelOut[];
  elapsed_ms?: number;
}

interface AlignedChannelOut {
  channel: string;
  path?: string;
  file_size_bytes?: number;
  offset?: [number, number];
}

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
  compress: FitsCompression;
  quantizeLevel: number;
}

interface RgbExportOptions {
  copyWcs: boolean;
  copyMetadata: boolean;
  bitpix: number;
  compress: FitsCompression;
  quantizeLevel: number;
}

interface ExportResult {
  output_path?: string;
  file_size_bytes?: number;
  elapsed_ms?: number;
  compress?: string;
  quantize_level?: number;
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
  const fieldId = useId();
  const [applyStf, setApplyStf] = useState(false);
  const [copyWcs, setCopyWcs] = useState(true);
  const [copyMetadata, setCopyMetadata] = useState(true);
  const [bitpix, setBitpix] = useState(-32);
  const [riceEnabled, setRiceEnabled] = useState(false);
  const [quantizeLevel, setQuantizeLevel] = useState(String(DEFAULT_QUANTIZE_LEVEL));
  const [mefLossless, setMefLossless] = useState(true);
  const [mefQuantizeLevel, setMefQuantizeLevel] = useState(String(DEFAULT_QUANTIZE_LEVEL));
  const [mefDropExtnames, setMefDropExtnames] = useState("");
  const [mefRawExtnames, setMefRawExtnames] = useState("");
  const [mefRunning, setMefRunning] = useState(false);
  const [mefResult, setMefResult] = useState<CompressMefResult | null>(null);
  const [exportDone, setExportDone] = useState(false);
  const [savedPath, setSavedPath] = useState<string | null>(null);
  const [alignedExporting, setAlignedExporting] = useState(false);
  const [alignedResult, setAlignedResult] = useState<AlignedExportState | null>(null);
  const [alignedMethod, setAlignedMethod] = useState(alignMethod ?? "phase_correlation");
  const [pngBitDepth, setPngBitDepth] = useState(16);
  const [pngApplyStf, setPngApplyStf] = useState(false);
  const [pngExporting, setPngExporting] = useState(false);
  const [pngExported, setPngExported] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [cutoutIncludeErr, setCutoutIncludeErr] = useState(true);
  const [cutoutIncludeDq, setCutoutIncludeDq] = useState(true);
  const [cutoutUnit, setCutoutUnit] = useState<CutoutSizeUnit>("px");
  const [cutoutCentreX, setCutoutCentreX] = useState("");
  const [cutoutCentreY, setCutoutCentreY] = useState("");
  const [cutoutWidth, setCutoutWidth] = useState("");
  const [cutoutHeight, setCutoutHeight] = useState("");
  const [cutoutPixelScale, setCutoutPixelScale] = useState<number | null>(null);
  const [cutoutExporting, setCutoutExporting] = useState(false);
  const [cutoutResult, setCutoutResult] = useState<CutoutExportResult | null>(null);

  const regionKey = useRegionKey();
  const regionDoc = useRegionDoc(regionKey);
  const selectedBox = selectedBoxRegion(regionDoc.regions, regionDoc.selectedId);

  useEffect(() => {
    if (alignMethod) setAlignedMethod(alignMethod);
  }, [alignMethod]);

  useEffect(() => {
    setCutoutResult(null);
    setCutoutPixelScale(null);
    if (!filePath) return;
    let cancelled = false;
    getWcsInfo(filePath)
      .then((info) => {
        if (cancelled) return;
        const scale = info.pixel_scale_arcsec > 0 ? info.pixel_scale_arcsec : null;
        setCutoutPixelScale(scale);
        if (scale == null) setCutoutUnit("px");
      })
      .catch(() => {
        if (cancelled) return;
        setCutoutPixelScale(null);
        setCutoutUnit("px");
      });
    return () => {
      cancelled = true;
    };
  }, [filePath]);

  const riceAvailable = riceSupportsBitpix(bitpix);
  const effectiveCompress: FitsCompression = riceEnabled && riceAvailable ? "rice" : "none";
  const effectiveQuantizeLevel = useMemo(() => {
    const parsed = parseFloat(quantizeLevel);
    return Number.isFinite(parsed) && parsed > 0 ? parsed : DEFAULT_QUANTIZE_LEVEL;
  }, [quantizeLevel]);

  const handleExport = useCallback(async () => {
    if (!filePath || !onExport) return;

    setError(null);
    const dir = await getExportDir();
    const stem = exportStem(filePath);
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
        compress: effectiveCompress,
        quantizeLevel: effectiveQuantizeLevel,
      });
      setExportDone(true);
      setSavedPath(outputPath);
      setTimeout(() => {
        setExportDone(false);
        setSavedPath(null);
      }, 8000);
    } catch (e) {
      console.error("Export failed:", e);
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [filePath, applyStf, stfParams, copyWcs, copyMetadata, bitpix, effectiveCompress, effectiveQuantizeLevel, onExport]);

  const handleExportRgb = useCallback(async () => {
    if ((!rgbChannels || (!rgbChannels.r && !rgbChannels.g && !rgbChannels.b)) && !compositeStf) return;
    if (!onExportRgb) return;
    setError(null);
    const dir = await getExportDir();
    const ts = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
    const outputPath = `${dir}/rgb_composite_${ts}.fits`;
    try {
      await onExportRgb(
        rgbChannels?.r ?? null,
        rgbChannels?.g ?? null,
        rgbChannels?.b ?? null,
        outputPath, {
          copyWcs,
          copyMetadata,
          bitpix,
          compress: effectiveCompress,
          quantizeLevel: effectiveQuantizeLevel,
        });
      setExportDone(true);
      setSavedPath(outputPath);
      setTimeout(() => {
        setExportDone(false);
        setSavedPath(null);
      }, 8000);
    } catch (e) {
      console.error("RGB FITS export failed:", e);
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [rgbChannels, copyWcs, copyMetadata, bitpix, effectiveCompress, effectiveQuantizeLevel, onExportRgb, compositeStf]);

  const handleCompressMef = useCallback(async () => {
    if (!filePath) return;
    const sourcePath = parseImageRef(filePath).path;
    setError(null);
    setMefRunning(true);
    setMefResult(null);
    try {
      const dir = await getExportDir();
      const outputPath = `${dir}/${exportStem(filePath)}_compressed.fits`;
      const parsedQuantize = parseFloat(mefQuantizeLevel);
      const result = await compressMef(sourcePath, outputPath, {
        lossless: mefLossless,
        quantizeLevel: Number.isFinite(parsedQuantize) && parsedQuantize > 0 ? parsedQuantize : DEFAULT_QUANTIZE_LEVEL,
        dropExtnames: parseExtnameList(mefDropExtnames),
        rawExtnames: parseExtnameList(mefRawExtnames),
      });
      setMefResult(result);
      setSavedPath(result.output_path);
      setTimeout(() => setSavedPath(null), 8000);
    } catch (e) {
      console.error("MEF compression failed:", e);
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setMefRunning(false);
    }
  }, [filePath, mefLossless, mefQuantizeLevel, mefDropExtnames, mefRawExtnames]);

  const handleExportAligned = useCallback(async () => {
    if (!rgbChannels) return;
    setError(null);
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
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setAlignedExporting(false);
    }
  }, [rgbChannels, alignedMethod, copyWcs, copyMetadata]);

  const handleExportPng = useCallback(async () => {
    if (!filePath) return;
    setError(null);
    setPngExporting(true);
    try {
      const dir = await getExportDir();
      const stem = exportStem(filePath);
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
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPngExporting(false);
    }
  }, [filePath, pngBitDepth, pngApplyStf, stfParams]);

  const handleExportRgbPng = useCallback(async () => {
    if (!rgbChannels && !compositeStf) return;
    setError(null);
    setPngExporting(true);
    try {
      const dir = await getExportDir();
      const hasComposite = !!compositeStf;
      const hasExplicitStf = hasComposite &&
        compositeStf!.r.midtone !== 0.5 &&
        compositeStf!.g.midtone !== 0.5 &&
        compositeStf!.b.midtone !== 0.5;
      const effectiveStf = hasExplicitStf || pngApplyStf;
      const suffix = effectiveStf ? "_stf" : "";
      const outputPath = `${dir}/rgb_composite${suffix}_${pngBitDepth}bit.png`;
      await exportRgbPng(
        rgbChannels?.r ?? null,
        rgbChannels?.g ?? null,
        rgbChannels?.b ?? null,
        outputPath, {
          bitDepth: pngBitDepth,
          applyStfStretch: effectiveStf,
          shadowR: hasExplicitStf ? compositeStf!.r.shadow : undefined,
          midtoneR: hasExplicitStf ? compositeStf!.r.midtone : undefined,
          highlightR: hasExplicitStf ? compositeStf!.r.highlight : undefined,
          shadowG: hasExplicitStf ? compositeStf!.g.shadow : undefined,
          midtoneG: hasExplicitStf ? compositeStf!.g.midtone : undefined,
          highlightG: hasExplicitStf ? compositeStf!.g.highlight : undefined,
          shadowB: hasExplicitStf ? compositeStf!.b.shadow : undefined,
          midtoneB: hasExplicitStf ? compositeStf!.b.midtone : undefined,
          highlightB: hasExplicitStf ? compositeStf!.b.highlight : undefined,
        });
      setPngExported(true);
      setSavedPath(outputPath);
      setTimeout(() => {
        setPngExported(false);
        setSavedPath(null);
      }, 8000);
    } catch (e) {
      console.error("RGB PNG export failed:", e);
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPngExporting(false);
    }
  }, [rgbChannels, pngBitDepth, pngApplyStf, compositeStf]);

  const handleExportCutout = useCallback(async () => {
    if (!filePath) return;
    const region = selectedBox ?? manualCutoutBox({
      centreX: parseFloat(cutoutCentreX),
      centreY: parseFloat(cutoutCentreY),
      width: parseFloat(cutoutWidth),
      height: parseFloat(cutoutHeight),
      unit: cutoutUnit,
      pixelScaleArcsec: cutoutPixelScale,
    });
    if (!region) {
      setError(
        cutoutUnit === "arcsec" && cutoutPixelScale == null
          ? "Arcsecond sizes need a WCS pixel scale; switch the unit to px or select a box region."
          : "Select a box region or enter a centre and a positive size for the cutout.",
      );
      return;
    }
    setError(null);
    setCutoutExporting(true);
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const target = await save({
        defaultPath: cutoutDefaultFileName(filePath),
        filters: [{ name: "FITS", extensions: ["fits", "fit"] }],
        title: "Export cutout",
      });
      if (!target) return;
      const result = await exportCutout(filePath, region, {
        outputPath: target,
        includeErr: cutoutIncludeErr,
        includeDq: cutoutIncludeDq,
      });
      setCutoutResult(result);
      setSavedPath(result.output_path);
      setTimeout(() => setSavedPath(null), 8000);
    } catch (e) {
      console.error("Cutout export failed:", e);
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setCutoutExporting(false);
    }
  }, [
    filePath,
    selectedBox,
    cutoutCentreX,
    cutoutCentreY,
    cutoutWidth,
    cutoutHeight,
    cutoutUnit,
    cutoutPixelScale,
    cutoutIncludeErr,
    cutoutIncludeDq,
  ]);

  const hasRgb = (rgbChannels && (rgbChannels.r || rgbChannels.g || rgbChannels.b)) || !!compositeStf;

  const exportLabel = exportDone ? "Saved!" : "Export as FITS";
  const cutoutInputClass =
    "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-violet-500/50 w-full";

  return (
    <div className="flex flex-col gap-4 h-full overflow-y-auto">
      <SectionHeader icon={ICON} title="Export FITS" />

      <div className="flex flex-col gap-1.5">
        <Toggle label="Apply current STF stretch" checked={applyStf} accent="amber" onChange={setApplyStf} />
        <Toggle label="Copy WCS (coordinates)" checked={copyWcs} accent="amber" onChange={setCopyWcs} />
        <Toggle label="Copy observation metadata" checked={copyMetadata} accent="amber" onChange={setCopyMetadata} />
      </div>

      <div className="flex items-center justify-between">
        <label htmlFor={`${fieldId}-bitpix`} className="text-xs text-zinc-400">BITPIX</label>
        <select id={`${fieldId}-bitpix`} value={bitpix} onChange={(e) => setBitpix(Number(e.target.value))} className="ab-select">
          {BITPIX_OPTIONS.map((opt) => (
            <option key={opt.value} value={opt.value}>{opt.label}</option>
          ))}
        </select>
      </div>

      <div className="flex flex-col gap-1.5">
        <Toggle
          label="Rice compression (RICE_1)"
          checked={riceEnabled && riceAvailable}
          accent="amber"
          disabled={!riceAvailable}
          onChange={setRiceEnabled}
        />
        {!riceAvailable && (
          <p className="text-[10px] text-amber-300/70 px-1">
            RICE_1 supports BITPIX 16 and -32 only; the file is written uncompressed at BITPIX {bitpix}.
          </p>
        )}
        {riceEnabled && riceAvailable && (
          <div className="flex items-center justify-between">
            <label htmlFor={`${fieldId}-quantize`} className="text-xs text-zinc-400">Quantize level</label>
            <input
              id={`${fieldId}-quantize`}
              type="number"
              min={1}
              step={1}
              value={quantizeLevel}
              onChange={(e) => setQuantizeLevel(e.target.value)}
              className={`${cutoutInputClass} w-24`}
            />
          </div>
        )}
        {riceEnabled && riceAvailable && bitpix === 16 && (
          <p className="text-[10px] text-zinc-600 px-1">
            At BITPIX 16 the data is scaled losslessly; the quantize level applies to float output.
          </p>
        )}
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
          <label htmlFor={`${fieldId}-png-depth`} className="text-xs text-zinc-400">Bit Depth</label>
          <select id={`${fieldId}-png-depth`} value={pngBitDepth} onChange={(e) => setPngBitDepth(Number(e.target.value))} className="ab-select">
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

      <div className="flex flex-col gap-2 border-t border-zinc-800/50 pt-3">
        <SectionHeader
          icon={<Scissors size={14} className="text-violet-400" />}
          title="Cutout"
          subtitle={selectedBox ? "selected box region" : "manual box"}
        />
        {selectedBox ? (
          <p className="text-[10px] text-zinc-500 px-1 font-mono">
            {selectedBox.width.toFixed(1)} x {selectedBox.height.toFixed(1)} px at ({selectedBox.x.toFixed(1)}, {selectedBox.y.toFixed(1)})
            {selectedBox.angle !== 0 && ", rotated: bounds are used"}
          </p>
        ) : (
          <>
            <div className="flex gap-2">
              <div className="flex-1 flex flex-col gap-0.5">
                <label htmlFor={`${fieldId}-centre-x`} className="text-[9px] text-zinc-500 uppercase">Centre X (px)</label>
                <input id={`${fieldId}-centre-x`} type="number" step={0.5} value={cutoutCentreX} onChange={(e) => setCutoutCentreX(e.target.value)} className={cutoutInputClass} />
              </div>
              <div className="flex-1 flex flex-col gap-0.5">
                <label htmlFor={`${fieldId}-centre-y`} className="text-[9px] text-zinc-500 uppercase">Centre Y (px)</label>
                <input id={`${fieldId}-centre-y`} type="number" step={0.5} value={cutoutCentreY} onChange={(e) => setCutoutCentreY(e.target.value)} className={cutoutInputClass} />
              </div>
            </div>
            <div className="flex gap-2">
              <div className="flex-1 flex flex-col gap-0.5">
                <label htmlFor={`${fieldId}-cutout-width`} className="text-[9px] text-zinc-500 uppercase">Width ({cutoutUnit})</label>
                <input id={`${fieldId}-cutout-width`} type="number" min={0} step={1} value={cutoutWidth} onChange={(e) => setCutoutWidth(e.target.value)} className={cutoutInputClass} />
              </div>
              <div className="flex-1 flex flex-col gap-0.5">
                <label htmlFor={`${fieldId}-cutout-height`} className="text-[9px] text-zinc-500 uppercase">Height ({cutoutUnit})</label>
                <input id={`${fieldId}-cutout-height`} type="number" min={0} step={1} value={cutoutHeight} onChange={(e) => setCutoutHeight(e.target.value)} className={cutoutInputClass} />
              </div>
              <div className="flex flex-col gap-0.5">
                <label htmlFor={`${fieldId}-cutout-unit`} className="text-[9px] text-zinc-500 uppercase">Unit</label>
                <select id={`${fieldId}-cutout-unit`} value={cutoutUnit} onChange={(e) => setCutoutUnit(e.target.value as CutoutSizeUnit)} className="ab-select">
                  <option value="px">px</option>
                  <option value="arcsec" disabled={cutoutPixelScale == null}>arcsec</option>
                </select>
              </div>
            </div>
            {cutoutPixelScale != null && (
              <p className="text-[10px] text-zinc-600 px-1">{cutoutPixelScale.toFixed(4)} arcsec/px from the image WCS</p>
            )}
          </>
        )}
        <Toggle label="Include ERR extension" checked={cutoutIncludeErr} accent="violet" onChange={setCutoutIncludeErr} />
        <Toggle label="Include DQ extension" checked={cutoutIncludeDq} accent="violet" onChange={setCutoutIncludeDq} />
        <RunButton
          label="Export cutout (SCI/ERR/DQ)"
          runningLabel="Exporting..."
          running={cutoutExporting}
          disabled={!filePath}
          accent="violet"
          icon={<Scissors size={12} />}
          onClick={handleExportCutout}
        />
        {cutoutResult && (
          <div className="flex flex-col gap-0.5 px-1">
            <p className="text-[10px] text-violet-300 font-mono">{describeCutout(cutoutResult)}</p>
            <p className="text-[10px] text-zinc-600 font-mono">LTV1 {cutoutResult.ltv1}, LTV2 {cutoutResult.ltv2}, {cutoutResult.elapsed_ms} ms</p>
            {cutoutResult.warnings.map((w) => (
              <p key={w} className="text-[10px] text-amber-300/80">{w}</p>
            ))}
          </div>
        )}
      </div>

      <div className="flex flex-col gap-2 border-t border-zinc-800/50 pt-3">
        <SectionHeader
          icon={<Archive size={14} className="text-emerald-400" />}
          title="Compress FITS (multi-extension)"
          subtitle="keeps SCI + ERR + DQ"
        />
        <p className="text-[10px] text-zinc-500 px-1">
          Compresses every image extension of the selected file in place of collapsing it to a single HDU.
        </p>
        <div className="flex items-center justify-between">
          <label htmlFor={`${fieldId}-mef-mode`} className="text-xs text-zinc-400">Mode</label>
          <select
            id={`${fieldId}-mef-mode`}
            value={mefLossless ? "lossless" : "lossy"}
            onChange={(e) => setMefLossless(e.target.value === "lossless")}
            className="ab-select"
            disabled={mefRunning}
          >
            <option value="lossless">Lossless</option>
            <option value="lossy">Lossy (quantized)</option>
          </select>
        </div>
        {!mefLossless && (
          <div className="flex items-center justify-between">
            <label htmlFor={`${fieldId}-mef-quantize`} className="text-xs text-zinc-400">Quantize level</label>
            <input
              id={`${fieldId}-mef-quantize`}
              type="number"
              min={1}
              step={1}
              value={mefQuantizeLevel}
              onChange={(e) => setMefQuantizeLevel(e.target.value)}
              className={`${cutoutInputClass} w-24`}
            />
          </div>
        )}
        <div className="flex flex-col gap-0.5">
          <label htmlFor={`${fieldId}-mef-drop`} className="text-[9px] text-zinc-500 uppercase">Drop extensions (comma separated)</label>
          <input
            id={`${fieldId}-mef-drop`}
            type="text"
            value={mefDropExtnames}
            onChange={(e) => setMefDropExtnames(e.target.value)}
            placeholder="VAR_POISSON, VAR_RNOISE"
            className={cutoutInputClass}
          />
        </div>
        <div className="flex flex-col gap-0.5">
          <label htmlFor={`${fieldId}-mef-raw`} className="text-[9px] text-zinc-500 uppercase">Keep uncompressed (comma separated)</label>
          <input
            id={`${fieldId}-mef-raw`}
            type="text"
            value={mefRawExtnames}
            onChange={(e) => setMefRawExtnames(e.target.value)}
            placeholder="DQ"
            className={cutoutInputClass}
          />
        </div>
        <RunButton
          label="Compress FITS"
          runningLabel="Compressing..."
          running={mefRunning}
          disabled={!filePath}
          accent="emerald"
          icon={<Archive size={12} />}
          onClick={handleCompressMef}
        />
        {mefResult && (
          <div className="flex flex-col gap-0.5 px-1">
            <p className="text-[10px] text-emerald-300 font-mono">
              {(mefResult.source_size_bytes / 1024).toFixed(0)} KB → {(mefResult.output_size_bytes / 1024).toFixed(0)} KB
              {mefResult.source_size_bytes > 0 && ` (${Math.round((mefResult.output_size_bytes / mefResult.source_size_bytes) * 100)}%)`}
              , {mefResult.elapsed_ms} ms
            </p>
            <p className="text-[10px] text-zinc-500 font-mono">
              dropped: {mefResult.dropped.length > 0 ? mefResult.dropped.join(", ") : "none"}
            </p>
            <p className="text-[10px] text-zinc-500 font-mono">
              kept raw: {mefResult.kept_raw.length > 0 ? mefResult.kept_raw.join(", ") : "none"}
            </p>
          </div>
        )}
      </div>

      {hasRgb && rgbChannels && (rgbChannels.r || rgbChannels.g || rgbChannels.b) && (
        <div className="flex flex-col gap-2 border-t border-zinc-800/50 pt-3">
          <div className="flex items-center justify-between">
            <label htmlFor={`${fieldId}-align-method`} className="text-xs text-zinc-400">Align Method</label>
            <select
              id={`${fieldId}-align-method`}
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
          {alignedResult.channels.map((ch) => (
            <div key={ch.channel} className="flex items-center justify-between text-[10px] text-zinc-400">
              <span className="text-teal-300">{ch.channel}</span>
              <span className="truncate ml-2">{ch.path?.split(/[/\\]/).pop()}</span>
              <span className="ml-auto pl-2 text-zinc-600">{ch.file_size_bytes ? `${(ch.file_size_bytes / 1024).toFixed(0)} KB` : ""}</span>
            </div>
          ))}
          <div className="text-[10px] text-zinc-600">{alignedResult.elapsed_ms} ms</div>
        </div>
      )}

      <ErrorAlert message={error} />

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
        <ResultGrid columns={4} items={[
          { label: "Output", value: lastResult.output_path?.split(/[/\\]/).pop() },
          { label: "Size", value: lastResult.file_size_bytes != null ? `${(lastResult.file_size_bytes / 1024).toFixed(0)} KB` : "--" },
          {
            label: lastResult.compress === "rice" ? "Compressed" : "Compression",
            value: lastResult.compress === "rice"
              ? `${lastResult.file_size_bytes != null ? `${(lastResult.file_size_bytes / 1024).toFixed(0)} KB` : "--"} RICE q${lastResult.quantize_level ?? DEFAULT_QUANTIZE_LEVEL}`
              : "none",
          },
          { label: "Time", value: lastResult.elapsed_ms != null ? `${lastResult.elapsed_ms} ms` : "--" },
        ]} />
      )}
    </div>
  );
}
