import { ARCHIVE_SHORTCUTS } from "../components/archive/logic/shortcuts";

export interface ShortcutRow {
  keys: string;
  desc: string;
}

export interface ShortcutGroup {
  id: string;
  title: string | null;
  rows: readonly ShortcutRow[];
}

export function shortcutSheet(mac: boolean): readonly ShortcutGroup[] {
  const mod = mac ? "⌘" : "Ctrl+";
  return [
    {
      id: "general",
      title: null,
      rows: [
        { keys: `${mod}O`, desc: "Open FITS files" },
        { keys: `${mod}K`, desc: "Search everywhere" },
        { keys: "Shift Shift", desc: "Search everywhere" },
        { keys: "↑ / ↓", desc: "Navigate file list (when focused)" },
        { keys: "Enter", desc: "Select focused file" },
        { keys: "?", desc: "Toggle this help" },
        { keys: "Esc", desc: "Close overlays" },
      ],
    },
    {
      id: "history",
      title: "History",
      rows: [
        { keys: `${mod}Z`, desc: "Undo processing step" },
        { keys: mac ? "⇧⌘Z" : "Ctrl+Shift+Z / Ctrl+Y", desc: "Redo processing step" },
      ],
    },
    {
      id: "cube-regions",
      title: "Cube and regions",
      rows: [
        { keys: "← / → (spectrum plot)", desc: "Previous / next channel of the cube or ramp" },
        { keys: "Shift+← / →", desc: "Step 10 channels" },
        { keys: "Double-click (spectrum plot)", desc: "Jump to the channel under the cursor" },
        { keys: "Enter / double-click (polygon tool)", desc: "Close the polygon being drawn (3 vertices or more)" },
      ],
    },
    { id: "viewer", title: "Viewer", rows: [] },
    { id: "archive", title: "Archive", rows: ARCHIVE_SHORTCUTS },
  ];
}
