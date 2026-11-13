import type { TsoFile } from "../shared/types/tso";
import type { TsoTransitBin, TsoTransitSpectrum } from "../shared/types/tsoTransit";
import { buildCsv, CSV_LINE_END, type CsvColumn } from "./catalogCsv";
import { PROVENANCE_FILE_SEPARATOR, provenanceLines, unitLines, withProvenance, type ProvenanceEntry } from "./csvProvenance";
import { formatSignificant } from "./formatSignificant";
import type { MeasurementLogDraft } from "./measurementLog";
import { tsoDepthSpectrumCsvFileName, type TsoExportContext } from "./tsoExports";
import { bandText, tsoTimeName, type DepthSpectrum } from "./tsoLightCurves";
import { TRANSIT_FIT_LABEL } from "./tsoTransitSpectrum";

type TsoLogFile = Pick<TsoFile, "file_name" | "sha256" | "level">;

export const TRANSIT_SPECTRUM_PRODUCT = "tso_transit_spectrum";
export const NOT_MODELLED =
  "orbital eccentricity; the finite exposure time (the model is evaluated at the mid-integration times); limb darkening from stellar models; systematics beyond the baseline polynomial; correlated noise; MCMC";

const DEPTH_RULE = "(Rp/Rs)^2 from the fit; depth_err is its 1 sigma error";
const COVARIANCE_ERRORS = "1 sigma from the covariance of the fit";
const SCALED_ERRORS = "1 sigma from the covariance, multiplied by sqrt(chi2 / dof) of each bin";
const QUICK_DEPTH_RULE = "quick look D = 1 - F_in / F_out of the same band";
const DEPTH_UNIT = "(Rp/Rs)^2";
const RADIUS_UNIT = "Rp/Rs";
const MICROMETRES = "um";
const FRACTION = "fraction";
const RATIO_DIGITS = 3;
const FIT_ERRORS_NOTE =
  "depth is (Rp/Rs)^2 from the fit; depth_err, rp_err, u1_err and u2_err are 1 sigma errors of the least-squares fit";
const GRAZING_NOTE = "grazing transit (b + Rp/Rs above 1): (Rp/Rs)^2 is not the fraction of light blocked";

const TRANSIT_UNITS: Record<string, string> = {
  band_lo_um: MICROMETRES,
  band_hi_um: MICROMETRES,
  centre_um: MICROMETRES,
  depth: DEPTH_UNIT,
  depth_err: DEPTH_UNIT,
  rp: RADIUS_UNIT,
  rp_err: RADIUS_UNIT,
  rms: "fraction of the median flux",
  quick_depth: FRACTION,
  quick_depth_err: FRACTION,
};

function sha256List(files: readonly TsoLogFile[]): string {
  return files.map((f) => f.sha256).join(PROVENANCE_FILE_SEPARATOR);
}

function fittedPerBin(spectrum: TsoTransitSpectrum): string {
  const order = spectrum.fit.baseline_order;
  return spectrum.fit.limb_darkening === "free"
    ? `Rp/Rs, u1, u2 and a baseline polynomial of order ${order}`
    : `Rp/Rs and a baseline polynomial of order ${order}`;
}

function limbEntries(spectrum: TsoTransitSpectrum): ProvenanceEntry[] {
  const { fit } = spectrum;
  return fit.limb_darkening === "free"
    ? [
        ["u1_start", fit.u1],
        ["u2_start", fit.u2],
      ]
    : [
        ["u1", fit.u1],
        ["u2", fit.u2],
      ];
}

function quickRow(quick: DepthSpectrum | null, band: readonly [number, number]) {
  return quick?.rows.find((row) => row.band[0] === band[0] && row.band[1] === band[1]);
}

export function tsoTransitSpectrumCsv(spectrum: TsoTransitSpectrum, quick: DepthSpectrum | null, context: TsoExportContext): string {
  const { fit } = spectrum;
  const entries: ProvenanceEntry[] = [
    ["software", context.software],
    ["exported_utc", context.exportedUtc],
    ["file", context.files.map((f) => f.file_name).join(PROVENANCE_FILE_SEPARATOR)],
    ["product", TRANSIT_SPECTRUM_PRODUCT],
    ["series", spectrum.label],
    ["detector", spectrum.series.detector],
    ["sporder", spectrum.series.sporder ?? "none"],
    ["mode", spectrum.mode],
    ["model", spectrum.model],
    ["fitted_per_bin", fittedPerBin(spectrum)],
    ["not_modelled", NOT_MODELLED],
    ["period_days", fit.period_days],
    ["t0", fit.t0],
    ["t0_time", tsoTimeName(spectrum.time_scale)],
    ["a_rs", fit.a_rs],
    ["b", fit.b],
    ["limb_darkening", fit.limb_darkening],
    ...limbEntries(spectrum),
    ["baseline_order", fit.baseline_order],
    ["rp_start", fit.rp_start],
    ["n_integrations", spectrum.n_integrations],
    ["n_excluded", spectrum.n_excluded],
    ["excluded_rows", fit.exclude_rows.length === 0 ? null : fit.exclude_rows.join(PROVENANCE_FILE_SEPARATOR)],
    ["depth", DEPTH_RULE],
    ["errors", fit.scale_errors ? SCALED_ERRORS : COVARIANCE_ERRORS],
    ["quick_depth", QUICK_DEPTH_RULE],
    ["set_id", spectrum.set_id],
    ["source_level", [...new Set(context.files.map((f) => f.level))].join(PROVENANCE_FILE_SEPARATOR)],
    ["source_sha256", sha256List(context.files)],
    ...spectrum.warnings.map((warning): ProvenanceEntry => ["warning", warning]),
    ...spectrum.notes.map((note): ProvenanceEntry => ["note", note]),
  ];
  const columns: CsvColumn<TsoTransitBin>[] = [
    { header: "band", value: (b) => bandText(b.band_um) },
    { header: "band_lo_um", value: (b) => b.band_um[0] },
    { header: "band_hi_um", value: (b) => b.band_um[1] },
    { header: "centre_um", value: (b) => (b.band_um[0] + b.band_um[1]) / 2 },
    { header: "depth", value: (b) => b.depth },
    { header: "depth_err", value: (b) => b.depth_err },
    { header: "rp", value: (b) => b.rp },
    { header: "rp_err", value: (b) => b.rp_err },
    { header: "u1", value: (b) => b.u1 },
    { header: "u1_err", value: (b) => b.u1_err },
    { header: "u2", value: (b) => b.u2 },
    { header: "u2_err", value: (b) => b.u2_err },
    { header: "chi2", value: (b) => b.chi2 },
    { header: "dof", value: (b) => b.dof },
    { header: "rms", value: (b) => b.rms },
    { header: "n_points", value: (b) => b.n_points },
    { header: "n_in_transit", value: (b) => b.n_in_transit },
    { header: "weighted", value: (b) => b.weighted },
    { header: "grazing", value: (b) => b.grazing },
    { header: "npixels_ratio", value: (b) => b.npixels_ratio },
    { header: "pixel_loss", value: (b) => b.pixel_loss },
    { header: "quick_depth", value: (b) => quickRow(quick, b.band_um)?.depth },
    { header: "quick_depth_err", value: (b) => quickRow(quick, b.band_um)?.err },
    { header: "error", value: (b) => b.error },
    { header: "warnings", value: (b) => b.warnings.join(PROVENANCE_FILE_SEPARATOR) },
  ];
  return withProvenance([...provenanceLines(entries), ...unitLines(TRANSIT_UNITS)], buildCsv(columns, spectrum.bins), CSV_LINE_END);
}

export function tsoTransitSpectrumCsvFileName(spectrum: Pick<TsoTransitSpectrum, "label" | "mode">, firstFileName: string): string {
  return tsoDepthSpectrumCsvFileName(spectrum, firstFileName).replace(/depth_spectrum\.csv$/, "transit_spectrum.csv");
}

function binNotes(spectrum: TsoTransitSpectrum, bin: TsoTransitBin): string[] {
  return [
    TRANSIT_FIT_LABEL,
    FIT_ERRORS_NOTE,
    `band edges in um; t0 in days, ${tsoTimeName(spectrum.time_scale)}`,
    ...(bin.grazing ? [GRAZING_NOTE] : []),
    ...(bin.pixel_loss && bin.npixels_ratio !== null
      ? [
          `mean NPIXELS inside the transit is ${formatSignificant(bin.npixels_ratio, RATIO_DIGITS)} of the baseline: the extraction lost pixels during the event, so this depth is biased`,
        ]
      : []),
    ...bin.notes,
    ...bin.warnings,
  ];
}

function transitEntry(spectrum: TsoTransitSpectrum, bin: TsoTransitBin, files: readonly TsoLogFile[], savedPath: string | null): MeasurementLogDraft {
  const { fit } = spectrum;
  return {
    kind: "tso_transit_spectrum",
    file: files[0]?.file_name ?? null,
    image: `${bin.n_in_transit} in transit, ${bin.n_points} points`,
    dq: spectrum.mode === "masked" ? "excluded" : "not_applied",
    unit: DEPTH_UNIT,
    source: JSON.stringify({
      kind: "x1dints",
      files: files.map((f) => f.file_name),
      detector: spectrum.series.detector,
      sporder: spectrum.series.sporder,
      band_um: bin.band_um,
    }),
    params: {
      series: spectrum.label,
      mode: spectrum.mode,
      band: bandText(bin.band_um),
      band_lo_um: bin.band_um[0],
      band_hi_um: bin.band_um[1],
      model: spectrum.model,
      period_days: fit.period_days,
      t0: fit.t0,
      time_scale: tsoTimeName(spectrum.time_scale),
      a_rs: fit.a_rs,
      b: fit.b,
      limb_darkening: fit.limb_darkening,
      baseline_order: fit.baseline_order,
      scale_errors: fit.scale_errors,
      rp_start: fit.rp_start,
      n_excluded: spectrum.n_excluded,
      set_id: spectrum.set_id,
      source_sha256: sha256List(files),
      saved_path: savedPath,
    },
    values: {
      depth: bin.depth,
      depth_err: bin.depth_err,
      rp: bin.rp,
      rp_err: bin.rp_err,
      u1: bin.u1,
      u1_err: bin.u1_err,
      u2: bin.u2,
      u2_err: bin.u2_err,
      chi2: bin.chi2,
      dof: bin.dof,
      rms: bin.rms,
      n_points: bin.n_points,
      n_in_transit: bin.n_in_transit,
      npixels_ratio: bin.npixels_ratio,
    },
    notes: binNotes(spectrum, bin),
  };
}

export function tsoTransitSpectrumEntries(
  spectrum: TsoTransitSpectrum,
  files: readonly TsoLogFile[],
  savedPath: string | null,
): MeasurementLogDraft[] {
  return spectrum.bins.filter((bin) => bin.depth !== null).map((bin) => transitEntry(spectrum, bin, files, savedPath));
}
