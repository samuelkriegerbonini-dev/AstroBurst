import { useState, useCallback, lazy, Suspense, memo, useMemo, useRef } from "react";
import { Layers, Loader2 } from "lucide-react";
import { composeRgb, drizzleStack, drizzleRgb } from "../../services/compose.service";
import { useDoneFilesContext, useRgbContext, useRenderContext, useNarrowbandContext } from "../../context/PreviewContext";
import { Slider, Toggle, RunButton, ResultGrid } from "../ui";
import SmartChannelMapper from "./SmartChannelMapper";
import type { ChannelFile, ChannelAssignment } from "./SmartChannelMapper";

const DrizzlePanel = lazy(() => import("./DrizzlePanel"));
const DrizzleRgbPanel = lazy(() => import("./DrizzleRgbPanel"));

function toChannelFiles(doneFiles: any[]): ChannelFile[] {
  return doneFiles.map((f) => ({
    id: f.id ?? f.path,
    path: f.path ?? "",
    name: f.name ?? "Unknown",
    filter: f.result?.header?.FILTER as string | undefined,
    instrument: f.result?.header?.INSTRUME as string | undefined,
    exptime: f.result?.header?.EXPTIME as number | undefined,
    previewUrl: f.result?.previewUrl,
  }));
}

function ComposeTabInner() {
  const { doneFiles } = useDoneFilesContext();
  const { setRgbChannels } = useRgbContext();
  const { setRenderedPreviewUrl } = useRenderContext();
  const { narrowbandPalette } = useNarrowbandContext();

  const [rgbLoading, setRgbLoading] = useState(false);
  const [rgbResult, setRgbResult] = useState<any>(null);
  const assignmentsRef = useRef<ChannelAssignment>({ L: null, R: null, G: null, B: null });
  const [hasL, setHasL] = useState(false);
  const [canCompose, setCanCompose] = useState(false);
  const [assignedCount, setAssignedCount] = useState(0);

  const [autoStretch, setAutoStretch] = useState(true);
  const [linkedStf, setLinkedStf] = useState(false);
  const [align, setAlign] = useState(true);
  const [alignMethod, setAlignMethod] = useState("phase_correlation");
  const [wbMode, setWbMode] = useState("auto");
  const [wbR, setWbR] = useState(1.0);
  const [wbG, setWbG] = useState(1.0);
  const [wbB, setWbB] = useState(1.0);
  const [scnrEnabled, setScnrEnabled] = useState(false);
  const [scnrMethod, setScnrMethod] = useState("average");
  const [scnrAmount, setScnrAmount] = useState(0.5);
  const [lrgbLightness, setLrgbLightness] = useState(1.0);
  const [lrgbChrominance, setLrgbChrominance] = useState(1.0);

  const [drizzleResult, setDrizzleResult] = useState<any>(null);
  const [drizzleLoading, setDrizzleLoading] = useState(false);
  const [drizzleProgress, setDrizzleProgress] = useState(0);
  const [drizzleStage, setDrizzleStage] = useState("");

  const [drizzleRgbResult, setDrizzleRgbResult] = useState<any>(null);
  const [drizzleRgbLoading, setDrizzleRgbLoading] = useState(false);
  const [drizzleRgbProgress, setDrizzleRgbProgress] = useState(0);
  const [drizzleRgbStage, setDrizzleRgbStage] = useState("");

  const channelFiles = useMemo(() => toChannelFiles(doneFiles), [doneFiles]);

  const handleAssignmentChange = useCallback((a: ChannelAssignment) => {
    assignmentsRef.current = a;
    const count = [a.R, a.G, a.B].filter(Boolean).length;
    setHasL(a.L !== null);
    setCanCompose(count >= 2);
    setAssignedCount(count);
  }, []);

  const composeOptions = useMemo(() => ({
    autoStretch,
    linkedStf,
    align,
    alignMethod: align ? alignMethod : undefined,
    wbMode,
    wbR: wbMode === "manual" ? wbR : undefined,
    wbG: wbMode === "manual" ? wbG : undefined,
    wbB: wbMode === "manual" ? wbB : undefined,
    scnrEnabled,
    scnrMethod,
    scnrAmount,
    lrgbLightness: hasL ? lrgbLightness : undefined,
    lrgbChrominance: hasL ? lrgbChrominance : undefined,
  }), [autoStretch, linkedStf, align, alignMethod, wbMode, wbR, wbG, wbB, scnrEnabled, scnrMethod, scnrAmount, hasL, lrgbLightness, lrgbChrominance]);

  const handleComposeRgb = useCallback(
    async (assignments: ChannelAssignment, options: Record<string, any>) => {
      const lPath = assignments.L?.path ?? null;
      const rPath = assignments.R?.path ?? null;
      const gPath = assignments.G?.path ?? null;
      const bPath = assignments.B?.path ?? null;
      setRgbLoading(true);
      try {
        const result = await composeRgb(lPath, rPath, gPath, bPath, "./output", options);
        setRgbResult(result);
        setRgbChannels({ r: rPath, g: gPath, b: bPath });
        if (result.previewUrl) {
          const bust = `${result.previewUrl}${result.previewUrl.includes("?") ? "&" : "?"}t=${Date.now()}`;
          setRenderedPreviewUrl(bust);
        }
      } catch (e) {
        console.error("RGB compose failed:", e);
      } finally {
        setRgbLoading(false);
      }
    },
    [setRgbChannels, setRenderedPreviewUrl],
  );

  const handleComposeClick = useCallback(() => {
    if (!canCompose) return;
    handleComposeRgb(assignmentsRef.current, composeOptions);
  }, [canCompose, handleComposeRgb, composeOptions]);

  const handleDrizzle = useCallback(
    async (paths: string[], options: any) => {
      setDrizzleLoading(true);
      setDrizzleProgress(0);
      setDrizzleStage(`Drizzling ${paths.length} frames...`);
      try {
        const result = await drizzleStack(paths, "./output", options);
        setDrizzleResult(result);
        setDrizzleProgress(100);
        setDrizzleStage("Done");
        if (result.previewUrl) {
          const bust = `${result.previewUrl}${result.previewUrl.includes("?") ? "&" : "?"}t=${Date.now()}`;
          setRenderedPreviewUrl(bust);
        }
      } catch {
        setDrizzleStage("Failed");
      } finally {
        setDrizzleLoading(false);
      }
    },
    [setRenderedPreviewUrl],
  );

  const handleDrizzleRgb = useCallback(
    async (
      rPaths: string[] | null,
      gPaths: string[] | null,
      bPaths: string[] | null,
      options: any,
    ) => {
      setDrizzleRgbLoading(true);
      setDrizzleRgbProgress(0);
      const channels = [
        rPaths && rPaths.length >= 2 ? "R" : null,
        gPaths && gPaths.length >= 2 ? "G" : null,
        bPaths && bPaths.length >= 2 ? "B" : null,
      ]
        .filter(Boolean)
        .join("+");
      setDrizzleRgbStage(`Drizzling ${channels}...`);
      try {
        const result = await drizzleRgb(rPaths, gPaths, bPaths, "./output", options);
        setDrizzleRgbResult(result);
        setDrizzleRgbProgress(100);
        setDrizzleRgbStage("Done");
        if (result.previewUrl) {
          const bust = `${result.previewUrl}${result.previewUrl.includes("?") ? "&" : "?"}t=${Date.now()}`;
          setRenderedPreviewUrl(bust);
        }
      } catch {
        setDrizzleRgbStage("Failed");
      } finally {
        setDrizzleRgbLoading(false);
      }
    },
    [setRenderedPreviewUrl],
  );

  if (doneFiles.length < 2) {
    return (
      <div className="flex flex-col items-center justify-center py-16 gap-3 text-zinc-600">
        <Layers size={32} strokeWidth={1} />
        <p className="text-sm">Need at least 2 processed files</p>
        <p className="text-xs text-zinc-700">
          Process more FITS files to enable RGB compose and drizzle
        </p>
      </div>
    );
  }

  const composeLabel = hasL
    ? `Compose LRGB (${assignedCount}/3 + L)`
    : `Compose RGB (${assignedCount}/3 channels)`;

  return (
    <Suspense
      fallback={
        <div className="flex items-center justify-center py-12">
          <Loader2 size={20} className="animate-spin text-zinc-500" />
        </div>
      }
    >
      <div className="flex flex-col gap-4">
        <SmartChannelMapper
          mode="rgb"
          files={channelFiles}
          onComposeRgb={handleComposeRgb}
          isLoading={rgbLoading}
          composeOptions={composeOptions}
          hideButton
          onAssignmentChange={handleAssignmentChange}
          paletteSuggestion={narrowbandPalette}
        />

        {hasL && (
          <div className="px-4 flex flex-col gap-2 border-t border-zinc-800/50 pt-3">
            <Slider label="Lightness" value={lrgbLightness} min={0} max={1} step={0.05} accent="violet" format={(v) => `${(v * 100).toFixed(0)}%`} onChange={setLrgbLightness} />
            <Slider label="Chrominance" value={lrgbChrominance} min={0} max={1} step={0.05} accent="violet" format={(v) => `${(v * 100).toFixed(0)}%`} onChange={setLrgbChrominance} />
          </div>
        )}

        <div className="px-4">
          <RunButton
            label={composeLabel}
            runningLabel="Composing..."
            running={rgbLoading}
            disabled={!canCompose}
            accent="violet"
            onClick={handleComposeClick}
          />
        </div>

        {rgbResult && (
          <div className="flex flex-col gap-3 px-4 animate-fade-in">
            {rgbResult.previewUrl && (
              <img src={rgbResult.previewUrl} alt="RGB composite" className="w-full rounded border border-zinc-700" />
            )}
            <ResultGrid columns={3} items={[
              { label: "R median", value: rgbResult.stats_r?.median?.toFixed(0) },
              { label: "G median", value: rgbResult.stats_g?.median?.toFixed(0) },
              { label: "B median", value: rgbResult.stats_b?.median?.toFixed(0) },
            ]} />
            {(rgbResult.offset_g || rgbResult.offset_b) && (
              <div className="text-[10px] text-zinc-500">
                Offsets — G: [{rgbResult.offset_g?.[0]}, {rgbResult.offset_g?.[1]}] B: [{rgbResult.offset_b?.[0]}, {rgbResult.offset_b?.[1]}]
              </div>
            )}
            {rgbResult.resampled && (
              <div className="text-[10px] text-amber-400/80">⚡ Auto-resampled (mixed SW/LW resolution)</div>
            )}
            {rgbResult.lrgb_applied && (
              <div className="text-[10px] text-zinc-400">
                ☀ LRGB applied (L: {(lrgbLightness * 100).toFixed(0)}%, C: {(lrgbChrominance * 100).toFixed(0)}%)
              </div>
            )}
            <div className="text-[10px] text-zinc-500">{rgbResult.elapsed_ms} ms</div>
          </div>
        )}

        <div className="flex flex-col gap-1.5 px-4 border-t border-zinc-800/50 pt-3">
          <Toggle label="Auto STF" checked={autoStretch} accent="violet" onChange={setAutoStretch} />
          <Toggle label="Linked STF" checked={linkedStf} accent="violet" onChange={setLinkedStf} />
          <Toggle label="Align channels" checked={align} accent="violet" onChange={setAlign} />

          {align && (
            <div className="flex items-center justify-between pl-4">
              <label className="text-xs text-zinc-400">Method</label>
              <select value={alignMethod} onChange={(e) => setAlignMethod(e.target.value)} className="ab-select">
                <option value="phase_correlation">Phase Correlation (sub-pixel)</option>
                <option value="affine">Star-based Affine (rotation)</option>
              </select>
            </div>
          )}

          <div className="flex items-center justify-between pt-1">
            <label className="text-xs text-zinc-400">White Balance</label>
            <select value={wbMode} onChange={(e) => setWbMode(e.target.value)} className="ab-select">
              <option value="auto">Auto (Stability)</option>
              <option value="none">None</option>
              <option value="manual">Manual</option>
            </select>
          </div>

          {wbMode === "manual" && (
            <div className="pl-4 flex flex-col gap-2">
              <Slider label="R" value={wbR} min={0.5} max={2.0} step={0.01} accent="red" format={(v) => v.toFixed(2)} onChange={setWbR} />
              <Slider label="G" value={wbG} min={0.5} max={2.0} step={0.01} accent="green" format={(v) => v.toFixed(2)} onChange={setWbG} />
              <Slider label="B" value={wbB} min={0.5} max={2.0} step={0.01} accent="blue" format={(v) => v.toFixed(2)} onChange={setWbB} />
            </div>
          )}

          <Toggle label="SCNR (Green Removal)" checked={scnrEnabled} accent="violet" onChange={setScnrEnabled} />

          {scnrEnabled && (
            <div className="pl-4 flex flex-col gap-2">
              <div className="flex items-center justify-between">
                <label className="text-xs text-zinc-400">Method</label>
                <select value={scnrMethod} onChange={(e) => setScnrMethod(e.target.value)} className="ab-select">
                  <option value="average">Average Neutral</option>
                  <option value="maximum">Maximum Neutral</option>
                </select>
              </div>
              <Slider label="Amount" value={scnrAmount} min={0} max={1} step={0.1} accent="violet" format={(v) => `${(v * 100).toFixed(0)}%`} onChange={setScnrAmount} />
            </div>
          )}
        </div>

        <DrizzlePanel
          files={doneFiles}
          onDrizzle={(paths: string[], opts: any) => handleDrizzle(paths, opts)}
          result={drizzleResult}
          isLoading={drizzleLoading}
          progress={drizzleProgress}
          progressStage={drizzleStage}
        />
        {doneFiles.length >= 3 && (
          <DrizzleRgbPanel
            files={doneFiles}
            onDrizzleRgb={handleDrizzleRgb}
            result={drizzleRgbResult}
            isLoading={drizzleRgbLoading}
            progress={drizzleRgbProgress}
            progressStage={drizzleRgbStage}
          />
        )}
      </div>
    </Suspense>
  );
}

export default memo(ComposeTabInner);
