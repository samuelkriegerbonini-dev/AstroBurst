import { memo } from "react";
import { Loader2 } from "lucide-react";
import { usePhotometryStatus } from "../../hooks/usePhotometryStatus";
import type { UserZeroPoint } from "../../services/analysis";
import { statusLineModel, type StatusTone } from "../../utils/photometryStatus";

interface PhotometryStatusLineProps {
  filePath: string | null;
  measureKey?: string | null;
  excludeDq: boolean;
  zeroPoint?: UserZeroPoint | null;
}

const LOADING_TEXT = "Reading calibration and layers...";
const ERROR_PREFIX = "Status not available: ";
const CHIP_CLASS = "text-[9px] px-1.5 py-0.5 rounded";
const TONE_CLASS: Record<StatusTone, string> = {
  ok: "text-emerald-300 bg-emerald-900/30",
  muted: "text-zinc-400 bg-zinc-800/60",
  warn: "text-amber-300 bg-amber-900/30",
};

function PhotometryStatusLine({ filePath, measureKey = null, excludeDq, zeroPoint }: PhotometryStatusLineProps) {
  const doc = usePhotometryStatus(filePath, excludeDq, measureKey, zeroPoint ?? null);
  if (filePath === null) return null;

  if (doc.error !== null) {
    return (
      <div data-photometry-status className="text-[10px] text-amber-400/90 bg-zinc-900/80 rounded px-2 py-1.5 break-words">
        {`${ERROR_PREFIX}${doc.error}`}
      </div>
    );
  }

  if (doc.status === null) {
    return (
      <div data-photometry-status className="flex items-center gap-1.5 text-[10px] text-zinc-500 bg-zinc-900/80 rounded px-2 py-1.5">
        <Loader2 size={10} className="animate-spin" />
        <span>{LOADING_TEXT}</span>
      </div>
    );
  }

  const model = statusLineModel(doc.status);
  return (
    <div data-photometry-status className="flex flex-col gap-1 text-[10px] bg-zinc-900/80 rounded px-2 py-1.5">
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="text-zinc-500">Calibration</span>
        <span data-status-calibration className={model.calibrated ? "text-zinc-300" : "text-amber-400/90"}>
          {model.calibration}
        </span>
      </div>
      <div className="flex flex-wrap items-center gap-1">
        {model.chips.map((chip) => (
          <span key={chip.key} data-status-chip={chip.key} title={chip.title ?? undefined} className={`${CHIP_CLASS} ${TONE_CLASS[chip.tone]}`}>
            {chip.text}
          </span>
        ))}
      </div>
      {model.warnings.length > 0 && (
        <div className="text-[9px] text-amber-300/90 bg-amber-900/15 border border-amber-800/30 rounded px-2 py-1 space-y-0.5">
          {model.warnings.map((warning, i) => (
            <div key={i} data-status-warning>
              {warning}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export default memo(PhotometryStatusLine);
