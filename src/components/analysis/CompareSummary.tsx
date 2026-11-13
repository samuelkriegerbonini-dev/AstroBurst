import { memo, type ReactNode } from "react";
import { apertureUsedText, compareHeadline, rejectedText, type CompareRun } from "../../utils/catalogCompare";
import { MISSING_VALUE } from "../../utils/formatUncertainty";
import { WarningList } from "../ui";

interface CompareSummaryProps {
  run: CompareRun;
}

const CARD_CLASS = "bg-zinc-900/80 rounded px-2 py-1.5";
const VALUE_CLASS = "text-zinc-300 font-mono break-words";
const RATIO_CLASS = "text-amber-300 font-mono break-words";
const NOTE_CLASS = "text-[9px] text-zinc-500";

const RATIO_TITLE = "Median over the rows used of the flux measured on the image over the catalogue flux, both in Jy";
const ERROR_RATIO_TITLE = "Median of the formal error of the app over the catalogue error; the notes state the two conventions";
const ZERO_POINT_TITLE =
  "AB zero point of one image unit that makes the measured fluxes equal the catalogue fluxes; it does not use the calibration of the image";
const NO_CALIBRATION = "none: the image has no flux calibration in Jy, so no ratio";
const NOTHING_LEFT_OUT = "none";

function card(key: string, label: string, lines: ReactNode, title?: string): ReactNode {
  return (
    <div className={CARD_CLASS} data-card={key} title={title}>
      <div className="text-zinc-500">{label}</div>
      {lines}
    </div>
  );
}

function valueLine(text: string, className = VALUE_CLASS): ReactNode {
  return <div className={className}>{text}</div>;
}

function CompareSummary({ run }: CompareSummaryProps) {
  const report = run.report;
  const h = compareHeadline(report);
  const presetText = report.preset === null ? "custom columns" : `preset ${report.preset}${run.presetEdited ? " (edited)" : ""}`;
  const filter = report.band.filter;
  const filterText = filter === null ? "" : `, filter ${filter.dataset} ${filter.filter}`;
  const rejectedLines = report.rejected.length === 0 ? [NOTHING_LEFT_OUT] : report.rejected.map(rejectedText);

  return (
    <div className="space-y-1.5 text-[10px]" data-compare-summary>
      {card(
        "ratio",
        "Measured / catalogue",
        <>
          {valueLine(h.ratio, RATIO_CLASS)}
          {h.offset !== null && valueLine(h.offset)}
        </>,
        RATIO_TITLE,
      )}
      {card("rows", "Rows", valueLine(h.counts))}
      {card("errors", "Formal error / catalogue error", valueLine(h.errRatio ?? MISSING_VALUE), ERROR_RATIO_TITLE)}
      {card("zero-point", "Zero point implied by the catalogue", valueLine(h.zeroPoint ?? MISSING_VALUE), ZERO_POINT_TITLE)}
      {card("aperture", "Aperture", valueLine(apertureUsedText(report.aperture)))}
      {card(
        "compared",
        "Compared",
        <>
          {valueLine(`${report.table.file_name}, ${presetText}, catalogue band ${report.catalogue_band ?? MISSING_VALUE}`)}
          {valueLine(`${report.band.file_name}${filterText}`)}
        </>,
      )}
      {card("calibration", "Calibration of the image", valueLine(report.calibration_origin ?? NO_CALIBRATION))}
      {card(
        "rejected",
        "Left out",
        rejectedLines.map((line) => (
          <div key={line} className={VALUE_CLASS}>
            {line}
          </div>
        )),
      )}
      <WarningList warnings={report.warnings} />
      {report.notes.length > 0 && (
        <details data-compare-notes>
          <summary className="text-[9px] text-zinc-500 cursor-pointer">{`notes (${report.notes.length})`}</summary>
          {report.notes.map((note) => (
            <div key={note} className={NOTE_CLASS}>
              {note}
            </div>
          ))}
        </details>
      )}
    </div>
  );
}

export default memo(CompareSummary);
