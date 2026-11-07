import { useState, useCallback, useLayoutEffect, useMemo, useRef, lazy, Suspense, memo } from "react";
import { FileText, X } from "lucide-react";
import { getPreviewUrl } from "../../infrastructure/tauri";
import { fileKeyOf, getRenderRecord, useFileContext, useCubeContext, useRenderActions } from "../../context/PreviewContext";
import { ANALYSIS_SECTION, CUBE_TOOL_EMPTY_TITLE, toolSections } from "../../utils/analysisSections";
import { filterFrameLabel, frameRecordLabel } from "../../utils/cubeNavigation";
import { cubePanelGates, formatRampFrameLabel } from "../../utils/rampLabels";
import { useSpectrum } from "../../hooks/useSpectrumStore";
import { openSpectrumCsv, spectrumCsvStore, useOpenSpectrumCsv, useSpectrum1dDoc } from "../../hooks/useSpectrum1dStore";
import { OPEN_SPECTRUM_CSV_LABEL, cubeToolContent, type SpectrumDocument } from "../../utils/spectrumDocument";
import { fileBaseName } from "../../utils/spectrumExport";
import { scientificInputRevisionOf } from "../../utils/pixelSource";
import type { CubeResult } from "./SpectroscopyPanel";
import TabSpinner from "./TabSpinner";
import CubeSampleOffers from "./CubeSampleOffers";
import SedPanel from "./SedPanel";
import SectionChips, { SectionTipNote } from "./SectionChips";
import { useSectionTip } from "./useSectionTip";

const SpectroscopyPanel = lazy(() => import("./SpectroscopyPanel"));
const PvPanel = lazy(() => import("./PvPanel"));
const RampPanel = lazy(() => import("./RampPanel"));

const LINE_TIP_TITLE = "Measure a spectral line";
const LINE_STEPS = "Drag across the plot to select a line range, set continuum windows A and B beside the line, choose a model, then click Measure line.";
const CUBE_SECTION_TIPS = {
  [ANALYSIS_SECTION.spectrum.id]: {
    title: LINE_TIP_TITLE,
    body: `Select Crosshair in the viewer and click the image to extract a spectrum. ${LINE_STEPS}`,
  },
};
const DOCUMENT_SECTION_TIPS = {
  [ANALYSIS_SECTION.spectrum.id]: { title: LINE_TIP_TITLE, body: LINE_STEPS },
};

function CubeTool() {
  const { file } = useFileContext();
  const { isCube, isSpectrum, cubeDims, ramp, rampSource } = useCubeContext();
  const { currentFileKey, publishProcessed, resetProcessed } = useRenderActions();
  const spec = useSpectrum();
  const csv = useOpenSpectrumCsv();

  const filePath = file?.path;
  const fileKey = fileKeyOf(file);
  const panelKey = scientificInputRevisionOf(file);
  const panelKeyRef = useRef(panelKey);
  panelKeyRef.current = panelKey;
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
  const frameSeqRef = useRef(0);
  useLayoutEffect(() => {
    const frameSeq = frameSeqRef;
    if (fileKey && currentFileKey() === fileKey) {
      const processed = getRenderRecord(fileKey)?.processed;
      if (processed?.kind === "cube" && processed.inputRevision !== panelKey) resetProcessed();
    }
    return () => { frameSeq.current++; };
  }, [fileKey, panelKey, currentFileKey, resetProcessed]);
  const publishCube = useCallback(
    (result: CubeResult) => {
      if (!fileKey || !filePath || currentFileKey() !== fileKey || panelKeyRef.current !== panelKey) return;
      frameSeqRef.current++;
      publishProcessed(fileKey, {
        previewUrl: result.previewUrl,
        fitsPath: result.fitsPath,
        dimensions: result.dimensions,
        label: result.label,
        kind: "cube",
        inputPath: filePath,
        inputRevision: panelKey,
        frameIndex: result.frameIndex,
        displayHint: result.displayHint,
      });
    },
    [currentFileKey, publishProcessed, fileKey, filePath, panelKey],
  );
  const invalidateCubeResult = useCallback(
    (fitsPaths: string[]) => {
      if (!fileKey || currentFileKey() !== fileKey) return;
      const processed = getRenderRecord(fileKey)?.processed;
      if (processed?.kind !== "cube" || processed.fitsPath === null || !fitsPaths.includes(processed.fitsPath)) return;
      frameSeqRef.current++;
      resetProcessed();
    },
    [currentFileKey, fileKey, resetProcessed],
  );

  const photometricPlanes = cubeDims?.photometric_planes?.length ? cubeDims.photometric_planes : null;

  const handleFramePreview = useCallback(
    async (outputPath: string, frameIndex: number, fitsPath?: string) => {
      const seq = ++frameSeqRef.current;
      try {
        const url = await getPreviewUrl(outputPath);
        if (frameSeqRef.current !== seq || currentFileKey() !== fileKey || !cubeGates.spectrum.isCurrent()) return;
        const frames = cubeDims?.frames ?? 0;
        const label = ramp
          ? formatRampFrameLabel(frameIndex, ramp)
          : (filterFrameLabel(frameIndex, frames, cubeDims?.photometric_planes) ??
            frameRecordLabel(frameIndex, frames, cubeDims?.spectral_axis ?? null));
        const dimensions: [number, number] | null = fitsPath && cubeDims ? [cubeDims.width, cubeDims.height] : null;
        publishCube({ label, previewUrl: url, fitsPath: fitsPath ?? null, dimensions, frameIndex });
      } catch (e) {
        console.error("Frame preview failed:", e);
      }
    },
    [publishCube, cubeDims, ramp, currentFileKey, fileKey, cubeGates],
  );

  const sections = toolSections("cube", {
    hasHistogram: false,
    showFft: false,
    showDeepZoom: false,
    isCube: content === "cube" && isCube,
    isRamp: content === "cube" && ramp !== null,
    hasSpectrumDocument: docPath !== null,
  });
  const sectionTip = useSectionTip(sections, content === "cube" ? (ramp ? undefined : CUBE_SECTION_TIPS) : DOCUMENT_SECTION_TIPS);

  return (
    <Suspense fallback={<TabSpinner />}>
      <div className="flex flex-col gap-3 p-3">
        <SectionChips label="Cube" sections={sections} tip={sectionTip} />
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
          <section id={ANALYSIS_SECTION.spectrum.id}>
            <SectionTipNote tip={sectionTip} sectionId={ANALYSIS_SECTION.spectrum.id} />
            <div className="flex flex-col gap-2">
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
                key={`${panelKey}|${content}|${docHdu ?? ""}|${docPath}`}
                documentMode
                spectrumDocument={spectrumDocument}
                isLoading={docSpectrum === null && doc.error === null}
                error={doc.error}
                filePath={docPath}
                publishGate={cubeGates.spectrum}
              />
            </div>
          </section>
        )}

        {content === "cube" && ramp && (
          <section id={ANALYSIS_SECTION.ramp.id}>
            <RampPanel
              key={panelKey}
              filePath={filePath}
              ramp={ramp}
              rampSource={rampSource}
              cubeDims={cubeDims}
              onCubeResult={publishCube}
              onInvalidateCubeResult={invalidateCubeResult}
              publishGate={cubeGates.ramp}
            />
          </section>
        )}

        {content === "cube" && isCube && (
          <section id={ANALYSIS_SECTION.spectrum.id}>
            <SectionTipNote tip={sectionTip} sectionId={ANALYSIS_SECTION.spectrum.id} />
            <SpectroscopyPanel
              key={panelKey}
              spectrum={spec.spectrum}
              wavelengths={spec.wavelengths}
              pixelCoord={spec.coord}
              isLoading={spec.loading}
              cubeDims={cubeDims}
              elapsed={spec.elapsed}
              error={spec.error}
              filePath={filePath}
              onCubeResult={publishCube}
              onInvalidateCubeResult={invalidateCubeResult}
              onFramePreview={handleFramePreview}
              publishGate={cubeGates.spectrum}
            />
          </section>
        )}
        {content === "cube" && isCube && photometricPlanes && (
          <SedPanel planes={photometricPlanes} bunit={cubeDims?.bunit ?? null} source={spec} />
        )}
        {content === "cube" && isCube && !ramp && (
          <section id={ANALYSIS_SECTION.pv.id}>
            <PvPanel key={panelKey} filePath={filePath} fileKey={fileKey} inputRevision={panelKey} cubeDims={cubeDims} />
          </section>
        )}
      </div>
    </Suspense>
  );
}

export default memo(CubeTool);
