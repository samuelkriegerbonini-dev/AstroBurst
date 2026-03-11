import { useState, useCallback, lazy, Suspense, memo } from "react";
import { Layers, Loader2 } from "lucide-react";
import { useBackend } from "../../hooks/useBackend";
import { useFileContext, useRgbContext, useRenderContext, useNarrowbandContext } from "../../context/PreviewContext";

const RgbComposePanel = lazy(() => import("./RgbComposePanel"));
const DrizzlePanel = lazy(() => import("./DrizzlePanel"));
const DrizzleRgbPanel = lazy(() => import("./DrizzleRgbPanel"));

function ComposeTabInner() {
  const { doneFiles } = useFileContext();
  const { setRgbChannels } = useRgbContext();
  const { setRenderedPreviewUrl } = useRenderContext();
  const { narrowbandPalette } = useNarrowbandContext();
  const { composeRgb, drizzleStack, drizzleRgb } = useBackend();

  const [rgbResult, setRgbResult] = useState<any>(null);
  const [rgbLoading, setRgbLoading] = useState(false);

  const [drizzleResult, setDrizzleResult] = useState<any>(null);
  const [drizzleLoading, setDrizzleLoading] = useState(false);

  const [drizzleRgbResult, setDrizzleRgbResult] = useState<any>(null);
  const [drizzleRgbLoading, setDrizzleRgbLoading] = useState(false);
  const [drizzleRgbProgress, setDrizzleRgbProgress] = useState(0);
  const [drizzleRgbStage, setDrizzleRgbStage] = useState("");

  const handleComposeRgb = useCallback(
    async (rPath: string | null, gPath: string | null, bPath: string | null, options: any) => {
      setRgbLoading(true);
      try {
        const result = await composeRgb(rPath, gPath, bPath, "./output", options);
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
    [composeRgb, setRgbChannels, setRenderedPreviewUrl],
  );

  const handleDrizzle = useCallback(
    async (paths: string[], options: any) => {
      setDrizzleLoading(true);
      try {
        const result = await drizzleStack(paths, "./output", options);
        setDrizzleResult(result);
        if (result.previewUrl) {
          const bust = `${result.previewUrl}${result.previewUrl.includes("?") ? "&" : "?"}t=${Date.now()}`;
          setRenderedPreviewUrl(bust);
        }
      } catch (e) {
        console.error("Drizzle stack failed:", e);
      } finally {
        setDrizzleLoading(false);
      }
    },
    [drizzleStack, setRenderedPreviewUrl],
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
    [drizzleRgb, setRenderedPreviewUrl],
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

  return (
    <Suspense
      fallback={
        <div className="flex items-center justify-center py-12">
          <Loader2 size={20} className="animate-spin text-zinc-500" />
        </div>
      }
    >
      <div className="flex flex-col gap-4">
        <RgbComposePanel
          files={doneFiles}
          onCompose={handleComposeRgb}
          result={rgbResult}
          isLoading={rgbLoading}
          narrowbandPalette={narrowbandPalette}
        />
        <DrizzlePanel
          files={doneFiles}
          onDrizzle={(paths: string[], opts: any) => handleDrizzle(paths, opts)}
          result={drizzleResult}
          isLoading={drizzleLoading}
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
