import type { DepthAperture, ImageDepth } from "../shared/types/depth";
import { buildCsv, CSV_LINE_END, type CsvColumn } from "./catalogCsv";
import { provenanceLines, summedPixelUnit, unitLines, withProvenance } from "./csvProvenance";
import { exportStem } from "./imageRef";

export interface CsvContext {
  fileName: string | null;
  appVersion: string;
  exportedUtc: string;
}

export const DEPTH_CSV_COLUMNS: CsvColumn<DepthAperture>[] = [
  { header: "x_px", value: (a) => a.x },
  { header: "y_px", value: (a) => a.y },
  { header: "sum", value: (a) => a.sum },
  { header: "formal_err", value: (a) => a.formal_err },
  { header: "usable", value: (a) => a.usable },
];

export const DEPTH_WEIGHTS_NOTE = "5 x 5 sub-pixel samples per boundary pixel; formal error = sqrt(sum of (weight x sigma)^2)";

const MICROJANSKY_PER_JANSKY = 1e6;
const UNKNOWN_SUM_UNIT = "image unit x pix";
const POSITIONS_NOTE = "0-based pixels, integer pixel centres";

export function depthCsv(depth: ImageDepth, context: CsvContext): string {
  const sumUnit = summedPixelUnit(depth.bunit) ?? UNKNOWN_SUM_UNIT;
  const lines = [
    ...provenanceLines([
      ["software", `AstroBurst ${context.appVersion}`],
      ["exported_utc", context.exportedUtc],
      ["file", context.fileName],
      ["definition", depth.definition],
      ["radius_arcsec", depth.radius_arcsec],
      ["radius_px", depth.radius_px],
      ["pixel_scale_arcsec", depth.pixel_scale_arcsec],
      ["n_requested", depth.n_requested],
      ["n_placed", depth.n_placed],
      ["n_used", depth.n_used],
      ["seed", depth.seed],
      ["n_sigma", depth.n_sigma],
      ["detect_nsigma", depth.detect_nsigma],
      ["masked_fraction", depth.masked_fraction],
      ["sigma_empty", depth.sigma_empty_native],
      ["sigma_clipped", depth.sigma_clipped_native],
      ["sigma_formal", depth.sigma_formal_native],
      ["f_corr", depth.f_corr],
      ["limit", depth.limit_native],
      ["limit_ujy", depth.limit_jy === null ? null : depth.limit_jy * MICROJANSKY_PER_JANSKY],
      ["limit_ab", depth.limit_ab],
      ["psf_fwhm_arcsec", depth.psf_fwhm_arcsec],
      ["psf_limit_formal", depth.psf_limit_formal_native],
      ["psf_limit_formal_ab", depth.psf_limit_formal_ab],
      ["calibration", depth.calibration_origin],
      ["error_layer", depth.err_origin],
      ["mask_layer", depth.dq_source],
      ["aperture_weights", DEPTH_WEIGHTS_NOTE],
      ["positions", POSITIONS_NOTE],
    ]),
    ...unitLines({ x_px: "pixel", y_px: "pixel", sum: sumUnit, formal_err: sumUnit }),
  ];
  return withProvenance(lines, buildCsv(DEPTH_CSV_COLUMNS, depth.apertures), CSV_LINE_END);
}

export function depthCsvFileName(filePath: string | null): string {
  return `${exportStem(filePath ?? "", "image")}_depth.csv`;
}
