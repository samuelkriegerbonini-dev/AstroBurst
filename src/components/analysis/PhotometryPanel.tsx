import { useState, useEffect, useRef, useCallback, useId, useMemo, memo } from "react";
import { useTimedFlag } from "../../hooks/useTimedNotice";
import { Check, Copy, Crosshair, Loader2, Star as StarIcon } from "lucide-react";
import { measurePhotometry, photometryGainModel } from "../../services/analysis";
import type { GainModel, PhotometryMeasurement, StarPhotometry, UserZeroPoint } from "../../services/analysis";
import { usePixelClick } from "../../hooks/useMousePixelStore";
import { usePhotometryStatus } from "../../hooks/usePhotometryStatus";
import { useImageDepth } from "../../hooks/usePhotometryLimit";
import { useDqContext } from "../../context/PreviewContext";
import { useToolHost } from "../../context/ToolHostContext";
import { useMeasurementProvenance } from "../../hooks/useMeasurementLog";
import { measurementLog, photometryEntry } from "../../utils/measurementLog";
import {
  ZERO_POINT_CAPTION,
  ZERO_POINT_FIELD,
  ZERO_POINT_TITLE,
  appliedDepthFactor,
  calibratedMagnitude,
  calibrationText,
  convertedRadiusText,
  createClickLogGate,
  depthNoiseText,
  gaiaFailureWarning,
  limitSentence,
  loadGaiaMatchPreference,
  magnitudeTitle,
  measurementWarnings,
  photometryLimit,
  replaceNewest,
  saveGaiaMatchPreference,
  userZeroPoint,
  withWarning,
} from "../../utils/photometryPanel";
import { noiseFactorOffer } from "../../utils/forcedPhotometryRequest";
import { DEFAULT_LIMIT_SIGMA } from "../../utils/upperLimits";
import { GAIN_TITLE, gainCaption, nextGainText } from "../../utils/photometryGain";
import { ZERO_BASED_PIXEL_TITLE } from "../../utils/regionGeometry";
import { skyPositionLine } from "../../utils/coordFormat";
import { DecimalField, ErrorAlert, Toggle } from "../ui";
import { decimalFieldState, firstFieldError, type DecimalFieldOptions } from "../../utils/decimalText";
import ProfilePlot from "../regions/ProfilePlot";
import type { ProfileSeries } from "../regions/ProfilePlot";
import {
  APERTURE_RANGE_HINT,
  FALLBACK_APERTURE_RADIUS_PX,
  MAX_SKY_OUTER_RADIUS_PX,
  abCellText,
  apertureRadiusInRange,
  growthReferenceLines,
  plateauCaption,
  summedFluxUnit,
} from "../../utils/photometryTable";
import { formatFluxDensityJy, formatWithUncertainty } from "../../utils/formatUncertainty";
import {
  RADIUS_UNITS,
  isUndersampled,
  radiiRequest,
  radiusInArcsec,
  radiusInPixels,
  skyLevelError,
  type RadiusUnit,
} from "../../utils/photometryUnits";
import MeasurementBadge from "./MeasurementBadge";
import PhotometryStatusLine from "./PhotometryStatusLine";
import ResolutionCard from "./ResolutionCard";

interface PhotometryPanelProps {
  filePath?: string | null;
  measureKey?: string | null;
}

const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-amber-500/50 w-full";
const GROWTH_PLOT_HEIGHT = 140;
const GROWTH_CURVE_COLOR = "#fbbf24";
const GROWTH_CSV_NAME = "growth-curve";
const ANNULUS_NEEDS_BOTH = "sky annulus needs both an inner and an outer radius";
const APERTURE_DEFAULTS_CAPTION = `blank: r_ap = 1.5 × FWHM (${FALLBACK_APERTURE_RADIUS_PX} px if the FWHM is not measured), sky 2–3 × r_ap`;
const APERTURE_TITLE = `Aperture radius in pixels; blank uses 1.5 × FWHM of the clicked star, or ${FALLBACK_APERTURE_RADIUS_PX} px when its FWHM is not measured`;
const SKY_IN_TITLE = "Inner sky radius in pixels; blank uses 2 × r_ap";
const SKY_OUT_TITLE = "Outer sky radius in pixels; blank uses 3 × r_ap";
const ARCSEC_APERTURE_TITLE = APERTURE_TITLE.replace("in pixels", "in arcsec");
const ARCSEC_SKY_IN_TITLE = SKY_IN_TITLE.replace("in pixels", "in arcsec");
const ARCSEC_SKY_OUT_TITLE = SKY_OUT_TITLE.replace("in pixels", "in arcsec");
const NO_ARCSEC_TITLE = "No celestial WCS: radii in arcsec are not available for this image";
const PIXEL_SCALE_PENDING = "Reading the pixel scale of this image...";
const UNIT_SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono";
const GAIA_PENDING_TEXT = "Querying Gaia DR3 (VizieR)...";
const NO_PLATEAU_TEXT =
  "no plateau: the curve of growth did not flatten before the sky annulus (crowded field, extended source or annulus too close), so the aperture correction, total flux and EE radii are not available";
const COPY_SKY_LABEL = "Copy sky position";
const COPY_SKY_TITLE = "Copy the sky position in decimal degrees and sexagesimal as one line";
const COPIED_FEEDBACK_MS = 1500;
const COPIED_TEXT = "Copied";
const CLIPBOARD_FAILED = "Clipboard copy failed: ";
const UNDERSAMPLED_NOTE =
  "undersampled: below 2 px the width depends on where the star falls on the pixel grid (a few percent at 1.5 px, up to 10 percent at 1 px)";
const NO_JY_TITLE = "No Jy for a zero point that is not AB: the conversion needs the AB offset of the band.";
const USE_DEPTH_FACTOR_LABEL = "Use the Depth noise factor for this aperture";
const REMOVE_DEPTH_FACTOR_LABEL = "Do not use the Depth noise factor";

const APERTURE_FIELD: DecimalFieldOptions = { optional: true };
const SKY_IN_FIELD: DecimalFieldOptions = { optional: true, min: 1 };
const SKY_OUT_FIELD: DecimalFieldOptions = { optional: true, min: 1, max: MAX_SKY_OUTER_RADIUS_PX };
const GAIN_FIELD: DecimalFieldOptions = { optional: true, min: 0.01 };
const ARCSEC_FIELD: DecimalFieldOptions = { optional: true };

function formatRadius(px: number | null | undefined, arcsecPerPx: number | null): string {
  if (px == null || !isFinite(px)) return "--";
  const base = `${px.toFixed(2)} px`;
  return arcsecPerPx != null && isFinite(arcsecPerPx) && arcsecPerPx > 0 ? `${base} (${(px * arcsecPerPx).toFixed(3)}")` : base;
}

function radiusEcho(aperture: number | null, unit: RadiusUnit, arcsecPerPx: number | null): string | null {
  if (aperture === null) return null;
  if (unit === "px") {
    const arcsec = radiusInArcsec(aperture, arcsecPerPx);
    return arcsec === null ? null : `r_ap ${aperture} px = ${arcsec.toFixed(3)} arcsec`;
  }
  const px = radiusInPixels(aperture, "arcsec", arcsecPerPx);
  return px === null ? null : `r_ap ${aperture} arcsec = ${px.toFixed(2)} px`;
}

function growthSeries(phot: StarPhotometry, eeFraction: boolean): ProfileSeries[] {
  const total = phot.flux_total;
  const normalise = eeFraction && total != null && isFinite(total) && total > 0;
  return [
    {
      x: phot.growth_curve.map((p) => p.r),
      y: phot.growth_curve.map((p) => (normalise ? p.flux / total : p.flux)),
      color: GROWTH_CURVE_COLOR,
      label: normalise ? "EE fraction(r)" : "net flux(r)",
    },
  ];
}

function formatMag(v: number | null | undefined): string {
  return v == null || !isFinite(v) ? "--" : v.toFixed(2);
}

function formatSci(v: number): string {
  return isFinite(v) ? v.toExponential(3) : "--";
}

function fluxUnitLabel(unit: string | null): string {
  return unit ? ` ${unit}` : "";
}

function bestMag(p: StarPhotometry): string {
  return p.mag_ab != null ? `AB ${formatMag(p.mag_ab)}` : `inst ${formatMag(p.mag_inst)}`;
}

function limitLog(phot: StarPhotometry, fCorr: number | null) {
  const limit = photometryLimit(phot, fCorr);
  return {
    limitSigma: DEFAULT_LIMIT_SIGMA,
    fCorr: limit.fCorr,
    limit: limit.isLimit ? { limitJy: limit.limitJy, limitAb: limit.limitAb, limitNative: limit.limitNative } : null,
  };
}

function PhotometryPanel({ filePath, measureKey }: PhotometryPanelProps) {
  const apertureId = useId();
  const skyInId = useId();
  const skyOutId = useId();
  const gainId = useId();
  const zeroPointId = useId();
  const [armed, setArmed] = useState(false);
  const [gaiaMatch, setGaiaMatch] = useState(() => loadGaiaMatchPreference());
  const [gaiaPending, setGaiaPending] = useState(false);
  const [apertureText, setApertureText] = useState("");
  const [skyInText, setSkyInText] = useState("");
  const [skyOutText, setSkyOutText] = useState("");
  const [radiusUnit, setRadiusUnit] = useState<RadiusUnit>("px");
  const [gainText, setGainText] = useState("");
  const [gainModel, setGainModel] = useState<GainModel | null>(null);
  const [zeroPointText, setZeroPointText] = useState("");
  const [zeroPointSystem, setZeroPointSystem] = useState<UserZeroPoint["system"]>("ab");
  const [zeroPointPerSecond, setZeroPointPerSecond] = useState(false);
  const [eeFraction, setEeFraction] = useState(false);
  const [isMeasuring, setIsMeasuring] = useState(false);
  const [result, setResult] = useState<PhotometryMeasurement | null>(null);
  const [measuredFCorr, setMeasuredFCorr] = useState<number | null>(null);
  const [acceptedFCorr, setAcceptedFCorr] = useState<number | null>(null);
  const [history, setHistory] = useState<PhotometryMeasurement[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [skyCopied, flagSkyCopied, clearSkyCopied] = useTimedFlag(COPIED_FEEDBACK_MS);
  const click = usePixelClick();
  const { excludeDq } = useDqContext();
  const status = usePhotometryStatus(filePath ?? null, excludeDq, measureKey ?? null);
  const arcsecPerPx = status.status?.pixel_scale_arcsec ?? null;
  const depth = useImageDepth(filePath ?? null);
  const { active } = useToolHost();
  const provenance = useMeasurementProvenance();
  const lastSeqRef = useRef(0);
  const busyRef = useRef(false);
  const [logGate] = useState(createClickLogGate);
  const clickRef = useRef(click);
  clickRef.current = click;
  const gainTextRef = useRef(gainText);
  gainTextRef.current = gainText;
  const gainPrefillRef = useRef("");
  const gainSeqRef = useRef(0);

  const changeGaiaMatch = useCallback((on: boolean) => {
    setGaiaMatch(on);
    saveGaiaMatchPreference(on);
  }, []);

  useEffect(() => {
    logGate.supersede();
    lastSeqRef.current = clickRef.current?.seq ?? lastSeqRef.current;
    setResult(null);
    setMeasuredFCorr(null);
    setHistory([]);
    setError(null);
    setIsMeasuring(false);
    setGaiaPending(false);
    busyRef.current = false;
  }, [filePath, measureKey, logGate]);

  useEffect(() => () => logGate.supersede(), [logGate]);

  useEffect(() => {
    const seq = ++gainSeqRef.current;
    setGainModel(null);
    const cleared = nextGainText(gainTextRef.current, gainPrefillRef.current, null);
    gainPrefillRef.current = cleared.prefill;
    gainTextRef.current = cleared.text;
    setGainText(cleared.text);
    if (!filePath) return;
    const apply = (model: GainModel | null) => {
      if (gainSeqRef.current !== seq) return;
      const next = nextGainText(gainTextRef.current, gainPrefillRef.current, model);
      gainPrefillRef.current = next.prefill;
      setGainModel(model);
      setGainText(next.text);
    };
    photometryGainModel(filePath).then(apply, () => apply(null));
  }, [filePath]);

  const inArcsec = radiusUnit === "arcsec";
  const apertureOptions = inArcsec ? ARCSEC_FIELD : APERTURE_FIELD;
  const skyInOptions = inArcsec ? ARCSEC_FIELD : SKY_IN_FIELD;
  const skyOutOptions = inArcsec ? ARCSEC_FIELD : SKY_OUT_FIELD;
  const apertureField = decimalFieldState(apertureText, apertureOptions);
  const skyInField = decimalFieldState(skyInText, skyInOptions);
  const skyOutField = decimalFieldState(skyOutText, skyOutOptions);
  const gainField = decimalFieldState(gainText, GAIN_FIELD);
  const typedRadii = useMemo(
    () => ({ aperture: apertureField.value ?? undefined, skyInner: skyInField.value ?? undefined, skyOuter: skyOutField.value ?? undefined }),
    [apertureField.value, skyInField.value, skyOutField.value],
  );
  const request = radiiRequest(typedRadii, radiusUnit, arcsecPerPx);
  const { apertureRadius, annulusInner, annulusOuter } = request.radii;
  const gain = gainField.value ?? undefined;
  const offersZeroPoint = status.status !== null && status.status.calibration_source !== "header";
  const zeroPointInput = offersZeroPoint ? zeroPointText : "";
  const zeroPointField = decimalFieldState(zeroPointInput, ZERO_POINT_FIELD);
  const zeroPoint = useMemo(
    () => userZeroPoint(zeroPointInput, zeroPointSystem, zeroPointPerSecond),
    [zeroPointInput, zeroPointSystem, zeroPointPerSecond],
  );
  const fieldError = firstFieldError([
    [`r_ap ${radiusUnit}`, apertureField],
    [`sky in ${radiusUnit}`, skyInField],
    [`sky out ${radiusUnit}`, skyOutField],
    ["gain e-/ADU", gainField],
    ["zero point", zeroPointField],
  ]);
  const statusPending = status.error !== null ? `Status not available: ${status.error}` : PIXEL_SCALE_PENDING;
  const radiusError = inArcsec && status.status === null ? statusPending : request.error;
  const echo = radiusEcho(apertureField.value, radiusUnit, arcsecPerPx);
  const annulusHalfFilled = (annulusInner === undefined) !== (annulusOuter === undefined);
  const apertureOutOfRange = !apertureRadiusInRange(apertureRadius);
  const offer = useMemo(() => noiseFactorOffer(depth, apertureField.value ?? null, radiusUnit), [depth, apertureField.value, radiusUnit]);
  const fCorr = appliedDepthFactor(offer, acceptedFCorr);
  const noiseText = depthNoiseText(offer, acceptedFCorr);

  useEffect(() => {
    if (acceptedFCorr !== null && appliedDepthFactor(offer, acceptedFCorr) === null) setAcceptedFCorr(null);
  }, [offer, acceptedFCorr]);

  const measure = useCallback(
    async (x: number, y: number) => {
      if (!filePath || busyRef.current || apertureOutOfRange || fieldError !== null || radiusError !== null) return;
      const seq = logGate.begin();
      const options = { apertureRadius, annulusInner, annulusOuter, gain, excludeDq };
      const logged = { ...options, radiusUnit, typedRadii, pixelScaleArcsec: arcsecPerPx, zeroPoint: zeroPoint ?? null };
      busyRef.current = true;
      setIsMeasuring(true);
      setGaiaPending(false);
      setError(null);
      const local = await measurePhotometry(filePath, x, y, { ...options, gaiaMatch: false, zeroPoint }).catch((e: unknown) => {
        if (logGate.isCurrent(seq)) {
          setError(e instanceof Error ? e.message : String(e));
          setIsMeasuring(false);
        }
        return null;
      });
      busyRef.current = false;
      if (!local || !logGate.isCurrent(seq)) return;
      setResult(local);
      setMeasuredFCorr(fCorr);
      setHistory((prev) => [local, ...prev].slice(0, 4));
      setIsMeasuring(false);
      const appendLocal = () =>
        measurementLog.append(photometryEntry(provenance, local, { ...logged, ...limitLog(local.photometry, fCorr), gaiaMatch: false }));
      if (!gaiaMatch) {
        logGate.commit(seq, appendLocal);
        return;
      }
      logGate.hold(seq, appendLocal);
      setGaiaPending(true);
      const matched = await measurePhotometry(filePath, x, y, { ...options, gaiaMatch: true, zeroPoint }).catch((e: unknown) =>
        withWarning(local, gaiaFailureWarning(e)),
      );
      if (!logGate.isCurrent(seq)) return;
      setGaiaPending(false);
      setResult(matched);
      setHistory((prev) => replaceNewest(prev, local, matched));
      logGate.commit(seq, () =>
        measurementLog.append(photometryEntry(provenance, matched, { ...logged, ...limitLog(matched.photometry, fCorr), gaiaMatch: true })),
      );
    },
    [
      filePath,
      apertureOutOfRange,
      fieldError,
      radiusError,
      apertureRadius,
      annulusInner,
      annulusOuter,
      gain,
      zeroPoint,
      gaiaMatch,
      excludeDq,
      provenance,
      logGate,
      radiusUnit,
      typedRadii,
      arcsecPerPx,
      fCorr,
    ],
  );

  useEffect(() => {
    if (!active) {
      lastSeqRef.current = clickRef.current?.seq ?? lastSeqRef.current;
      return;
    }
    if (!armed || !click || click.seq === lastSeqRef.current) return;
    lastSeqRef.current = click.seq;
    void measure(click.x, click.y);
  }, [active, armed, click, measure]);

  const changeRadiusUnit = (next: RadiusUnit) => {
    const clears = radiusUnit === "arcsec" && next === "px" && arcsecPerPx === null;
    const convert = (text: string) => (clears ? "" : convertedRadiusText(text, radiusUnit, next, arcsecPerPx));
    setApertureText(convert);
    setSkyInText(convert);
    setSkyOutText(convert);
    setRadiusUnit(next);
  };

  const sky = result?.sky ?? null;
  const copySky = useCallback(async () => {
    if (!sky) return;
    try {
      await navigator.clipboard.writeText(skyPositionLine(sky.ra, sky.dec));
      setError((current) => (current?.startsWith(CLIPBOARD_FAILED) ? null : current));
      flagSkyCopied();
    } catch (e: unknown) {
      clearSkyCopied();
      setError(`${CLIPBOARD_FAILED}${e instanceof Error ? e.message : String(e)}`);
    }
  }, [sky, flagSkyCopied, clearSkyCopied]);

  const phot = result?.photometry;
  const photcal = result?.photcal ?? null;
  const warnings = result ? measurementWarnings(result) : [];
  const summedUnit = result?.summed_unit ?? summedFluxUnit(photcal);
  const unit = fluxUnitLabel(summedUnit);
  const pixelUnit = fluxUnitLabel(result?.bunit ?? photcal?.bunit ?? null);
  const magnitude = phot ? calibratedMagnitude(phot, photcal) : null;
  const limit = phot ? photometryLimit(phot, measuredFCorr) : null;
  const limitText = limit ? limitSentence(limit, summedUnit) : null;
  const magnitudeText =
    limit?.isLimit && limit.limitAb !== null
      ? `> ${limit.limitAb.toFixed(2)} (${limit.sigma} sigma)`
      : abCellText(magnitude?.mag, magnitude?.magErr);
  const noJyTitle =
    photcal && phot?.flux_jy == null && (photcal.mag_system === "vega" || photcal.mag_system === "st") ? NO_JY_TITLE : undefined;
  const growth = useMemo(() => (phot && phot.growth_curve.length > 0 ? growthSeries(phot, eeFraction) : null), [phot, eeFraction]);
  const growthLines = useMemo(() => (phot ? growthReferenceLines(phot) : []), [phot]);

  return (
    <div className="ab-panel overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <StarIcon size={12} className="text-yellow-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">
            Photometry
          </span>
          <MeasurementBadge />
        </div>
        {isMeasuring && <Loader2 size={12} className="animate-spin text-yellow-400/70" />}
      </div>

      <div className="px-3 py-2 space-y-2">
        <PhotometryStatusLine filePath={filePath ?? null} measureKey={measureKey ?? null} excludeDq={excludeDq} zeroPoint={zeroPoint ?? null} />
        <Toggle label="Measure on image click" checked={armed} accent="amber" onChange={setArmed} />

        <div className="flex flex-col gap-1">
          <div className="flex items-center justify-between gap-1.5">
            <span className="text-[9px] text-zinc-500 uppercase">Aperture</span>
            <select
              aria-label="Radius unit"
              value={radiusUnit}
              onChange={(e) => changeRadiusUnit(e.target.value as RadiusUnit)}
              className={UNIT_SELECT_CLASS}
            >
              {RADIUS_UNITS.map((unit) => {
                const unavailable = unit === "arcsec" && arcsecPerPx === null;
                return (
                  <option key={unit} value={unit} disabled={unavailable} title={unavailable ? NO_ARCSEC_TITLE : undefined}>
                    {unit}
                  </option>
                );
              })}
            </select>
          </div>
          <div className="grid grid-cols-4 gap-1.5">
            <div className="flex flex-col gap-0.5">
              <label htmlFor={apertureId} className="text-[9px] text-zinc-500">
                r_ap {radiusUnit}
              </label>
              <DecimalField
                id={apertureId}
                {...apertureOptions}
                value={apertureText}
                placeholder="auto"
                title={inArcsec ? ARCSEC_APERTURE_TITLE : APERTURE_TITLE}
                onChange={setApertureText}
                className={INPUT_CLASS}
              />
            </div>
            <div className="flex flex-col gap-0.5">
              <label htmlFor={skyInId} className="text-[9px] text-zinc-500">
                sky in {radiusUnit}
              </label>
              <DecimalField
                id={skyInId}
                {...skyInOptions}
                value={skyInText}
                placeholder="2×r"
                title={inArcsec ? ARCSEC_SKY_IN_TITLE : SKY_IN_TITLE}
                onChange={setSkyInText}
                className={INPUT_CLASS}
              />
            </div>
            <div className="flex flex-col gap-0.5">
              <label htmlFor={skyOutId} className="text-[9px] text-zinc-500">
                sky out {radiusUnit}
              </label>
              <DecimalField
                id={skyOutId}
                {...skyOutOptions}
                value={skyOutText}
                placeholder="3×r"
                title={inArcsec ? ARCSEC_SKY_OUT_TITLE : SKY_OUT_TITLE}
                onChange={setSkyOutText}
                className={INPUT_CLASS}
              />
            </div>
            <div className="flex flex-col gap-0.5">
              <label htmlFor={gainId} className="text-[9px] text-zinc-500">
                gain e-/ADU
              </label>
              <DecimalField
                id={gainId}
                {...GAIN_FIELD}
                value={gainText}
                placeholder="none"
                title={GAIN_TITLE}
                data-gain-source={gainModel?.source ?? ""}
                onChange={setGainText}
                className={INPUT_CLASS}
              />
            </div>
          </div>
          {echo && (
            <div className="text-[9px] text-zinc-500" data-radius-echo>
              {echo}
            </div>
          )}
          <div className="text-[9px] text-zinc-600">{APERTURE_DEFAULTS_CAPTION}</div>
          <div className="text-[9px] text-zinc-500" data-gain-caption>
            {gainCaption(gainModel, gainText)}
          </div>
          {offersZeroPoint && (
            <div className="flex flex-col gap-0.5">
              <label htmlFor={zeroPointId} className="text-[9px] text-zinc-500">
                zero point mag
              </label>
              <div className="flex items-center gap-1.5">
                <DecimalField
                  id={zeroPointId}
                  {...ZERO_POINT_FIELD}
                  value={zeroPointText}
                  placeholder="none"
                  title={ZERO_POINT_TITLE}
                  onChange={setZeroPointText}
                  className={INPUT_CLASS}
                />
                <select
                  aria-label="Zero point system"
                  value={zeroPointSystem}
                  onChange={(e) => setZeroPointSystem(e.target.value as UserZeroPoint["system"])}
                  className={UNIT_SELECT_CLASS}
                >
                  <option value="ab">AB</option>
                  <option value="vega">Vega</option>
                </select>
              </div>
              <Toggle label="Zero point is per second" checked={zeroPointPerSecond} accent="amber" onChange={setZeroPointPerSecond} />
              <div className="text-[9px] text-zinc-500">{ZERO_POINT_CAPTION}</div>
            </div>
          )}
          {annulusHalfFilled && <div className="text-[9px] text-amber-400/90">{ANNULUS_NEEDS_BOTH}</div>}
          {apertureOutOfRange && <div className="text-[9px] text-amber-400/90">{APERTURE_RANGE_HINT}</div>}
          {fieldError && <div className="text-[9px] text-amber-400/90" data-field-error>{fieldError}</div>}
          {radiusError && (
            <div className="text-[9px] text-amber-400/90" data-radius-error>
              {radiusError}
            </div>
          )}
          {noiseText && (
            <div data-depth-noise-offer={offer.kind} className="text-[9px] text-zinc-500">
              {noiseText}{" "}
              {offer.kind === "usable" &&
                (acceptedFCorr === offer.fCorr ? (
                  <button
                    type="button"
                    data-action="remove-depth-fcorr"
                    aria-label={REMOVE_DEPTH_FACTOR_LABEL}
                    onClick={() => setAcceptedFCorr(null)}
                    className="text-zinc-500 hover:text-zinc-200 transition-colors"
                  >
                    Remove
                  </button>
                ) : (
                  <button
                    type="button"
                    data-action="use-depth-fcorr"
                    aria-label={USE_DEPTH_FACTOR_LABEL}
                    onClick={() => setAcceptedFCorr(offer.fCorr)}
                    className="text-zinc-500 hover:text-zinc-200 transition-colors"
                  >
                    Use
                  </button>
                ))}
            </div>
          )}
        </div>

        <Toggle label="Match Gaia DR3 (online)" checked={gaiaMatch} accent="amber" onChange={changeGaiaMatch} />

        {armed && (
          <div className="flex items-center gap-1.5 text-[10px] text-yellow-400/70">
            <Crosshair size={10} />
            <span>Select Crosshair in the viewer toolbar, then click a star.</span>
          </div>
        )}

        <ErrorAlert compact message={error} />

        {phot && (
          <div className="grid grid-cols-2 gap-1.5 text-[10px]">
            <div className="bg-zinc-900/80 rounded px-2 py-1.5 col-span-2 flex flex-wrap items-center gap-1.5">
              <span className="text-zinc-500">Calibration</span>
              <span className={photcal ? "text-zinc-300" : "text-amber-400/90"}>
                {calibrationText(photcal) ?? "no calibration in header"}
              </span>
              {phot.err_used && (
                <span className="text-[9px] px-1.5 py-0.5 rounded text-emerald-300 bg-emerald-900/30">ERR plane</span>
              )}
              {phot.n_masked > 0 && (
                <span className="text-[9px] px-1.5 py-0.5 rounded text-amber-300 bg-amber-900/30">
                  {phot.n_masked} masked
                </span>
              )}
              {phot.n_nonfinite > 0 && (
                <span className="text-[9px] px-1.5 py-0.5 rounded text-amber-300 bg-amber-900/30">
                  {phot.n_nonfinite} no data
                </span>
              )}
              {phot.n_saturated > 0 && (
                <span className="text-[9px] px-1.5 py-0.5 rounded text-red-300 bg-red-900/30">
                  {phot.n_saturated} saturated
                </span>
              )}
            </div>
            {photcal && photcal.notes.length > 0 && (
              <details className="text-[10px] font-mono text-zinc-600 col-span-2">
                <summary className="cursor-pointer select-none">calibration notes ({photcal.notes.length})</summary>
                <ul className="mt-0.5 flex flex-col gap-0.5 text-zinc-500">
                  {photcal.notes.map((note, i) => (
                    <li key={i} className="break-words">
                      {note}
                    </li>
                  ))}
                </ul>
              </details>
            )}
            <div className="bg-zinc-900/80 rounded px-2 py-1.5" title={ZERO_BASED_PIXEL_TITLE}>
              <div className="text-zinc-500">Centroid (0-based)</div>
              <div className="text-yellow-300 font-mono">
                {phot.x.toFixed(2)}, {phot.y.toFixed(2)}
              </div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">Net flux{unit}</div>
              <div className="text-yellow-300 font-mono">{formatWithUncertainty(phot.net_flux, phot.flux_err)}</div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">{magnitudeTitle(magnitude?.system)}</div>
              <div className="text-yellow-300 font-mono">{magnitudeText}</div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">Flux density</div>
              <div className="text-yellow-300 font-mono" title={noJyTitle}>
                {formatFluxDensityJy(phot.flux_jy, phot.flux_err_jy)}
              </div>
            </div>
            {limitText && (
              <div className="col-span-2 text-[9px] text-amber-400/90" data-photometry-limit>
                {limitText}
              </div>
            )}
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">Mag (inst){phot.st_mag != null ? " / ST" : ""}</div>
              <div className="text-zinc-300 font-mono">
                {formatMag(phot.mag_inst)}
                {phot.st_mag != null ? ` / ${formatMag(phot.st_mag)}` : ""}
              </div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">SNR</div>
              <div className="text-yellow-300 font-mono">{phot.snr.toFixed(1)}</div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">FWHM</div>
              <div className="text-yellow-300 font-mono">{formatRadius(phot.fwhm, arcsecPerPx)}</div>
              {isUndersampled(phot.fwhm) && <div className="text-[9px] text-amber-400/90">{UNDERSAMPLED_NOTE}</div>}
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">Aperture</div>
              <div className="text-zinc-300 font-mono">
                r={formatRadius(phot.aperture_radius, arcsecPerPx)} ({phot.aperture_pixels}px)
              </div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5 col-span-2">
              <div className="text-zinc-500">
                Sky level per pixel ({phot.bg_pixels} px, annulus {phot.sky_inner.toFixed(1)} - {phot.sky_outer.toFixed(1)} px)
              </div>
              <div className="text-zinc-300 font-mono">
                {formatWithUncertainty(phot.bg_mean, null)}
                {pixelUnit}; pixel scatter {formatWithUncertainty(phot.bg_sigma, null)}; error of the level{" "}
                {formatWithUncertainty(skyLevelError(phot.bg_sigma, phot.bg_pixels), null)}
              </div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">EE50 r</div>
              <div className="text-yellow-300 font-mono">{formatRadius(phot.ee50_radius, arcsecPerPx)}</div>
            </div>
            <div className="bg-zinc-900/80 rounded px-2 py-1.5">
              <div className="text-zinc-500">EE80 r</div>
              <div className="text-yellow-300 font-mono">{formatRadius(phot.ee80_radius, arcsecPerPx)}</div>
            </div>
            <div className="col-span-2">
              <ResolutionCard
                filePath={filePath ?? null}
                fwhmArcsec={radiusInArcsec(phot.fwhm, arcsecPerPx)}
                halfLightRadiusArcsec={radiusInArcsec(phot.ee50_radius, arcsecPerPx)}
                snr={phot.snr}
                context="photometry"
                sourceLabel={`source at (${phot.x.toFixed(1)}, ${phot.y.toFixed(1)})`}
              />
            </div>
            {phot.aperture_correction == null && (
              <div className="text-[9px] text-zinc-500 col-span-2">{NO_PLATEAU_TEXT}</div>
            )}
            {phot.aperture_correction != null && (
              <div className="bg-zinc-900/80 rounded px-2 py-1.5 col-span-2">
                <div className="text-zinc-500">
                  Aperture correction
                  {plateauCaption(phot.plateau_radius)}
                </div>
                <div className="text-zinc-300 font-mono">
                  {phot.aperture_correction.toFixed(4)}
                  {phot.flux_total != null ? ` → total ${formatSci(phot.flux_total)}${unit}` : ""}
                  {phot.mag_ab_total != null ? `, AB ${formatMag(phot.mag_ab_total)}` : ""}
                </div>
                {phot.ee_in_aperture != null && (
                  <div className="text-zinc-500">
                    EE in aperture <span className="text-zinc-300 font-mono">{phot.ee_in_aperture.toFixed(4)}</span>
                  </div>
                )}
              </div>
            )}
            {sky && (
              <div className="bg-zinc-900/80 rounded px-2 py-1.5 col-span-2">
                <div className="flex items-center justify-between gap-1">
                  <span className="text-zinc-500">Sky (RA, Dec)</span>
                  <button
                    type="button"
                    onClick={() => void copySky()}
                    aria-label={COPY_SKY_LABEL}
                    title={COPY_SKY_TITLE}
                    className="text-zinc-500 hover:text-zinc-200 transition-colors"
                  >
                    {skyCopied ? <Check size={10} className="text-emerald-400" /> : <Copy size={10} />}
                  </button>
                  <span role="status" className="sr-only" data-copy-status>
                    {skyCopied ? COPIED_TEXT : ""}
                  </span>
                </div>
                <div className="text-zinc-300 font-mono">
                  {sky.ra.toFixed(5)}°, {sky.dec.toFixed(5)}°
                </div>
              </div>
            )}
            {gaiaPending && (
              <div className="flex items-center gap-1.5 text-emerald-400/80 col-span-2">
                <Loader2 size={10} className="animate-spin" />
                <span>{GAIA_PENDING_TEXT}</span>
              </div>
            )}
            {result?.gaia && (
              <div className="bg-emerald-900/20 border border-emerald-800/20 rounded px-2 py-1.5 col-span-2">
                <div className="text-emerald-500/80">Gaia DR3 match ({result.gaia.separation_arcsec.toFixed(1)}")</div>
                <div className="text-emerald-300 font-mono">
                  G = {formatMag(result.gaia.gmag)}  BP−RP = {result.gaia.bp_rp.toFixed(3)}
                </div>
              </div>
            )}
            {phot.saturated && (
              <div className="text-[10px] text-amber-400/90 bg-amber-900/20 border border-amber-800/30 rounded px-2 py-1.5 col-span-2">
                Star appears saturated (level from {phot.saturation_source}) - flux and magnitude are unreliable.
              </div>
            )}
            {warnings.length > 0 && (
              <div className="text-[9px] text-amber-300/90 bg-amber-900/15 border border-amber-800/30 rounded px-2 py-1 col-span-2 space-y-0.5">
                {warnings.map((w, i) => (
                  <div key={i}>{w}</div>
                ))}
              </div>
            )}
          </div>
        )}

        {phot && growth && (
          <div className="flex flex-col gap-1">
            <span className="text-[9px] text-zinc-600 uppercase">Curve of growth</span>
            <ProfilePlot
              series={growth}
              xLabel="r (px)"
              yLabel={eeFraction && phot.flux_total != null ? "EE fraction" : `net flux${unit}`}
              height={GROWTH_PLOT_HEIGHT}
              referenceLines={growthLines}
              toolbar
              csvName={GROWTH_CSV_NAME}
            />
            <Toggle label="EE fraction" checked={eeFraction} disabled={phot.flux_total == null} accent="amber" onChange={setEeFraction} />
          </div>
        )}

        {history.length > 1 && (
          <div className="flex flex-col gap-0.5">
            <span className="text-[9px] text-zinc-600 uppercase">Previous</span>
            {history.slice(1).map((m, i) => (
              <div key={i} className="flex justify-between text-[9px] font-mono text-zinc-500">
                <span>
                  ({m.photometry.x.toFixed(0)}, {m.photometry.y.toFixed(0)})
                </span>
                <span>{bestMag(m.photometry)}</span>
                <span>SNR {m.photometry.snr.toFixed(0)}</span>
                <span>{m.gaia ? `G ${formatMag(m.gaia.gmag)}` : "--"}</span>
              </div>
            ))}
          </div>
        )}

        {!phot && !error && (
          <div className="text-[10px] text-zinc-600">
            Aperture photometry with a local background annulus and error propagation from the error layer.
            The calibration is read from the header and its origin is shown with the result; a zero point can
            be typed when the header has none. Magnitudes are AB unless the zero point states another system; a
            Gaia match adds the catalog G magnitude.
          </div>
        )}
      </div>
    </div>
  );
}

export default memo(PhotometryPanel);
