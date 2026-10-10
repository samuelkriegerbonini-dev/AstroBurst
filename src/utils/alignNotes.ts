import type { AlignedChannel, AlignResult } from "../shared/types/compose";
import { alignRunMethodLabel, formatAlignOffset } from "./wizard";

export const ALIGN_REF_AUTO = "auto";

export const ALIGN_REF_AUTO_LABEL = "Auto (finest pixel scale)";

export interface AlignChannelNote {
  text: string;
  wcs: boolean;
}

export function alignReferenceIndex(inputs: readonly { binId: string }[], choice: string | null): number | null {
  if (choice === null) return null;
  const index = inputs.findIndex((input) => input.binId === choice);
  return index >= 0 ? index : null;
}

export function alignWcsNote(ratio: number, deg: number, k: number, offset: readonly [number, number] | null): string {
  return `reprojected through WCS (scale x${ratio.toFixed(4)}, rotation ${deg >= 0 ? "+" : "-"}${Math.abs(deg).toFixed(2)}°${k > 1 ? `, pre-reduced x${k}` : ""}); ${offset ? `residual ${formatAlignOffset(offset)}` : "residual not measured"}`;
}

export function alignNoWcsNote(channel: string, method: string): string {
  return `no celestial WCS in ${channel}; resampled by array size and registered by ${method}`;
}

export function alignNoWcsPairNote(channel: string, reference: string, method: string): string {
  return `no celestial WCS pair between ${channel} and the reference ${reference}; resampled by array size and registered by ${method}`;
}

export function alignChannelNote(
  channel: AlignedChannel | undefined,
  label: string,
  requestedMethod: string,
  isReference: boolean,
  referenceRule?: AlignResult["reference_rule"],
  referenceLabel?: string,
): AlignChannelNote | null {
  if (!channel || isReference) return null;
  if (channel.reprojected !== true) {
    const method = alignRunMethodLabel({ align_method: channel.method_used ?? requestedMethod });
    const text = referenceRule === "selected" && referenceLabel !== undefined
      ? alignNoWcsPairNote(label, referenceLabel, method)
      : alignNoWcsNote(label, method);
    return { text, wcs: false };
  }
  const ratio = channel.wcs_scale_ratio;
  const deg = channel.wcs_rotation_deg;
  if (ratio == null || deg == null) return null;
  const residual = channel.residual_measured === false ? null : channel.offset;
  return { text: alignWcsNote(ratio, deg, channel.prefilter_k ?? 1, residual), wcs: true };
}
