import type { ChainStep } from "../../shared/types/preview";
import type { CompositeChainInput, CompositeStepResult, DisplayStf } from "../../shared/types/compositeChain";

export type {
  ChannelTriple,
  CompositeBackgroundResult,
  CompositeChainCall,
  CompositeChainInput,
  CompositeDeconvolveResult,
  CompositeDisplayed,
  CompositeLocalContrastResult,
  CompositeMaskedStretchResult,
  CompositePixelMathResult,
  CompositeStepResult,
  CompositeStretchResult,
  CompositeWaveletResult,
  DisplayStf,
} from "../../shared/types/compositeChain";

export interface CompositeInputView {
  input: CompositeChainInput;
  previewUrl: string | null;
  label: string;
}

export interface CompositePanelProps {
  compositeMode: boolean;
  compositeInput: CompositeInputView | null;
  onCompositeDone: (step: ChainStep, label: string, result: CompositeStepResult, callStf: DisplayStf) => void;
  fileName: string;
}

export type CompositeNoticeProps = Pick<CompositePanelProps, "compositeMode" | "fileName">;

export const COMPOSITE_RESTARTED_NOTICE =
  "The composite changed since the last step, so the processing chain restarted from the composite on screen.";

export function unchangedFileClause(fileName: string): string {
  return fileName ? `; ${fileName} is not changed.` : ".";
}

export function compositeModeNotice(fileName: string): string {
  return `Composite mode: processes the three channels of the colour composite${unchangedFileClause(fileName)}`;
}

export function fileOnlyNotice(action: string, fileName: string): string {
  return `${action} ${fileName || "the selected file"}; the composite on screen does not change.`;
}

export function channelTriple(values: readonly number[], format: (v: number) => string): string {
  return values.map(format).join(" · ");
}
