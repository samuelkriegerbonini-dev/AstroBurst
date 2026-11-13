import { Table } from "lucide-react";
import { tableViewerTitle } from "../../utils/previewShell";
import type { TableNote, TableRouteKind } from "../../utils/fileKinds";

interface TableViewerNoteProps {
  note: TableNote;
  onAction?: (id: string, kind: TableRouteKind) => void;
}

export default function TableViewerNote({ note, onAction }: TableViewerNoteProps) {
  const action = note.action;
  return (
    <div data-table-note="" className="ab-viewer-empty">
      <div className="flex flex-col items-center gap-3 px-6 text-center max-w-md">
        <Table size={36} strokeWidth={1} style={{ color: "var(--ab-sky)" }} />
        <span className="text-xs text-zinc-300 break-all">{tableViewerTitle(note.name)}</span>
        <span className="text-[11px] text-zinc-400">{note.text}</span>
        {note.reason && <span className="text-[11px] text-zinc-500">{note.reason}</span>}
        <span className="text-[11px] text-zinc-500">{note.hint}</span>
        {action && onAction && (
          <button
            type="button"
            title={action.title}
            onClick={() => onAction(note.id, action.kind)}
            className="text-[11px] px-3 py-1 rounded text-zinc-200 hover:text-white transition-colors"
            style={{ border: "1px solid var(--ab-border)", background: "rgba(20,184,166,0.12)" }}
          >
            {action.label}
          </button>
        )}
      </div>
    </div>
  );
}
