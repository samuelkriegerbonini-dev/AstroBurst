import type { WcsInfo, WcsOrientationInfo } from "../shared/types/astrometry";

const SCALE_DIGITS = 3;
const ROTATION_DIGITS = 1;
const SIP_RESIDUAL_DIGITS = 3;
const MAS_DIGITS = 2;
const SUB_RESOLUTION_PX = 0.0005;
const REFUSAL_SUFFIX_MAX_CHARS = 80;

function finiteResidual(value: number | null | undefined): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

export function sipSuffix(info: WcsOrientationInfo): string {
  if (!info.sip_present) return "";
  const { sip_max_err_px: max, sip_inv_err_px: inv } = info;
  if (!finiteResidual(max)) return " - SIP";
  const fit = `gWCS fit ≤${max.toFixed(SIP_RESIDUAL_DIGITS)} px`;
  const inverse = finiteResidual(inv) ? `, inverse ≤${inv.toFixed(SIP_RESIDUAL_DIGITS)} px` : "";
  return ` - SIP (${fit}${inverse})`;
}

export function gwcsSuffix(info: WcsOrientationInfo): string {
  const { gwcs_vs_header_sip_max_px: px, gwcs_vs_header_sip_max_mas: mas } = info;
  if (!finiteResidual(px) || !finiteResidual(mas)) return "";
  const shownPx = px < SUB_RESOLUTION_PX ? "<0.001" : px.toFixed(SIP_RESIDUAL_DIGITS);
  return ` - header SIP within ${shownPx} px (${mas.toFixed(MAS_DIGITS)} mas)`;
}

function refusalSuffix(info: WcsOrientationInfo): string {
  if (info.wcs_kind !== "header" || typeof info.gwcs_refusal !== "string") return "";
  const suffix = ` - gWCS not used: ${info.gwcs_refusal}`;
  return suffix.length > REFUSAL_SUFFIX_MAX_CHARS ? `${suffix.slice(0, REFUSAL_SUFFIX_MAX_CHARS)}…` : suffix;
}

function gwcsHead(info: WcsOrientationInfo): string | null {
  const { gwcs_steps: steps, gwcs_frames: frames } = info;
  if (typeof steps !== "number" || !Array.isArray(frames)) return null;
  return `gWCS (${steps} steps: ${frames.join("→")})`;
}

export function orientationLine(info: WcsInfo): string | null {
  const { projection, pixel_scale_x_arcsec, pixel_scale_y_arcsec, rotation_deg, flipped } = info;
  if (
    !projection ||
    pixel_scale_x_arcsec === undefined ||
    pixel_scale_y_arcsec === undefined ||
    rotation_deg === undefined ||
    flipped === undefined
  ) {
    return null;
  }
  const scale = `${pixel_scale_x_arcsec.toFixed(SCALE_DIGITS)}"/px x ${pixel_scale_y_arcsec.toFixed(SCALE_DIGITS)}"/px`;
  const east = flipped ? "right" : "left";
  const gwcs = info.wcs_kind === "gwcs";
  const head = (gwcs ? gwcsHead(info) : null) ?? projection;
  const suffix = gwcs ? gwcsSuffix(info) : `${sipSuffix(info)}${refusalSuffix(info)}`;
  return `${head} - ${scale} - rot ${rotation_deg.toFixed(ROTATION_DIGITS)} deg - E ${east}${suffix}`;
}
