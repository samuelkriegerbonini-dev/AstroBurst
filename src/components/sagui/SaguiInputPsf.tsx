import { Fragment, useEffect, useRef, useState } from "react";
import { WarningList } from "../ui";
import { useResolutionSettings } from "../../hooks/useResolutionSettings";
import { resolutionCheck } from "../../services/resolution";
import type { ResolutionCheck } from "../../shared/types/resolution";
import type { SaguiInputInfo } from "../../shared/types/sagui";
import { resolutionRows } from "../../utils/resolutionText";
import { bandName, bandPsfLines, cubeRedshift, physicalRequest, regionBudgetText, type RegionBudgetText } from "./saguiInputPsfModel";

export interface SaguiInputPsfProps {
  input: SaguiInputInfo;
  psfTarget: string;
  nRegions: number;
}

export interface SaguiInputPsfViewProps extends SaguiInputPsfProps {
  check: ResolutionCheck | null;
  error: string | null;
}

const TONE: Record<RegionBudgetText["tone"], string> = { ok: "text-zinc-400", warn: "text-amber-300", muted: "text-zinc-500" };
const NO_REDSHIFT_CARD = "The cube has no REDSHIFT card: the PSF is given in arcsec only. Cube Prep writes the card in Edit, Set centre / redshift.";

export function SaguiInputPsfView({ input, psfTarget, nRegions, check, error }: SaguiInputPsfViewProps): React.JSX.Element {
  const lines = bandPsfLines(input);
  const budget = regionBudgetText(input, psfTarget, nRegions);
  const redshift = cubeRedshift(input);
  return (
    <div data-sagui-psf className="space-y-1 text-[10px]">
      {lines === null ? (
        <div className="text-zinc-400">{input.bands.map(bandName).join(" · ")}</div>
      ) : (
        lines.map((line, index) => (
          <div key={index} className="text-zinc-400">
            {line}
          </div>
        ))
      )}
      {budget !== null && (
        <p data-region-budget={budget.tone} className={TONE[budget.tone]}>
          {budget.text}
        </p>
      )}
      {redshift !== null ? (
        <p className="text-zinc-400">{`Redshift ${redshift} from the REDSHIFT card of the cube.`}</p>
      ) : (
        lines !== null && <p className="text-zinc-500">{NO_REDSHIFT_CARD}</p>
      )}
      {check !== null && (
        <>
          <div className="grid grid-cols-[auto_1fr] gap-x-2">
            {resolutionRows(check).map(([label, value]) => (
              <Fragment key={label}>
                <span className="text-zinc-500">{label}</span>
                <span className="font-mono text-zinc-200 break-words">{value}</span>
              </Fragment>
            ))}
          </div>
          <WarningList warnings={check.warnings} />
        </>
      )}
      {error !== null && <p className="text-zinc-500">{`Physical scale not available: ${error}`}</p>}
    </div>
  );
}

interface KeyedAnswer {
  key: string;
  check: ResolutionCheck | null;
  error: string | null;
}

export default function SaguiInputPsf({ input, psfTarget, nRegions }: SaguiInputPsfProps): React.JSX.Element {
  const settings = useResolutionSettings();
  const request = physicalRequest(input, psfTarget, settings);
  const asked = request === null ? null : { request, key: JSON.stringify(request) };
  const key = asked?.key ?? null;
  const [answer, setAnswer] = useState<KeyedAnswer | null>(null);
  const askedRef = useRef(asked);
  askedRef.current = asked;

  useEffect(() => {
    const pending = askedRef.current;
    if (pending === null) return;
    let current = true;
    resolutionCheck(pending.request).then(
      (check) => {
        if (current) setAnswer({ key: pending.key, check, error: null });
      },
      (reason: unknown) => {
        if (current) setAnswer({ key: pending.key, check: null, error: reason instanceof Error ? reason.message : String(reason) });
      },
    );
    return () => {
      current = false;
    };
  }, [key]);

  const shown = answer !== null && answer.key === key ? answer : null;
  return <SaguiInputPsfView input={input} psfTarget={psfTarget} nRegions={nRegions} check={shown?.check ?? null} error={shown?.error ?? null} />;
}
