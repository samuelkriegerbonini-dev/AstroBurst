import { deconvolveOptionsOf, type DeconvolveSettings } from "../../services/processing";
import type { PsfSource } from "../../shared/types/compositeChain";
import type { OpSpec, PsfSpec } from "../../shared/types/ops";

export type PsfChoice = "gaussian" | "measured" | "estimate";
type MeasuredKernel = number[][] | null | undefined;

export interface DeconvPanelSettings extends Omit<DeconvolveSettings, "useEmpiricalPsf"> {
  psf: PsfChoice;
}

export interface PsfRequest {
  useEmpiricalPsf: boolean;
  psfKernel: number[][] | null;
}

export const PSF_CHOICES: readonly PsfChoice[] = ["gaussian", "measured", "estimate"];

export const GAUSSIAN_PSF_HINT = "Builds a Gaussian PSF from PSF Sigma and PSF Size";
export const PSF_TAB_KERNEL_HINT = "Uses the kernel from the PSF tab";
export const ESTIMATED_PSF_HINT = "Estimates the PSF from this image";
export const MEASURED_PSF_CHIP = "A measured PSF is ready: use it";
export const NO_MEASURED_PSF_REASON = "No measured PSF yet: run Estimate PSF in the PSF tab to enable Measured.";
export const MEASURED_PSF_FALLBACK =
  "No measured PSF is loaded, so the PSF is estimated from this image. Run Estimate PSF in the PSF tab to use a measured one.";

const PSF_CHOICE_HINTS: Record<PsfChoice, string> = {
  gaussian: GAUSSIAN_PSF_HINT,
  measured: PSF_TAB_KERNEL_HINT,
  estimate: ESTIMATED_PSF_HINT,
};

const PSF_CHOICE_OF_KIND: Record<PsfSpec["kind"], PsfChoice> = {
  gaussian: "gaussian",
  kernel: "measured",
  empirical: "estimate",
};

const PSF_SOURCE_LABELS: Record<PsfSource, string> = {
  gaussian: "Gaussian",
  estimated: "Estimated",
  provided: "PSF tab",
};

function hasKernel(kernel: MeasuredKernel): kernel is number[][] {
  return !!kernel && kernel.length > 0;
}

export function psfChoiceAvailable(choice: PsfChoice, kernel: MeasuredKernel): boolean {
  return choice !== "measured" || hasKernel(kernel);
}

export function effectivePsfChoice(selected: PsfChoice, kernel: MeasuredKernel): PsfChoice {
  return psfChoiceAvailable(selected, kernel) ? selected : "estimate";
}

export function psfChoiceLabel(choice: PsfChoice, kernel: MeasuredKernel): string {
  if (choice === "gaussian") return "Gaussian σ";
  if (choice === "estimate") return "Estimate from this image";
  return hasKernel(kernel) ? `Measured (PSF tab, ${kernel.length}×${kernel[0].length})` : "Measured (PSF tab)";
}

export function psfChoiceHint(choice: PsfChoice): string {
  return PSF_CHOICE_HINTS[choice];
}

export function measuredPsfNote(selected: PsfChoice, kernel: MeasuredKernel): string | null {
  if (hasKernel(kernel)) return null;
  return selected === "measured" ? MEASURED_PSF_FALLBACK : NO_MEASURED_PSF_REASON;
}

export function offersMeasuredPsf(selected: PsfChoice, kernel: MeasuredKernel): boolean {
  return selected === "gaussian" && hasKernel(kernel);
}

export function psfRequest(selected: PsfChoice, kernel: MeasuredKernel): PsfRequest {
  if (selected === "gaussian") return { useEmpiricalPsf: false, psfKernel: null };
  return { useEmpiricalPsf: true, psfKernel: selected === "measured" && hasKernel(kernel) ? kernel : null };
}

export function deconvPanelSettingsOf(op: OpSpec): DeconvPanelSettings | null {
  const settings = deconvolveOptionsOf(op);
  if (settings === null || op.op !== "rl_deconv") return null;
  const { useEmpiricalPsf: _impliedByKind, ...values } = settings;
  return { ...values, psf: PSF_CHOICE_OF_KIND[op.psf.kind] };
}

export function psfSourceLabel(source: PsfSource | undefined): string | null {
  return source ? PSF_SOURCE_LABELS[source] : null;
}
