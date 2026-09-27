import { withPreview } from "../infrastructure/tauri";
import type { HdrConfig, LheConfig, LocalContrastResult } from "../shared/types/localContrast";
import type { CompositeChainCall, CompositeLocalContrastResult } from "../shared/types/compositeChain";
import { CHAIN_PREVIEWS, chainArgs } from "./compositeChain";

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
