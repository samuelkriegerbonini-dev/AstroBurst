import { Fragment, memo, useEffect, useRef, useState } from "react";
import { ErrorAlert, SMALL_BUTTON_CLASS, WarningList } from "../ui";
import { useMeasurementProvenance } from "../../hooks/useMeasurementLog";
import { imagePsfStore, useHeaderSeeing, useImagePsf, useResolutionSettings, useWcsPixelScale } from "../../hooks/useResolutionSettings";
import { resolutionCheck } from "../../services/resolution";
import type { ResolutionCheck, ResolutionVerdict } from "../../shared/types/resolution";
import { measurementLog, positiveOrNull } from "../../utils/measurementLog";
import { resolutionCheckEntry } from "../../utils/measurementLogQa";
import {
  CONTEXT_NOTE,
  NO_VERDICT_TEXT,
  VERDICT_LABEL,
  VERDICT_SCIENCE,
  psfChoice,
  resolutionRequest,
  resolutionRows,
  type ResolutionContext,
} from "../../utils/resolutionText";

export interface ResolutionCardProps {
  filePath: string | null;
  fwhmArcsec?: number | null;
  halfLightRadiusArcsec?: number | null;
  snr?: number | null;
  context: ResolutionContext;
  sourceLabel?: string;
}

export interface ResolutionCardViewProps {
  check: ResolutionCheck | null;
  missing: string | null;
  context: ResolutionContext;
  loading: boolean;
  error: string | null;
  canMeasure: boolean;
  measuring: boolean;
  onMeasure: () => void;
  onLog: () => void;
}

export const RESOLUTION_DEBOUNCE_MS = 150;

const DEFAULT_SOURCE_LABEL: Record<ResolutionContext, string> = {
  photometry: "photometry source",
  profile: "profile region",
  forced: "forced position",
  manual: "typed sizes",
};

const BADGE_CLASS = "text-[10px] px-1.5 py-0.5 rounded font-semibold";
const VERDICT_CLASS: Record<ResolutionVerdict, string> = {
  unresolved: "text-zinc-200 bg-zinc-700/50",
  marginal: "text-amber-300 bg-amber-900/30",
  resolved: "text-emerald-300 bg-emerald-900/30",
};

export function ResolutionCardView({
  check,
  missing,
  context,
  loading,
  error,
  canMeasure,
  measuring,
  onMeasure,
  onLog,
}: ResolutionCardViewProps): React.JSX.Element {
  const verdict = check?.verdict ?? null;
  return (
    <div data-resolution-card={context} data-resolution-verdict={verdict ?? "none"} className="bg-zinc-900/80 rounded px-2 py-1.5 space-y-1">
      <div className="flex items-center gap-1.5">
        <span className="text-[9px] text-zinc-500 uppercase">Resolved or not</span>
        {verdict !== null && <span className={`${BADGE_CLASS} ${VERDICT_CLASS[verdict]}`}>{VERDICT_LABEL[verdict]}</span>}
        {loading && <span className="text-[9px] text-zinc-500">Checking…</span>}
      </div>
      {missing !== null && <p className="text-[10px] text-amber-300">{missing}</p>}
      {missing !== null && canMeasure && (
        <button type="button" className={SMALL_BUTTON_CLASS} disabled={measuring} onClick={onMeasure}>
          {measuring ? "Measuring the PSF…" : "Measure PSF from stars"}
        </button>
      )}
      {verdict !== null && <p className="text-[10px] text-zinc-300">{VERDICT_SCIENCE[verdict]}</p>}
      {check !== null && verdict === null && check.psf_fwhm_arcsec !== null && <p className="text-[10px] text-zinc-300">{NO_VERDICT_TEXT}</p>}
      {check !== null && (
        <>
          <dl className="grid grid-cols-[auto_1fr] gap-x-2 gap-y-0.5 items-baseline">
            {resolutionRows(check).map(([label, value]) => (
              <Fragment key={label}>
                <dt className="text-[9px] text-zinc-500">{label}</dt>
                <dd className="text-[10px] font-mono text-zinc-200 break-words">{value}</dd>
              </Fragment>
            ))}
          </dl>
          <p className="text-[9px] text-zinc-500">{CONTEXT_NOTE[context]}</p>
          <WarningList warnings={check.warnings} />
          {check.notes.length > 0 && (
            <details className="text-[9px] text-zinc-500">
              <summary className="cursor-pointer">{`notes (${check.notes.length})`}</summary>
              <ul className="list-disc pl-4 space-y-0.5">
                {check.notes.map((note, index) => (
                  <li key={index}>{note}</li>
                ))}
              </ul>
            </details>
          )}
        </>
      )}
      <ErrorAlert message={error} />
      {check !== null && (
        <button type="button" className={SMALL_BUTTON_CLASS} title="Log this check with its inputs and thresholds" disabled={loading} onClick={onLog}>
          Log
        </button>
      )}
    </div>
  );
}

export function ResolutionCardInner({
  filePath,
  fwhmArcsec,
  halfLightRadiusArcsec,
  snr,
  context,
  sourceLabel,
}: ResolutionCardProps): React.JSX.Element | null {
  const settings = useResolutionSettings();
  const image = useImagePsf(filePath);
  const headerCards = useHeaderSeeing(settings.psfSource === "header" ? filePath : null);
  const wcsScale = useWcsPixelScale(filePath);
  const provenance = useMeasurementProvenance();
  const [check, setCheck] = useState<ResolutionCheck | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const seqRef = useRef(0);

  const visible =
    filePath !== null && (context === "manual" || positiveOrNull(fwhmArcsec) !== null || positiveOrNull(halfLightRadiusArcsec) !== null);
  const choice = psfChoice(settings, image.psf, headerCards);
  const request = resolutionRequest(settings, choice, image.psf?.pixel_scale_arcsec ?? wcsScale, { fwhmArcsec, halfLightRadiusArcsec, snr });
  const requestKey = visible && (choice.missing === null || settings.redshift !== null) ? JSON.stringify(request) : null;
  const requestRef = useRef(request);
  requestRef.current = request;

  useEffect(() => {
    setCheck(null);
    setError(null);
  }, [filePath]);

  useEffect(() => {
    const seq = ++seqRef.current;
    const pending = requestRef.current;
    if (requestKey === null) {
      setCheck(null);
      setError(null);
      setLoading(false);
      return;
    }
    setLoading(true);
    const timer = setTimeout(() => {
      resolutionCheck(pending).then(
        (result) => {
          if (seqRef.current !== seq) return;
          setCheck(result);
          setError(null);
          setLoading(false);
        },
        (reason: unknown) => {
          if (seqRef.current !== seq) return;
          setCheck(null);
          setError(reason instanceof Error ? reason.message : String(reason));
          setLoading(false);
        },
      );
    }, RESOLUTION_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [filePath, requestKey]);

  if (!visible) return null;

  const onMeasure = () => {
    if (filePath !== null) void imagePsfStore.measure(filePath);
  };
  const onLog = () => {
    if (check !== null) measurementLog.append(resolutionCheckEntry(provenance, check, context, sourceLabel ?? DEFAULT_SOURCE_LABEL[context]));
  };

  return (
    <ResolutionCardView
      check={check}
      missing={choice.missing}
      context={context}
      loading={loading}
      error={image.error ?? error}
      canMeasure={settings.psfSource === "image" && image.psf === null}
      measuring={image.loading}
      onMeasure={onMeasure}
      onLog={onLog}
    />
  );
}

export default memo(ResolutionCardInner);
