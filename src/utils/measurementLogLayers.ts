import type { PixelTableResult } from "../shared/types/analysis";
import type { PixelTableLayers } from "../shared/types/layers";
import type { PlaneInfo } from "../shared/types/fits.types";
import { pixelEntry, type MeasurementLogDraft, type MeasurementProvenance } from "./measurementLog";
import { layerNotes } from "./pixelTable";

export type LayerChange =
  | { role: "err" | "dq"; action: "assign" | "switch_off" | "automatic" }
  | { role: "err"; action: "sdss_recipe" }
  | { role: "exclusion"; action: "choose" | "default"; mask: number | null };

export function pixelLayersEntry(prov: MeasurementProvenance, result: PixelTableResult & PixelTableLayers): MeasurementLogDraft {
  const entry = pixelEntry(prov, result);
  return {
    ...entry,
    params: { ...entry.params, err_source: result.err_source ?? null, dq_source: result.dq_source ?? null },
    notes: [...entry.notes, ...layerNotes(result).gaps],
  };
}

export function layerAssignmentEntry(prov: MeasurementProvenance, change: LayerChange, plane: PlaneInfo): MeasurementLogDraft {
  return {
    kind: "layer_assignment",
    file: prov.file,
    image: prov.image,
    dq: "reported",
    unit: null,
    source: JSON.stringify({ kind: "layers", role: change.role, action: change.action }),
    params: { role: change.role, action: change.action, exclusion_mask: change.role === "exclusion" ? change.mask : null },
    values: {
      err_source: plane.err_origin?.source ?? null,
      err_origin: plane.err_origin?.origin ?? null,
      err_ref: plane.err_ref,
      err_unavailable: plane.err_unavailable ?? null,
      dq_source: plane.dq_origin ?? null,
      dq_origin: plane.dq_origin_kind ?? null,
      dq_ref: plane.dq_ref,
      dq_unavailable: plane.dq_unavailable ?? null,
      sdss_gain: plane.err_recipe?.gain ?? null,
      sdss_dark_variance: plane.err_recipe?.dark_variance ?? null,
    },
    notes: [...(plane.pairing_notes ?? []), ...(plane.warnings ?? [])],
  };
}
