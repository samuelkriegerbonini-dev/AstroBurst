import { withPreview } from "../infrastructure/tauri";
import type { HdrConfig, LheConfig, LocalContrastResult } from "../shared/types/localContrast";
import type { CompositeChainCall, CompositeLocalContrastResult } from "../shared/types/compositeChain";
import { CHAIN_PREVIEWS, chainArgs } from "./compositeChain";
import type { HistoryScope, OpSpec } from "../shared/types/ops";
import { recordOp } from "./ops";
import type { RecordedResult } from "./processing";

export function lheConfigOf(op: OpSpec): LheConfig | null {
  if (op.op !== "lhe") return null;
  return { kernelRadius: op.kernelRadius, contrastLimit: op.contrastLimit, amount: op.amount, histBits: op.histBits, circular: op.circular };
}

export function hdrmtConfigOf(op: OpSpec): HdrConfig | null {
  if (op.op !== "hdrmt") return null;
  return {
    layers: op.layers,
    iterations: op.iterations,
    overdrive: op.overdrive,
    inverted: op.inverted,
    toLightness: op.toLightness,
    deringing: op.deringing,
    deringingAmount: op.deringingAmount,
  };
}

export async function recordLhe(scope: HistoryScope, path: string, config: LheConfig): Promise<RecordedResult<LocalContrastResult>> {
  return (await recordOp(scope, path, { op: "lhe", v: 1, ...config })) as RecordedResult<LocalContrastResult>;
}

export async function recordHdrmt(scope: HistoryScope, path: string, config: HdrConfig): Promise<RecordedResult<LocalContrastResult>> {
  return (await recordOp(scope, path, { op: "hdrmt", v: 1, ...config })) as RecordedResult<LocalContrastResult>;
}

export function applyLhe(
  path: string,
  outputDir: string | undefined,
  config: LheConfig,
): Promise<LocalContrastResult> {
  return withPreview<LocalContrastResult>("lhe_cmd", outputDir, { path, config });
}

export function applyLheComposite(outputDir: string | undefined, config: LheConfig): Promise<LocalContrastResult>;
export function applyLheComposite(
  outputDir: string | undefined,
  config: LheConfig,
  chain: CompositeChainCall,
): Promise<CompositeLocalContrastResult>;
export function applyLheComposite(
  outputDir: string | undefined,
  config: LheConfig,
  chain?: CompositeChainCall,
): Promise<LocalContrastResult | CompositeLocalContrastResult> {
  if (chain) {
    return withPreview<CompositeLocalContrastResult>("lhe_composite_cmd", outputDir, { config, ...chainArgs(chain) }, CHAIN_PREVIEWS);
  }
  return withPreview<LocalContrastResult>("lhe_composite_cmd", outputDir, { config });
}

export function applyHdrmt(
  path: string,
  outputDir: string | undefined,
  config: HdrConfig,
): Promise<LocalContrastResult> {
  return withPreview<LocalContrastResult>("hdrmt_cmd", outputDir, { path, config });
}

export function applyHdrmtComposite(outputDir: string | undefined, config: HdrConfig): Promise<LocalContrastResult>;
export function applyHdrmtComposite(
  outputDir: string | undefined,
  config: HdrConfig,
  chain: CompositeChainCall,
): Promise<CompositeLocalContrastResult>;
export function applyHdrmtComposite(
  outputDir: string | undefined,
  config: HdrConfig,
  chain?: CompositeChainCall,
): Promise<LocalContrastResult | CompositeLocalContrastResult> {
  if (chain) {
    return withPreview<CompositeLocalContrastResult>("hdrmt_composite_cmd", outputDir, { config, ...chainArgs(chain) }, CHAIN_PREVIEWS);
  }
  return withPreview<LocalContrastResult>("hdrmt_composite_cmd", outputDir, { config });
}
