import { archiveShortcuts } from "../components/archive/logic/shortcuts";

export interface ShortcutRow {
  keys: string;
  desc: string;
}

export interface ShortcutGroup {
  id: string;
  title: string | null;
  rows: readonly ShortcutRow[];
}

export function modifierKey(mac: boolean): string {
  return mac ? "⌘" : "Ctrl+";
}

export function redoKeys(mac: boolean): string {
  return mac ? "⇧⌘Z" : "Ctrl+Shift+Z";
}

export function shortcutSheet(mac: boolean): readonly ShortcutGroup[] {
  const mod = modifierKey(mac);
  return [
    {
      id: "general",
      title: null,
      rows: [
        { keys: `${mod}O`, desc: "Open FITS files" },
        { keys: `${mod}K`, desc: "Search everywhere" },
        { keys: "Shift Shift", desc: "Search everywhere" },
        { keys: `${mod}1 / ${mod}2`, desc: "Switch to Workspace / Archive" },
        { keys: "↑ / ↓", desc: "Navigate file list (when focused)" },
        { keys: "Enter", desc: "Open the selected spectrum in the Spectra tool" },
        { keys: "Shift+F10 / right-click (sidebar tool)", desc: "Move or reorder the tool, reset the layout, or open All tools" },
        { keys: `${mod}K, then All tools`, desc: "Open any tool or choose the sidebar shortcuts" },
        { keys: "?", desc: "Toggle this help" },
        { keys: "Esc", desc: "Close overlays" },
      ],
    },
    {
      id: "history",
      title: "History",
      rows: [
        { keys: `${mod}Z`, desc: "Undo processing step (single files)" },
        { keys: mac ? redoKeys(true) : `${redoKeys(false)} / Ctrl+Y`, desc: "Redo processing step (single files)" },
      ],
    },
    {
      id: "cube-regions",
      title: "Cube and regions",
      rows: [
        { keys: "← / → (spectrum plot)", desc: "Previous / next channel of the cube or ramp" },
        { keys: "← / → (focused image)", desc: "Previous / next frame of the cube or ramp on screen" },
        { keys: "Shift+← / →", desc: "Step 10 channels or frames" },
        { keys: "Double-click (spectrum plot)", desc: "Jump to the channel under the cursor" },
        { keys: "Enter / double-click (polygon tool)", desc: "Close the polygon being drawn (3 vertices or more)" },
        { keys: "Delete / Backspace (image focused)", desc: "Delete the selected region" },
      ],
    },
    { id: "archive", title: "Archive", rows: archiveShortcuts(mac) },
  ];
}
