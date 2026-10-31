import type { OverlayLayer } from "../../utils/overlayStore";
import { isFootprintLayer } from "./painters/footprintPainter";
import { isMaskLayer } from "./painters/maskPainter";

export function overlayDataAttributes(layers: readonly OverlayLayer[]): Record<string, string> {
  const visible = layers.filter((l) => l.visible);
  const attrs: Record<string, string> = { "data-testid": "viewer-overlay", "data-layers": visible.map((l) => l.id).join(" ") };
  const footprints = visible.find(isFootprintLayer);
  if (footprints) {
    attrs["data-footprints"] = String(footprints.footprints);
    if (footprints.highlight !== null) attrs["data-highlight"] = footprints.highlight;
  }
  const mask = visible.find(isMaskLayer);
  if (mask) {
    attrs["data-mask"] = mask.maskId;
    attrs["data-mask-cells"] = String(mask.markedCells);
  }
  return attrs;
}
