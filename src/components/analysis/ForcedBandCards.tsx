import { Fragment, memo } from "react";
import { WarningList } from "../ui";
import type { ForcedPhotometryResult } from "../../shared/types/forcedPhotometry";
import { bandCardRows } from "./forcedBandCardsModel";

export interface ForcedBandCardsProps {
  result: ForcedPhotometryResult;
  currentPath: string | null;
}

const SECTION_CLASS = "text-[9px] text-zinc-500 uppercase tracking-wider";
const CARD_CLASS = "rounded border border-zinc-800/60 bg-zinc-900/40 px-2 py-1.5 space-y-1.5";
const LABEL_CLASS = "text-[9px] text-zinc-500";
const VALUE_CLASS = "text-[10px] font-mono text-zinc-200 break-words";
const CHIP_CLASS = "text-[9px] px-1.5 py-0.5 rounded bg-cyan-500/10 border border-cyan-500/30 text-cyan-300";

function ForcedBandCards({ result, currentPath }: ForcedBandCardsProps) {
  const nSources = result.sources.length;
  return (
    <div className="space-y-1.5">
      <div className={SECTION_CLASS}>Bands</div>
      {result.bands.map((band) => (
        <div
          key={band.index}
          data-forced-band-card={band.index}
          data-band-state={band.error ? "failed" : "measured"}
          className={CARD_CLASS}
        >
          <div className="flex items-center gap-2">
            <span className="text-[11px] font-semibold font-mono text-zinc-200 break-all">{band.label}</span>
            {band.path === currentPath && (
              <span data-on-screen className={CHIP_CLASS}>
                on screen
              </span>
            )}
          </div>
          {band.error ? (
            <div data-band-error role="alert" className="text-[10px] text-red-300 break-words">
              {`Not measured: ${band.error}`}
            </div>
          ) : (
            <>
              <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 items-baseline">
                {bandCardRows(band, nSources).map(([label, value]) => (
                  <Fragment key={label}>
                    <dt className={LABEL_CLASS}>{label}</dt>
                    <dd className={VALUE_CLASS} title={value}>
                      {value}
                    </dd>
                  </Fragment>
                ))}
              </dl>
              <WarningList warnings={band.warnings} />
            </>
          )}
        </div>
      ))}
    </div>
  );
}

export default memo(ForcedBandCards);
