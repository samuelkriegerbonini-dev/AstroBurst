import { useState, useCallback, useRef, lazy, Suspense, memo } from "react";
import { getPreviewUrl } from "../../infrastructure/tauri";
import { fileKeyOf, useFileContext, useCubeContext, useRenderActions } from "../../context/PreviewContext";
import { ANALYSIS_SECTION, CUBE_TOOL_EMPTY_TITLE } from "../../utils/analysisSections";
import { frameRecordLabel } from "../../utils/cubeNavigation";
import { cubePanelGates, formatRampFrameLabel } from "../../utils/rampLabels";
import { useSpectrum } from "../../hooks/useSpectrumStore";
import type { CubeResult } from "./SpectroscopyPanel";
import TabSpinner from "./TabSpinner";

const SpectroscopyPanel = lazy(() => import("./SpectroscopyPanel"));
const PvPanel = lazy(() => import("./PvPanel"));
const RampPanel = lazy(() => import("./RampPanel"));

function CubeTool() {
  const { file } = useFileContext();
  const { isCube, cubeDims, ramp, rampSource } = useCubeContext();
  const { publishProcessed } = useRenderActions();
  const spec = useSpectrum();

  const filePath = file?.path;
  const fileKey = fileKeyOf(file);
  const [cubeGates] = useState(cubePanelGates);
  const publishCube = useCallback(
    (result: CubeResult) => {
      if (!fileKey || !filePath) return;
      publishProcessed(fileKey, {
        previewUrl: result.previewUrl,
        fitsPath: result.fitsPath,
        dimensions: result.dimensions,
        label: result.label,
        kind: "cube",
        inputPath: filePath,
        frameIndex: result.frameIndex,
        displayHint: result.displayHint,
      });
    },
    [publishProcessed, fileKey, filePath],
  );

  const frameSeqRef = useRef(0);
  const handleFramePreview = useCallback(
    async (outputPath: string, frameIndex: number, fitsPath?: string) => {
      const seq = ++frameSeqRef.current;
      try {
        const url = await getPreviewUrl(outputPath);
        if (frameSeqRef.current !== seq) return;
        const label = ramp
          ? formatRampFrameLabel(frameIndex, ramp)
          : frameRecordLabel(frameIndex, cubeDims?.frames ?? 0, cubeDims?.spectral_axis ?? null);
        const dimensions: [number, number] | null = fitsPath && cubeDims ? [cubeDims.width, cubeDims.height] : null;
        publishCube({ label, previewUrl: url, fitsPath: fitsPath ?? null, dimensions, frameIndex });
      } catch (e) {
        console.error("Frame preview failed:", e);
      }
    },
    [publishCube, cubeDims, ramp],
  );

  return (
    <Suspense fallback={<TabSpinner />}>
      <div className="flex flex-col gap-3 p-3">
        {!ramp && !isCube && (
          <div data-cube-empty className="px-3 py-2 text-[10px] text-zinc-500">
            {CUBE_TOOL_EMPTY_TITLE}
          </div>
        )}

        {ramp && (
          <section id={ANALYSIS_SECTION.ramp.id}>
            <RampPanel
              key={filePath ?? ""}
              filePath={filePath}
              ramp={ramp}
              rampSource={rampSource}
              cubeDims={cubeDims}
              onCubeResult={publishCube}
              publishGate={cubeGates.ramp}
            />
          </section>
        )}

        {isCube && (
          <section id={ANALYSIS_SECTION.spectrum.id}>
            <SpectroscopyPanel
              spectrum={spec.spectrum}
              wavelengths={spec.wavelengths}
              pixelCoord={spec.coord}
              isLoading={spec.loading}
              cubeDims={cubeDims}
              elapsed={spec.elapsed}
              error={spec.error}
              filePath={filePath}
              onCubeResult={publishCube}
              onFramePreview={handleFramePreview}
              publishGate={cubeGates.spectrum}
            />
          </section>
        )}
        {isCube && (
          <section id={ANALYSIS_SECTION.pv.id}>
            <PvPanel filePath={filePath} fileKey={fileKey} cubeDims={cubeDims} />
          </section>
        )}
      </div>
    </Suspense>
  );
}

export default memo(CubeTool);
