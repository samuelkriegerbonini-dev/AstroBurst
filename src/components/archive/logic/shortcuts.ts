export interface ArchiveShortcut {
  keys: string;
  desc: string;
}

export const ARCHIVE_SHORTCUTS: readonly ArchiveShortcut[] = [
  { keys: "Ctrl+1 / Ctrl+2", desc: "Switch to Workspace / Archive" },
  { keys: "/ or Ctrl+F", desc: "Focus the query field" },
  { keys: "Enter", desc: "Run the search from the query bar; from the results, move into the detail pane" },
  { keys: "Esc", desc: "Close the innermost layer: menu, Large view, drawer, then cancel the running search" },
  { keys: "↓ (empty query)", desc: "Show recent searches of this session" },
  { keys: "← / → (mission tabs)", desc: "Previous / next mission" },
  { keys: "F6 / Shift+F6", desc: "Cycle regions: query bar, results, detail" },
  { keys: "V", desc: "Toggle gallery or table" },
  { keys: "Arrow keys", desc: "Move between cards in the gallery" },
  { keys: "↑ / ↓ (table)", desc: "Move between rows in the table" },
  { keys: "Home / End", desc: "First / last item of the row" },
  { keys: "Ctrl+Home / Ctrl+End", desc: "First / last loaded result" },
  { keys: "PageUp / PageDown", desc: "Move one screen; the last loaded row loads the next 200" },
  { keys: "Space", desc: "Open the Large view on the focused dataset or image" },
  { keys: "Enter / Space (table header)", desc: "Sort by the column; Shift adds a secondary key" },
  { keys: "Ctrl+C", desc: "Copy the dataset id, or the focused cell" },
  { keys: "Ctrl+Shift+C", desc: "Copy the table row as TSV" },
  { keys: "T", desc: "Toggle thumbnails in the table" },
  { keys: "R", desc: "Retry the focused item or the last failed request" },
  { keys: "I", desc: "Hide or show the detail pane (the info rail in the Large view)" },
  { keys: "Alt+1 / Alt+2 / Alt+3", desc: "Detail tab: Previews / Metadata / Products" },
  { keys: "← / → (previews)", desc: "Previous / next image in the group" },
  { keys: "G", desc: "Next product group" },
  { keys: "↑ / ↓ (Large view)", desc: "Previous / next dataset, keeping the group" },
  { keys: "+ / - / 0 / 1", desc: "Large view zoom: in / out / fit / 1:1" },
];
