import type { OverlayLayer } from "../../utils/overlayStore";
import { isFootprintLayer } from "./painters/footprintPainter";

export function overlayDataAttributes(layers: readonly OverlayLayer[]): Record<string, string> {
  const visible = layers.filter((l) => l.visible);
  const attrs: Record<string, string> = { "data-testid": "viewer-overlay", "data-layers": visible.map((l) => l.id).join(" ") };
  const footprints = visible.find(isFootprintLayer);
  if (footprints) {
    attrs["data-footprints"] = String(footprints.footprints);
    if (footprints.highlight !== null) attrs["data-highlight"] = footprints.highlight;
  }
  return attrs;
}
