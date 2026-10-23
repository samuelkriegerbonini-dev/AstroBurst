import { useState, useCallback, useMemo, useRef, lazy, Suspense, memo } from "react";
import { FileText, X } from "lucide-react";
import { getPreviewUrl } from "../../infrastructure/tauri";
import { fileKeyOf, useFileContext, useCubeContext, useRenderActions } from "../../context/PreviewContext";
import { ANALYSIS_SECTION, CUBE_TOOL_EMPTY_TITLE } from "../../utils/analysisSections";
import { frameRecordLabel } from "../../utils/cubeNavigation";
import { cubePanelGates, formatRampFrameLabel } from "../../utils/rampLabels";
import { useSpectrum } from "../../hooks/useSpectrumStore";
import { openSpectrumCsv, spectrumCsvStore, useOpenSpectrumCsv, useSpectrum1dDoc } from "../../hooks/useSpectrum1dStore";
import { OPEN_SPECTRUM_CSV_LABEL, cubeToolContent, type SpectrumDocument } from "../../utils/spectrumDocument";
import { fileBaseName } from "../../utils/spectrumExport";
import type { CubeResult } from "./SpectroscopyPanel";
import TabSpinner from "./TabSpinner";
import CubeSampleOffers from "./CubeSampleOffers";

const SpectroscopyPanel = lazy(() => import("./SpectroscopyPanel"));
const PvPanel = lazy(() => import("./PvPanel"));
const RampPanel = lazy(() => import("./RampPanel"));

function CubeTool() {
  const { file } = useFileContext();
  const { isCube, isSpectrum, cubeDims, ramp, rampSource } = useCubeContext();
  const { publishProcessed } = useRenderActions();
  const spec = useSpectrum();
  const csv = useOpenSpectrumCsv();

  const filePath = file?.path;
  const fileKey = fileKeyOf(file);
  const content = cubeToolContent({ isCube, isRamp: ramp !== null, isSpectrum, fileKey, csv });
  const docPath = content === "spectrum" ? (filePath ?? null) : content === "csv" && csv ? csv.path : null;
  const docHdu = content === "spectrum" ? (file?.spectrum?.hdu ?? null) : null;
  const doc = useSpectrum1dDoc(docPath, docHdu);
  const docSpectrum = doc.spectrum;
  const spectrumDocument = useMemo<SpectrumDocument | null>(
    () => (docSpectrum ? { source: { kind: "file", hdu: docHdu }, spectrum: docSpectrum } : null),
    [docSpectrum, docHdu],
  );
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
        {content === "empty" && (
          <div className="px-3 py-2 flex flex-col items-start gap-2 text-[10px] text-zinc-500">
            <span data-cube-empty>{CUBE_TOOL_EMPTY_TITLE}</span>
            <button
              data-open-spectrum-csv
              onClick={() => void openSpectrumCsv(fileKey)}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[10px] font-medium transition-all"
              style={{
                background: "color-mix(in srgb, var(--ab-violet) 8%, transparent)",
                border: "1px solid color-mix(in srgb, var(--ab-violet) 20%, transparent)",
                color: "var(--ab-violet)",
              }}
            >
              <FileText size={10} />
              {OPEN_SPECTRUM_CSV_LABEL}
            </button>
          </div>
        )}
        <CubeSampleOffers idle={content === "empty"} />

        {docPath !== null && (
          <section id={ANALYSIS_SECTION.spectrum.id} className="flex flex-col gap-2">
            {content === "csv" && (
              <div data-spectrum-csv className="px-1 flex items-center gap-2 text-[10px] font-mono text-zinc-500">
                <FileText size={10} />
                <span className="truncate" title={docPath}>
                  {fileBaseName(docPath)}
                </span>
                <button
                  onClick={() => spectrumCsvStore.close()}
                  className="ml-auto flex items-center gap-1 text-zinc-500 hover:text-zinc-300"
                  title="Close the CSV spectrum"
                >
                  <X size={10} />
                  Close
                </button>
              </div>
            )}
            <SpectroscopyPanel
              key={`${content}|${docHdu ?? ""}|${docPath}`}
              documentMode
              spectrumDocument={spectrumDocument}
              isLoading={docSpectrum === null && doc.error === null}
              error={doc.error}
              filePath={docPath}
              publishGate={cubeGates.spectrum}
            />
          </section>
        )}

        {content === "cube" && ramp && (
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

        {content === "cube" && isCube && (
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
        {content === "cube" && isCube && !ramp && (
          <section id={ANALYSIS_SECTION.pv.id}>
            <PvPanel filePath={filePath} fileKey={fileKey} cubeDims={cubeDims} />
          </section>
        )}
      </div>
    </Suspense>
  );
}

export default memo(CubeTool);
