import type { GainModel } from "../services/analysis";
import { formatSignificant } from "./formatSignificant";

export const GAIN_TITLE = "Gain in e-/ADU; prefilled from the header when known";

const ERR_PLANE_CAPTION = "Poisson noise from the ERR plane; gain not used";

function finite(value: number | null | undefined): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

export function gainPrefillText(model: GainModel | null): string {
  if (!model || model.poisson_route !== "header_gain") return "";
  const gain = model.effective_gain;
  return finite(gain) && gain > 0 ? formatSignificant(gain) : "";
}

export function nextGainText(current: string, lastPrefill: string, model: GainModel | null): { text: string; prefill: string } {
  const prefill = gainPrefillText(model);
  return { text: current === "" || current === lastPrefill ? prefill : current, prefill };
}

function scaledGainCaption(model: GainModel, effective: number): string | null {
  const { gain_e_per_adu: unitGain, source, ncombine, drizzle_scale: scale } = model;
  if (!finite(unitGain) || !source || !finite(ncombine)) return null;
  const head = `gain ${formatSignificant(effective)} e-/ADU = `;
  const product = `${ncombine} × ${formatSignificant(unitGain)} (${source})`;
  const frames = `(NCOMBINE=${ncombine})`;
  if (finite(scale)) {
    return `${head}${product} / ${formatSignificant(scale)}², drizzle ${frames}; noise is correlated between output pixels, so the Poisson term is approximate`;
  }
  if (model.combine_method === "median") return `${head}2 × ${product} / π, median stack ${frames}`;
  return `${head}${product}, mean stack ${frames}`;
}

function headerGainCaption(model: GainModel): string | null {
  const effective = model.effective_gain;
  if (!finite(effective)) return model.note;
  const scaled = model.combine_scaled ? scaledGainCaption(model, effective) : null;
  if (scaled) return scaled;
  const from = model.source ? ` from ${model.source}` : "";
  const base = `gain ${formatSignificant(effective)} e-/ADU${from}`;
  return model.note ? `${base}; ${model.note}` : base;
}

function errPlaneCaption(model: GainModel): string {
  const fallback = model.fallback_gain;
  return finite(fallback)
    ? `${ERR_PLANE_CAPTION} (header gain ${formatSignificant(fallback)} e-/ADU used only where ERR is not finite)`
    : ERR_PLANE_CAPTION;
}

function unavailableCaption(model: GainModel): string | null {
  const parts: string[] = [];
  if (model.note) parts.push(model.note);
  if (finite(model.gain_card) && model.source === null) {
    parts.push(
      `GAIN=${formatSignificant(model.gain_card)} present (camera gain setting on ZWO/QHY CMOS; e-/ADU on IRAF/NOAO/LCO pipelines) — not applied; enter the e-/ADU value if that is what it is`,
    );
  }
  return parts.length > 0 ? parts.join("; ") : null;
}

function modelCaption(model: GainModel): string | null {
  if (model.poisson_route === "header_gain") return headerGainCaption(model);
  if (model.poisson_route === "err_plane") return errPlaneCaption(model);
  return unavailableCaption(model);
}

export function gainCaption(model: GainModel | null, fieldText: string): string | null {
  if (!model) return null;
  const caption = modelCaption(model);
  if (caption === null) return null;
  const carriedOver = fieldText !== "" && fieldText !== gainPrefillText(model);
  return carriedOver ? `field value ${fieldText} differs from the header model — ${caption}` : caption;
}
