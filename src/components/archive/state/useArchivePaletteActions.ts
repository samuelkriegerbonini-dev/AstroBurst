import { useMemo } from "react";
import type { PaletteAction } from "../../CommandPalette";
import type { AppMode } from "../../../hooks/useAppMode";
import type { AroundTarget } from "../logic/aroundQuery";
import { archiveController, type ArchiveController } from "./archiveController";
import { useAroundTarget, type WorkspaceFileRef } from "./useAroundTarget";

export interface ArchivePaletteInput {
  mode: AppMode;
  setMode: (mode: AppMode) => void;
  around: AroundTarget | null;
  controller: Pick<ArchiveController, "searchAround" | "openRomanAccess" | "toggleView" | "clearCache">;
}

function archiveHint(): string {
  const platform = typeof navigator === "undefined" ? "" : navigator.platform ?? "";
  return platform.includes("Mac") ? "⌘2" : "Ctrl+2";
}

export function buildArchivePaletteActions({ mode, setMode, around, controller }: ArchivePaletteInput): PaletteAction[] {
  const actions: PaletteAction[] = [
    { id: "mast-browse", label: "Browse MAST archive", hint: archiveHint(), keywords: ["mast", "archive", "jwst", "hst", "search"], run: () => setMode("archive") },
  ];
  if (around) {
    actions.push({
      id: "mast-around",
      label: "Search MAST around this image",
      keywords: ["mast", "archive", "around", "wcs"],
      run: () => {
        setMode("archive");
        void controller.searchAround(around);
      },
    });
  }
  actions.push({
    id: "mast-token",
    label: "MAST token",
    keywords: ["mast", "roman", "token", "key"],
    run: () => {
      setMode("archive");
      controller.openRomanAccess();
    },
  });
  if (mode === "archive") {
    actions.push({ id: "mast-view", label: "Toggle gallery or table", hint: "V", keywords: ["mast", "gallery", "table", "view"], run: () => controller.toggleView() });
  }
  actions.push({ id: "mast-cache-clear", label: "Clear MAST cache", keywords: ["mast", "cache", "clear"], run: () => void controller.clearCache() });
  return actions;
}

export function useArchivePaletteActions({
  mode,
  setMode,
  workspaceFile,
}: {
  mode: AppMode;
  setMode: (mode: AppMode) => void;
  workspaceFile: WorkspaceFileRef | null;
}): PaletteAction[] {
  const around = useAroundTarget(workspaceFile);
  return useMemo(() => buildArchivePaletteActions({ mode, setMode, around, controller: archiveController }), [mode, setMode, around]);
}
