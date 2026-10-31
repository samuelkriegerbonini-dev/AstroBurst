import { createContext, useContext } from "react";

export const OverlayLayerKeyContext = createContext<string | null>(null);

export function useOverlayLayerKey(): string | null {
  return useContext(OverlayLayerKeyContext);
}

export interface OverlayHost {
  enabled: boolean;
  regionKey: string | null;
  layerKey: string | null;
}

export function overlayHostKey({ enabled, regionKey, layerKey }: OverlayHost): string | null {
  return layerKey ?? (enabled ? regionKey : null);
}
