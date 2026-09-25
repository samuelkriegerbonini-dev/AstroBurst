import { typedInvoke } from "../infrastructure/tauri";
import { parseDqMaskBuffer } from "../infrastructure/tauri/parsers";
import type { ColormapLutResult, ColormapName, DisplaySettings, ScaleLimits } from "../shared/types/display";
import type { PixelProbeResult } from "../shared/types/analysis";
import type { DqFlagTable, DqMaskData } from "../shared/types/dq";
import { withNodataEntry } from "../utils/displayTransfer";

export type { ColormapLutResult, ColormapName, DisplaySettings, ScaleLimits } from "../shared/types/display";
export type { PixelProbeResult } from "../shared/types/analysis";

export function computeScaleLimits(path: string, s: DisplaySettings): Promise<ScaleLimits> {
  return typedInvoke<ScaleLimits>("compute_scale_limits_cmd", {
    path,
    algorithm: s.limits,
    vmin: s.userLo,
    vmax: s.userHi,
    percentile: [s.percentileLow, s.percentileHigh],
    zscaleContrast: s.zscaleContrast,
    symmetric: s.symmetric,
    centre: s.centre,
  });
}

const lutCache = new Map<ColormapName, Promise<Uint8Array>>();

export function getColormapLut(name: ColormapName): Promise<Uint8Array> {
  const cached = lutCache.get(name);
  if (cached) return cached;
  const pending = typedInvoke<ColormapLutResult>("get_colormap_lut_cmd", { name })
    .then((res) => withNodataEntry(res.rgba, res.nodata))
    .catch((err) => {
      lutCache.delete(name);
      throw err;
    });
  lutCache.set(name, pending);
  return pending;
}

export function probePixel(path: string, x: number, y: number, boxSize = 5): Promise<PixelProbeResult> {
  return typedInvoke<PixelProbeResult>("probe_pixel_cmd", { path, x, y, boxSize });
}

export function getDqFlagTable(path: string): Promise<DqFlagTable> {
  return typedInvoke<DqFlagTable>("get_dq_flag_table_cmd", { path });
}

export async function getDqMaskPreview(path: string, mask: number, maxDim = 2048): Promise<DqMaskData> {
  const raw = await typedInvoke<ArrayBuffer>("get_dq_mask_preview", { path, mask, maxDim });
  return parseDqMaskBuffer(raw);
}
