import { lazy } from "react";
import { useStarOverlayContext } from "../../../context/PreviewContext";
import { useSpectrum } from "../../../hooks/useSpectrumStore";

const AnalysisTab = lazy(() => import("../../analysis/AnalysisTab"));

export default function AnalysisTool() {
  const { starOverlayRef } = useStarOverlayContext();
  const spec = useSpectrum();
  return (
    <AnalysisTab
      spectrum={spec.spectrum}
      specWavelengths={spec.wavelengths}
      specCoord={spec.coord}
      specLoading={spec.loading}
      specElapsed={spec.elapsed}
      specError={spec.error}
      starOverlayRef={starOverlayRef}
    />
  );
}
