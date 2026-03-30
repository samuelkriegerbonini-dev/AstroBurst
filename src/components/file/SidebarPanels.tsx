import { lazy, Suspense, memo, useRef } from "react";
import { Loader2 } from "lucide-react";
import { useFileContext, useHistContext } from "../../context/PreviewContext";
import { useMousePixel } from "../../hooks/useMousePixelStore";
import WcsReadout from "../header/WcsReadout";

const AnalysisTab = lazy(() => import("../analysis/AnalysisTab"));
const HeadersTab = lazy(() => import("../header/HeadersTab"));
const ExportTab = lazy(() => import("../export/ExportTab"));

const EMPTY_SPECTRUM: number[] = [];

function Spinner() {
  return (
    <div className="flex items-center justify-center py-12">
      <Loader2 size={18} className="animate-spin" style={{ color: "var(--ab-teal)" }} />
    </div>
  );
}

export const InfoPanel = memo(function InfoPanel() {
  const { file } = useFileContext();
  const { histData, stfParams } = useHistContext();
  const mousePixel = useMousePixel();
  if (!file) return <div className="px-3 py-4 text-[10px] text-zinc-600">No file selected</div>;
  return (
    <div className="flex flex-col gap-3 px-3 py-2 text-[10px] font-mono text-zinc-500">
      {file.path && file.result?.dimensions && (
        <WcsReadout
          filePath={file.path}
          imageWidth={file.result.dimensions[0]}
          imageHeight={file.result.dimensions[1]}
          mouseX={mousePixel?.x ?? null}
          mouseY={mousePixel?.y ?? null}
        />
      )}
      {file.result?.dimensions && (
        <div className="flex items-center gap-3 flex-wrap">
          <span className="text-zinc-400">{file.result.dimensions[0]}&times;{file.result.dimensions[1]}</span>
          {file.result.header?.BITPIX && <span>BITPIX {file.result.header.BITPIX}</span>}
          {file.result.elapsed_ms && <span>{(file.result.elapsed_ms / 1000).toFixed(2)}s</span>}
        </div>
      )}
      {histData && (
        <div className="flex flex-col gap-1">
          <div className="flex items-center gap-3 flex-wrap">
            <span>mean={histData.mean?.toFixed(2)}</span>
            <span>median={histData.median?.toFixed(2)}</span>
            <span>&sigma;={histData.sigma?.toFixed(2)}</span>
          </div>
          <div className="flex items-center gap-3 flex-wrap">
            <span style={{ color: "rgba(239,68,68,0.6)" }}>S={stfParams.shadow.toFixed(4)}</span>
            <span style={{ color: "rgba(245,158,11,0.6)" }}>M={stfParams.midtone.toFixed(4)}</span>
            <span style={{ color: "rgba(16,185,129,0.6)" }}>H={stfParams.highlight.toFixed(4)}</span>
          </div>
        </div>
      )}
      {file.result?.header && (
        <div className="flex flex-col gap-1 text-zinc-600">
          {file.result.header.TELESCOP && <span>TELESCOP: {file.result.header.TELESCOP}</span>}
          {file.result.header.INSTRUME && <span>INSTRUME: {file.result.header.INSTRUME}</span>}
          {file.result.header.FILTER && <span>FILTER: {file.result.header.FILTER}</span>}
          {file.result.header.EXPTIME && <span>EXPTIME: {file.result.header.EXPTIME}s</span>}
          {file.result.header["DATE-OBS"] && <span>DATE: {file.result.header["DATE-OBS"]}</span>}
          {file.result.header.OBJECT && <span>OBJECT: {file.result.header.OBJECT}</span>}
        </div>
      )}
    </div>
  );
});

function AnalysisWrapper() {
  const overlayRef = useRef<HTMLCanvasElement>(null);
  return (
    <AnalysisTab
      spectrum={EMPTY_SPECTRUM}
      specWavelengths={null}
      specCoord={null}
      specLoading={false}
      specElapsed={0}
      starOverlayRef={overlayRef}
    />
  );
}

export type LeftTabId = "files" | "info" | "analysis" | "headers" | "export";

interface SidebarPanelsProps {
  activeTab: LeftTabId;
}

export default function SidebarPanels({ activeTab }: SidebarPanelsProps) {
  return (
    <div className="flex-1 overflow-y-auto">
      <Suspense fallback={<Spinner />}>
        {activeTab === "info" && <InfoPanel />}
        {activeTab === "analysis" && <AnalysisWrapper />}
        {activeTab === "headers" && <HeadersTab />}
        {activeTab === "export" && <ExportTab />}
      </Suspense>
    </div>
  );
}
