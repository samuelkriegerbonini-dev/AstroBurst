import { createContext, useContext, useState, useEffect, useCallback, type ReactNode } from "react";

interface ChannelSuggestion {
  file_path: string;
  file_name: string;
  detection: FilterDetection | null;
}

interface FilterDetection {
  filter_name: string;
  method: string;
  confidence: number;
}

interface PaletteSuggestion {
  r_file: ChannelSuggestion | null;
  g_file: ChannelSuggestion | null;
  b_file: ChannelSuggestion | null;
  unmapped: ChannelSuggestion[];
  is_complete: boolean;
  palette_name: string;
}

interface NarrowbandState {
  narrowbandPalette: PaletteSuggestion | null;
  setNarrowbandPalette: (palette: PaletteSuggestion | null) => void;
}

const NarrowbandCtx = createContext<NarrowbandState | null>(null);

export function NarrowbandProvider({ fileId, children }: { fileId: string | undefined; children: ReactNode }) {
  const [narrowbandPalette, setNarrowbandPalette] = useState<PaletteSuggestion | null>(null);

  useEffect(() => {
    setNarrowbandPalette(null);
  }, [fileId]);

  return (
    <NarrowbandCtx.Provider value={{ narrowbandPalette, setNarrowbandPalette }}>
      {children}
    </NarrowbandCtx.Provider>
  );
}

export function useNarrowbandContext(): NarrowbandState {
  const ctx = useContext(NarrowbandCtx);
  if (!ctx) throw new Error("useNarrowbandContext must be used within NarrowbandProvider");
  return ctx;
}

export type { PaletteSuggestion, ChannelSuggestion, FilterDetection };
