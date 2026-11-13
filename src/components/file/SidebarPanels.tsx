import { memo } from "react";
import { Info } from "lucide-react";
import { useFileContext, useHistContext, useRenderContext } from "../../context/PreviewContext";
import { useMousePixel } from "../../hooks/useMousePixelStore";
import { useAnalysisTarget, useMeasurementSource } from "../../hooks/useAnalysisTarget";
import WcsReadout from "../header/WcsReadout";
import PixelReadout from "../header/PixelReadout";
import MeasurementBadge from "../analysis/MeasurementBadge";
import { SIGMA_MAD_LABEL, SIGMA_MAD_TITLE } from "../../utils/analysisLabels";
import { wcsProbeTarget } from "../../utils/analysisTarget";
import { colorbarUnit } from "../../utils/colorbar";
import { formatSignificant } from "../../utils/formatSignificant";
import { CUBE_FRAME_VALUE_PENDING, pointerProbePath } from "../../utils/previewShell";
import "./info-panel.css";

const OBSERVATION_FIELDS = [
  ["OBJECT", "Object"],
  ["TELESCOP", "Telescope"],
  ["INSTRUME", "Instrument"],
  ["FILTER", "Filter"],
  ["PUPIL", "Pupil"],
  ["EXPTIME", "Exposure"],
  ["DATE-OBS", "Observed"],
] as const;

function formatStat(value: number | undefined, digits: number): string {
  return typeof value === "number" && Number.isFinite(value) ? value.toFixed(digits) : "—";
}

function formatDataStat(value: number | undefined, unit: string | null): string {
  if (typeof value !== "number" || !Number.isFinite(value)) return "—";
  const text = value !== 0 && Math.abs(value) < 1e-3 ? value.toExponential(3) : formatSignificant(value);
  return unit === null ? text : `${text} ${unit}`;
}

export const InfoPanel = memo(function InfoPanel() {
  const { file } = useFileContext();
  const { histData, histDataPath, stfParams } = useHistContext();
  const { processed } = useRenderContext();
  const mousePixel = useMousePixel();
  const target = useAnalysisTarget();
  const probePath = target.path;
  const pixelProbePath = pointerProbePath(probePath, processed);
  const valueSource = useMeasurementSource(false);
  if (!file) return (
    <section className="ab-info-panel">
      <header className="ab-info-heading"><Info size={15} aria-hidden="true" /><h2>File info</h2></header>
      <div className="ab-info-empty"><strong>No file selected</strong><p>Select a file to inspect its details.</p></div>
    </section>
  );
  const wcsProbe = probePath
    ? wcsProbeTarget({
        path: probePath,
        dimensions: target.dimensions,
        previewOnly: target.displayed.previewOnly,
        fileDims: file.result?.dimensions ?? null,
      })
    : null;
  const result = file.result;
  const header = result?.header;
  const statUnit = colorbarUnit(header?.BUNIT, histDataPath !== file.path);
  const observation = OBSERVATION_FIELDS.filter(([key]) => header?.[key]);
  return (
    <section className="ab-info-panel">
      <header className="ab-info-header">
        <div className="ab-info-heading"><Info size={15} aria-hidden="true" /><h2>File info</h2></div>
        <p className="ab-info-filename" title={file.name}>{file.name}</p>
      </header>
      <div className="ab-info-body">
        {result && (
          <div className="ab-info-section">
            <h3 className="sr-only">File details</h3>
            <dl className="ab-info-fields">
              {result.dimensions && <div><dt>Dimensions</dt><dd>{result.dimensions.join(" × ")}</dd></div>}
              {header?.BITPIX && <div><dt title="FITS BITPIX">BITPIX</dt><dd>{header.BITPIX}</dd></div>}
              {header?.BUNIT && <div><dt title="FITS BUNIT">Data unit</dt><dd>{header.BUNIT}</dd></div>}
              {typeof result.elapsed_ms === "number" && Number.isFinite(result.elapsed_ms) && <div><dt>Load time</dt><dd>{(result.elapsed_ms / 1000).toFixed(2)} s</dd></div>}
            </dl>
          </div>
        )}
        {observation.length > 0 && (
          <div className="ab-info-section">
            <h3>Observation</h3>
            <dl className="ab-info-fields">
              {observation.map(([key, label]) => <div key={key}><dt title={key}>{label}</dt><dd>{header?.[key]}{key === "EXPTIME" ? " s" : ""}</dd></div>)}
            </dl>
          </div>
        )}
        {histData && (
          <div className="ab-info-section">
            <h3>Image statistics</h3>
            <dl className="ab-info-metrics">
              <div><dt>Mean</dt><dd>{formatDataStat(histData.mean, statUnit)}</dd></div>
              <div><dt>Median</dt><dd>{formatDataStat(histData.median, statUnit)}</dd></div>
              <div title={SIGMA_MAD_TITLE}><dt>{SIGMA_MAD_LABEL}</dt><dd>{formatDataStat(histData.sigma, statUnit)}</dd></div>
            </dl>
            {histData.masked && <p className="ab-info-note" title="Pixels flagged by the DQ exclusion mask were ignored">DQ-masked · {histData.dq_excluded ?? 0} excluded</p>}
          </div>
        )}
        {histData && (
          <div className="ab-info-section">
            <h3 title="Screen transfer function">Display transfer <span>STF</span></h3>
            <dl className="ab-info-fields ab-info-stf">
              <div data-tone="shadow"><dt title="S: shadow clipping point">Shadows</dt><dd>{formatStat(stfParams.shadow, 4)}</dd></div>
              <div data-tone="midtone"><dt title="M: midtone balance">Midtones</dt><dd>{formatStat(stfParams.midtone, 4)}</dd></div>
              <div data-tone="highlight"><dt title="H: highlight clipping point">Highlights</dt><dd>{formatStat(stfParams.highlight, 4)}</dd></div>
            </dl>
          </div>
        )}
        {probePath && (
          <div className="ab-info-section ab-info-live">
            <h3>Live readout</h3>
            {valueSource && <div className="ab-info-source"><span>Values from</span><MeasurementBadge /></div>}
            {wcsProbe && (
              <WcsReadout
                filePath={wcsProbe.path}
                imageWidth={wcsProbe.dimensions[0]}
                imageHeight={wcsProbe.dimensions[1]}
                mouseX={mousePixel?.x ?? null}
                mouseY={mousePixel?.y ?? null}
              />
            )}
            <PixelReadout
              filePath={pixelProbePath}
              mouseX={mousePixel?.x ?? null}
              mouseY={mousePixel?.y ?? null}
            />
            {!mousePixel && <p className="ab-info-note">Hover the image for pixel values.</p>}
            {mousePixel && pixelProbePath === null && <p className="ab-info-note">{CUBE_FRAME_VALUE_PENDING}</p>}
          </div>
        )}
        {!result && <p className="ab-info-note">No image details available yet.</p>}
      </div>
    </section>
  );
});
