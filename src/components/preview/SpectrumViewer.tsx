import { memo, useCallback, useMemo, useState } from "react";
import SpectrumPlot from "../analysis/SpectrumPlot";
import SpectrumViewControls from "../analysis/SpectrumViewControls";
import SpectrumViewerEmpty from "./SpectrumViewerEmpty";
import { useActiveSpectrumDocument } from "../../hooks/useActiveSpectrumDocument";
import type { Spectrum1dDoc } from "../../hooks/useSpectrum1dStore";
import { useSpectrumLineOverlay, useSpectrumView } from "../../hooks/useSpectrumViewStore";
import { lineClickSetsRest, type BandMark, type LineMark } from "../../utils/lineList";
import { SPECTRUM_VIEWER_OPEN_CUBE } from "../../utils/previewShell";
import { documentSummary } from "../../utils/spectrumDocument";
import { SPECTRUM_PLOT_HEIGHT, bandChannelRange, documentPlot } from "../../utils/spectrumPlotModel";
import type { ChannelRange } from "../../utils/spectrumRange";
import { fileBaseName } from "../../utils/values";

export interface SpectrumViewerProps {
  name: string;
  onOpenTool: () => void;
}

const PLOT_HEIGHT_BEFORE_MEASURE = 320;
const VIEWER_HINT =
  "Drag to select a line range, wheel to zoom, Shift and drag to pan, double-click to reset the zoom. The Spectra tool measures the selected line.";
const CURSOR_KEY_HINT =
  "Left and Right arrows move the channel cursor, Shift moves ten channels, Home and End go to the ends, Escape clears it.";
const NO_FLUX_MESSAGE = "This spectrum has no flux values to plot";

function documentMessage(doc: Spectrum1dDoc, shownName: string): string {
  if (doc.error !== null) return `This spectrum could not be read: ${doc.error}`;
  if (doc.spectrum === null) return `Reading ${shownName}`;
  return NO_FLUX_MESSAGE;
}

function SpectrumViewer({ name, onOpenTool }: SpectrumViewerProps) {
  const active = useActiveSpectrumDocument();
  const [view, patch] = useSpectrumView(active.viewKey);
  const lineOverlay = useSpectrumLineOverlay(active.viewKey);
  const spectrum = active.doc.spectrum;
  const plot = useMemo(() => (spectrum && spectrum.flux.length > 0 ? documentPlot(spectrum, view) : null), [spectrum, view]);
  const [measuredHeight, setMeasuredHeight] = useState<number | null>(null);
  const measurePlotArea = useCallback((area: HTMLDivElement | null) => {
    if (!area) return;
    const observer = new ResizeObserver(() => setMeasuredHeight(Math.floor(area.getBoundingClientRect().height)));
    observer.observe(area);
    return () => observer.disconnect();
  }, []);
  const brushRange = useCallback((range: ChannelRange) => patch({ range }), [patch]);
  const pickMark = useCallback(
    (mark: LineMark) => {
      if (plot && lineClickSetsRest(plot.axis)) patch({ restUm: mark.restVacuumUm, pickedLineId: mark.id });
    },
    [plot, patch],
  );
  const pickBand = useCallback(
    (band: BandMark) => {
      const range = plot ? bandChannelRange(band, plot.model) : null;
      if (range) patch({ range });
    },
    [plot, patch],
  );

  const viewKey = active.viewKey;
  if (active.path === null || viewKey === null) return <SpectrumViewerEmpty name={name} onOpenCubeTool={onOpenTool} />;
  const shownName = active.content === "spectrum" ? name : fileBaseName(active.path);
  const openTool = (
    <button
      type="button"
      onClick={onOpenTool}
      className="shrink-0 text-[11px] px-3 py-1 rounded text-zinc-200 hover:text-white transition-colors"
      style={{ border: "1px solid var(--ab-border)", background: "rgba(20,184,166,0.12)" }}
    >
      {SPECTRUM_VIEWER_OPEN_CUBE}
    </button>
  );

  if (!spectrum || !plot) {
    return (
      <div data-spectrum-viewer="" className="h-full flex flex-col">
        <div className="ab-viewer-empty">
          <div className="flex flex-col items-center gap-3 px-6 text-center max-w-md">
            <span className="text-xs text-zinc-300 break-words">{documentMessage(active.doc, shownName)}</span>
            {openTool}
          </div>
        </div>
      </div>
    );
  }

  return (
    <div data-spectrum-viewer="" className="h-full flex flex-col">
      <div className="px-3 py-2 flex items-center gap-3 min-w-0">
        <span className="text-xs text-zinc-300 truncate" title={active.path}>
          {shownName}
        </span>
        <span className="flex-1 min-w-0 text-[10px] font-mono text-zinc-500 truncate">{documentSummary(spectrum)}</span>
        {openTool}
      </div>
      <SpectrumViewControls viewKey={viewKey} spectrum={spectrum} model={plot.model} />
      <div ref={measurePlotArea} className="flex-1 min-h-0 px-3">
        <SpectrumPlot
          model={plot.model}
          range={view.range}
          windows={view.windows}
          marks={plot.marks}
          bands={plot.bands}
          pickedMarkId={plot.pickedMarkId}
          lineOverlay={lineOverlay}
          height={Math.max(SPECTRUM_PLOT_HEIGHT, measuredHeight ?? PLOT_HEIGHT_BEFORE_MEASURE)}
          cursorKeys
          doubleClick="reset"
          keyHint={CURSOR_KEY_HINT}
          onBrush={brushRange}
          onPickMark={pickMark}
          onPickBand={pickBand}
          onViewChange={patch}
        />
      </div>
      <p className="px-3 py-2 text-[10px] text-zinc-500">{VIEWER_HINT}</p>
    </div>
  );
}

export default memo(SpectrumViewer);
