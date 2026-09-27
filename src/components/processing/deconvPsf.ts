import type { PsfSource } from "../../shared/types/compositeChain";

export const PSF_TAB_KERNEL_HINT = "Uses the kernel from the PSF tab";
export const ESTIMATED_PSF_HINT = "Estimates the PSF from this image";

const PSF_SOURCE_LABELS: Record<PsfSource, string> = {
  gaussian: "Gaussian",
  estimated: "Estimated",
  provided: "PSF tab",
};

export function empiricalPsfHint(kernel: number[][] | null | undefined): string {
  return kernel ? PSF_TAB_KERNEL_HINT : ESTIMATED_PSF_HINT;
}

export function requestedPsfKernel(useEmpiricalPsf: boolean, kernel: number[][] | null | undefined): number[][] | null {
  return useEmpiricalPsf && kernel ? kernel : null;
}

export function psfSourceLabel(source: PsfSource | undefined): string | null {
  return source ? PSF_SOURCE_LABELS[source] : null;
}
