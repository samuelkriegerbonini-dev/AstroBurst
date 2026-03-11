import { useState, useCallback } from "react";

interface ChannelStats {
  label: string;
  lights_input: number;
  lights_after_rejection: number[];
  mean: number;
  stddev: number;
}

interface PipelineStats {
  darks_combined: number;
  flats_combined: number;
  bias_combined: number;
  channels: ChannelStats[];
}

interface ChannelPreview {
  label: string;
  pixels_b64: string;
  width: number;
  height: number;
}

interface PipelineResponse {
  stats: PipelineStats;
  channel_previews: ChannelPreview[];
  rgb_preview: string | null;
}

interface ChannelFilesInput {
  label: string;
  paths: string[];
}

interface PipelineConfig {
  sigma_low?: number;
  sigma_high?: number;
  normalize?: boolean;
}

const safeInvoke = async (cmd: string, args: Record<string, any> = {}) => {
  if (!(window as any).__TAURI_INTERNALS__) throw new Error("Requires Tauri");
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke(cmd, args);
};

export function useCalibrationPipeline() {
  const [result, setResult] = useState<PipelineResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [progress, setProgress] = useState("");

  const run = useCallback(
    async (
      channels: ChannelFilesInput[],
      darkPaths: string[],
      flatPaths: string[],
      biasPaths: string[],
      config?: PipelineConfig
    ) => {
      setLoading(true);
      setError(null);
      setProgress("Building calibration masters...");
      try {
        const res = await safeInvoke("run_pipeline_cmd", {
          request: {
            channels,
            dark_paths: darkPaths,
            flat_paths: flatPaths,
            bias_paths: biasPaths,
            sigma_low: config?.sigma_low,
            sigma_high: config?.sigma_high,
            normalize: config?.normalize,
          },
        }) as PipelineResponse;
        setResult(res);
        setProgress("");
        return res;
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        setError(msg);
        setProgress("");
        return null;
      } finally {
        setLoading(false);
      }
    },
    []
  );

  return { result, loading, error, progress, run };
}

export type {
  PipelineResponse,
  PipelineStats,
  ChannelStats,
  ChannelPreview,
  ChannelFilesInput,
};
