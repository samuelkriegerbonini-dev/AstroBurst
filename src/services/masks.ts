import { announceCommandOutputs, typedInvoke } from "../infrastructure/tauri";
import { parseMaskPreviewBuffer } from "../infrastructure/tauri/parsers";
import type { HistoryScope } from "../shared/types/ops";
import type {
  MaskCreateOptions,
  MaskCreateResult,
  MaskDeleteResult,
  MaskExportKind,
  MaskExportResult,
  MaskPreview,
  MaskSpec,
} from "../shared/types/masks";
import { COMPOSITE_RUN_KEY } from "../utils/compositeChain";

export interface MaskTarget extends HistoryScope {
  inputPath?: string | null;
}

export const COMPOSITE_MASK_TARGET: Readonly<MaskTarget> = Object.freeze({ imageKey: COMPOSITE_RUN_KEY, basePath: null, inputPath: null });

function scopeArgs({ imageKey, basePath }: HistoryScope) {
  return { imageKey, basePath };
}

function inputArgs(target: MaskTarget) {
  return { ...scopeArgs(target), inputPath: target.inputPath ?? null };
}

export function createMask(target: MaskTarget, recipe: MaskSpec, options: MaskCreateOptions = {}): Promise<MaskCreateResult> {
  return typedInvoke<MaskCreateResult>("mask_create_cmd", {
    ...inputArgs(target),
    recipe,
    shapesSystem: options.shapesSystem ?? null,
    mods: options.mods ?? null,
  });
}

export async function previewMask(target: MaskTarget, maskId: string, maxDim?: number): Promise<MaskPreview> {
  const raw = await typedInvoke<ArrayBuffer>("mask_preview_cmd", { ...inputArgs(target), maskId, maxDim: maxDim ?? null });
  return parseMaskPreviewBuffer(raw);
}

export function deleteMask(target: HistoryScope, maskId: string): Promise<MaskDeleteResult> {
  return typedInvoke<MaskDeleteResult>("mask_delete_cmd", { ...scopeArgs(target), maskId });
}

export async function exportMask(target: MaskTarget, maskId: string, path: string, kind: MaskExportKind): Promise<MaskExportResult> {
  const result = await typedInvoke<MaskExportResult>("mask_export_cmd", { ...inputArgs(target), maskId, path, kind });
  announceCommandOutputs(result);
  return result;
}
