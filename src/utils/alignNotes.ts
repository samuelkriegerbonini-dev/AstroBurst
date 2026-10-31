import type { AlignedChannel, AlignPlan, AlignResult } from "../shared/types/compose";
import { alignRunMethodLabel, formatAlignOffset } from "./wizard";

export const ALIGN_REF_AUTO = "auto";

export const ALIGN_REF_AUTO_LABEL = "Auto (finest pixel scale)";

export const ALIGN_METHOD_LABEL = "Method";

export const ALIGN_FALLBACK_LABEL = "Fallback (channels without a WCS pair)";

export const ALIGN_METHODS = [
  { value: "phase_correlation", label: "Phase Correlation", detail: "sub-pixel" },
  { value: "affine", label: "Star-based Affine", detail: "rotation" },
] as const;

export interface AlignPlanChip {
  kind: "reference" | "wcs" | "fallback";
  text: string;
  title: string;
}

const ALIGN_REFERENCE_RULE_TITLES: Record<AlignPlan["reference_rule"], string> = {
  selected: "Reference you selected",
  finest_wcs: "Reference: the finest WCS pixel scale (Auto)",
  first: "Reference: the first channel, since no channel has a celestial WCS (Auto)",
};

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

export function alignMethodChoiceLabel(method: string): string {
  return ALIGN_METHODS.find((m) => m.value === method)?.label ?? method;
}

function missingWcsPairReason(channel: string, reference: string, channelHasWcs: boolean, referenceHasWcs: boolean): string {
  if (!channelHasWcs && !referenceHasWcs) return `No celestial WCS in ${channel} or the reference ${reference}`;
  if (!channelHasWcs) return `No celestial WCS in ${channel}`;
  if (!referenceHasWcs) return `No celestial WCS in the reference ${reference}`;
  return `No celestial WCS pair between ${channel} and the reference ${reference}`;
}

export function alignPlanChips(plan: AlignPlan | null, labels: readonly string[], method: string): AlignPlanChip[] | null {
  if (!plan || plan.channels.length !== labels.length) return null;
  const referenceIndex = plan.reference_index;
  const reference = plan.channels[referenceIndex];
  if (!reference) return null;
  const referenceLabel = labels[referenceIndex];
  const methodLabel = alignMethodChoiceLabel(method);
  return plan.channels.map((channel, i): AlignPlanChip => {
    if (i === referenceIndex) {
      return { kind: "reference", text: "reference", title: ALIGN_REFERENCE_RULE_TITLES[plan.reference_rule] };
    }
    if (channel.reprojected) {
      return { kind: "wcs", text: "WCS", title: `Reprojected onto ${referenceLabel} through the celestial WCS of both` };
    }
    const reason = missingWcsPairReason(labels[i], referenceLabel, channel.has_wcs, reference.has_wcs);
    return {
      kind: "fallback",
      text: `no WCS pair: ${methodLabel}`,
      title: `${reason}; resampled by array size and registered by ${methodLabel}`,
    };
  });
}

export function alignMethodSelectLabel(chips: readonly AlignPlanChip[] | null): string {
  return chips?.some((chip) => chip.kind === "wcs") ? ALIGN_FALLBACK_LABEL : ALIGN_METHOD_LABEL;
}
