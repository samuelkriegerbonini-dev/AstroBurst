import { Fragment, memo, useCallback, useEffect, useMemo, useState } from "react";
import { CornerDownRight, Download, GitBranch, History, Pencil, RefreshCw, Redo2, RotateCcw, Trash2, Undo2 } from "lucide-react";
import { ErrorAlert } from "../ui";
import { CONFIRM_DELETE_LABEL, useArmedConfirm } from "../../hooks/useArmedConfirm";
import { loadHistory, useHistoryController, useHistoryRender, useHistorySnapshot } from "../../hooks/useHistory";
import { estimateNextNode } from "../../services/ops";
import type { HistoryDoc, HistoryNode, NodeEstimate } from "../../shared/types/ops";
import { activePath, branchMarkers, nodeFile, opLabel, opSummary, redoTail } from "../../utils/historyModel";
import { sectionForOp } from "../../utils/historyRerunStore";
import type { HistorySnapshot } from "../../utils/historyStore";
import { useFileContext } from "../../context/PreviewContext";
import { formatDecimalBytes } from "../../utils/format";

export interface HistoryPanelViewProps {
  snapshot: HistorySnapshot;
  estimate: NodeEstimate | null;
  fileName: string;
  onGoTo: (node: string | null) => void;
  onRerun: (node: HistoryNode, downstream: boolean) => void;
  onDelete: (node: string) => void;
  onUndo: () => void;
  onRedo: () => void;
  onExport: () => void;
  onRefresh: () => void;
}

const ICON_BUTTON = "p-1 rounded text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800/60 disabled:opacity-30 disabled:cursor-not-allowed";
const HEADER_BUTTON = "flex items-center gap-1 text-[10px] px-2 py-0.5 rounded text-zinc-400 hover:text-zinc-200 disabled:opacity-30 disabled:cursor-not-allowed";

function statusOf(node: HistoryNode): string | null {
  if (node.present) return null;
  return node.replayable === false ? "files removed · cannot be rebuilt" : "evicted · replayable";
}

interface RowActions {
  onGoTo: (node: string | null) => void;
  onRerun: (node: HistoryNode, downstream: boolean) => void;
  armedKey: string | null;
  pressDelete: (node: string) => void;
  blurDelete: (node: string) => void;
}

interface NodeRowProps {
  node: HistoryNode;
  current: boolean;
  undone: boolean;
  nested: boolean;
  hasDownstream: boolean;
  actions: RowActions;
}

function NodeRow({ node, current, undone, nested, hasDownstream, actions }: NodeRowProps) {
  const status = statusOf(node);
  const rerunnable = sectionForOp(node.op) !== null;
  const armed = actions.armedKey === node.id;
  return (
    <li
      data-history-row={node.id}
      aria-current={current ? "step" : undefined}
      data-undone={undone ? "true" : undefined}
      className={`group flex items-start gap-2 px-2 py-1.5 rounded ${current ? "bg-amber-500/10 ring-1 ring-amber-500/30" : "hover:bg-zinc-800/40"} ${undone ? "opacity-50" : ""} ${nested ? "ml-4" : ""}`}
    >
      <button type="button" className="flex-1 min-w-0 text-left" onClick={() => actions.onGoTo(node.id)} aria-label={`Go to ${node.id}`} title={`Show ${opLabel(node.op)} (${node.id})`}>
        <span className="flex items-center gap-1.5">
          <span className="font-mono text-[9px] text-zinc-600">{node.id}</span>
          <span className="text-[11px] text-zinc-200 truncate">{opLabel(node.op)}</span>
        </span>
        <span className="block text-[10px] font-mono text-zinc-500 truncate">{opSummary(node.op)}</span>
        {status && <span className="block text-[9px] text-amber-400/80">{status}</span>}
      </button>
      <span className="flex items-center gap-0.5 shrink-0">
        {rerunnable && (
          <button type="button" className={ICON_BUTTON} onClick={() => actions.onRerun(node, false)} aria-label={`Re-run ${node.id} with other settings`} title="Re-run with… (opens the step pre-filled)">
            <Pencil size={11} />
          </button>
        )}
        {rerunnable && hasDownstream && (
          <button type="button" className={ICON_BUTTON} onClick={() => actions.onRerun(node, true)} aria-label={`Re-run ${node.id} and re-apply the steps after it`} title="Re-run downstream: change this step, then re-apply the later steps on a new branch">
            <CornerDownRight size={11} />
          </button>
        )}
        <button
          type="button"
          className={`${ICON_BUTTON} ${armed ? "text-red-300" : ""}`}
          onClick={() => actions.pressDelete(node.id)}
          onBlur={() => actions.blurDelete(node.id)}
          aria-label={`Delete ${node.id} and the steps after it`}
          title={armed ? CONFIRM_DELETE_LABEL : "Delete branch: removes this step, the steps after it and their files"}
        >
          {armed ? <span className="text-[9px]">{CONFIRM_DELETE_LABEL}</span> : <Trash2 size={11} />}
        </button>
      </span>
    </li>
  );
}

function canRedo(doc: HistoryDoc): boolean {
  return doc.nodes.some((n) => n.parent === doc.head);
}

function HistoryList({ doc, actions }: { doc: HistoryDoc; actions: RowActions }) {
  const done = activePath(doc);
  const undone = redoTail(doc);
  const shown = [...done, ...undone];
  const markers = new Map(branchMarkers(doc).map((m) => [m.at, m.branches]));
  const rootBranches = markers.get(null) ?? [];
  const forkLine = (at: string | null) => {
    const branches = markers.get(at);
    if (!branches) return null;
    const count = branches.length;
    return (
      <li className="ml-4">
        <details>
          <summary data-branch-marker={at ?? "root"} className="flex items-center gap-1 text-[10px] text-violet-300/80 cursor-pointer select-none">
            <GitBranch size={10} />
            {count === 1 ? "1 other branch" : `${count} other branches`}
          </summary>
          <ol className="flex flex-col gap-0.5 mt-0.5">
            {branches.flatMap((branch) =>
              branch.map((node, i) => (
                <NodeRow key={node.id} node={node} current={false} undone={false} nested hasDownstream={i < branch.length - 1} actions={actions} />
              )),
            )}
          </ol>
        </details>
      </li>
    );
  };
  return (
    <ol className="flex flex-col gap-0.5">
      <li
        data-history-row="root"
        aria-current={doc.head === null ? "step" : undefined}
        className={`flex items-center gap-2 px-2 py-1.5 rounded ${doc.head === null ? "bg-amber-500/10 ring-1 ring-amber-500/30" : "hover:bg-zinc-800/40"}`}
      >
        <button type="button" className="flex-1 text-left text-[11px] text-zinc-300" onClick={() => actions.onGoTo(null)} aria-label="Go to the original image">
          Original
        </button>
      </li>
      {rootBranches.length > 0 && forkLine(null)}
      {shown.map((node, i) => (
        <Fragment key={node.id}>
          <NodeRow node={node} current={node.id === doc.head} undone={i >= done.length} nested={false} hasDownstream={i < shown.length - 1} actions={actions} />
          {forkLine(node.id)}
        </Fragment>
      ))}
    </ol>
  );
}

function StorageLine({ estimate }: { estimate: NodeEstimate | null }) {
  if (!estimate) return null;
  return (
    <div className="text-[10px] font-mono text-zinc-500" title={`Every step writes a ${estimate.dimensions[0]}×${estimate.dimensions[1]}×${estimate.planes} float32 node under ${estimate.history_root}`}>
      Next step ≈ {formatDecimalBytes(estimate.next_node_bytes)} · history/ {formatDecimalBytes(estimate.history_bytes)} · {estimate.history_files} files
    </div>
  );
}

export function HistoryPanelView({ snapshot, estimate, fileName, onGoTo, onRerun, onDelete, onUndo, onRedo, onExport, onRefresh }: HistoryPanelViewProps) {
  const { armedKey, press, blur } = useArmedConfirm();
  const state = snapshot.state;
  const readOnly = state?.read_only ?? null;
  const doc = readOnly === null ? state?.doc ?? null : null;
  const actions: RowActions = useMemo(
    () => ({ onGoTo, onRerun, armedKey, pressDelete: (node: string) => press(node, () => onDelete(node)), blurDelete: blur }),
    [onGoTo, onRerun, armedKey, press, blur, onDelete],
  );
  if (!snapshot.fileKey) {
    return <div className="p-4 text-xs text-zinc-500 italic">Select an image to see its processing history.</div>;
  }
  const hasNodes = (doc?.nodes.length ?? 0) > 0;
  return (
    <div className="flex flex-col gap-2 p-3">
      <div className="flex items-center gap-2">
        <History size={14} className="text-amber-400" />
        <span className="text-sm font-semibold text-zinc-200 tracking-wide">History</span>
        <span className="text-[10px] text-zinc-500 truncate" title={fileName}>{fileName}</span>
        <span className="ml-auto flex items-center gap-1">
          <button type="button" data-history-undo="" className={HEADER_BUTTON} onClick={onUndo} disabled={!doc || doc.head === null} title="Undo (Ctrl+Z)">
            <Undo2 size={11} />
            Undo
          </button>
          <button type="button" data-history-redo="" className={HEADER_BUTTON} onClick={onRedo} disabled={!doc || !canRedo(doc)} title="Redo (Ctrl+Shift+Z or Ctrl+Y)">
            <Redo2 size={11} />
            Redo
          </button>
          <button type="button" className={ICON_BUTTON} onClick={onRefresh} aria-label="Reload the history from disk" title="Reload the history from disk">
            <RefreshCw size={11} />
          </button>
        </span>
      </div>
      <StorageLine estimate={estimate} />
      {readOnly && (
        <div role="note" className="text-[10px] text-amber-300/90 bg-amber-900/15 border border-amber-700/25 rounded px-2 py-1.5">
          {readOnly}
        </div>
      )}
      {doc && state?.fingerprint_matches === false && (
        <div role="note" className="text-[10px] text-amber-300/90 bg-amber-900/15 border border-amber-700/25 rounded px-2 py-1.5">
          The source file changed since this history was recorded, so the original is shown. The steps below were computed from the old file: they can still be shown, but evicted ones cannot be rebuilt. The next step you run records from the new file.
        </div>
      )}
      <ErrorAlert message={snapshot.error} />
      {doc && hasNodes ? (
        <HistoryList doc={doc} actions={actions} />
      ) : (
        !readOnly && <div className="text-xs text-zinc-500 italic px-1">No processing steps yet. Run a step in Processing to start the history.</div>
      )}
      {hasNodes && (
        <p className="text-[9px] text-zinc-600 px-1">
          Steps keep their order along a branch: running an earlier step starts a new branch, and the old one stays here.
        </p>
      )}
      <div className="flex items-center gap-2">
        <button type="button" className={HEADER_BUTTON} onClick={onExport} disabled={!hasNodes} style={{ border: "1px solid var(--ab-border)" }}>
          <Download size={11} />
          Export history JSON
        </button>
        {doc && doc.head !== null && (
          <button type="button" className={HEADER_BUTTON} onClick={() => onGoTo(null)} style={{ border: "1px solid var(--ab-border)" }} title="Show the original image; the history keeps every step">
            <RotateCcw size={11} />
            Revert to original
          </button>
        )}
      </div>
    </div>
  );
}

function headInputPath(snapshot: HistorySnapshot): string | null {
  const state = snapshot.state;
  const doc = state?.doc;
  if (!state) return null;
  const head = doc && state.read_only === null ? doc.nodes.find((n) => n.id === doc.head) : undefined;
  return head ? nodeFile(state.history_dir, head.output.path) : state.root_path;
}

function exportName(fileName: string): string {
  const stem = fileName.replace(/\.(fits?|fts|asdf|xisf|tiff?)$/i, "") || "image";
  return `${stem}_history.json`;
}

function HistoryPanel() {
  const snapshot = useHistorySnapshot();
  const controller = useHistoryController();
  const render = useHistoryRender();
  const { file } = useFileContext();
  const fileName = file?.name ?? "";
  const [estimate, setEstimate] = useState<NodeEstimate | null>(null);
  const [exportError, setExportError] = useState<string | null>(null);
  const inputPath = headInputPath(snapshot);
  const nodeCount = snapshot.state?.doc?.nodes.length ?? 0;
  const target = controller.target;

  useEffect(() => {
    if (!inputPath) {
      setEstimate(null);
      return;
    }
    let live = true;
    estimateNextNode(inputPath)
      .then((next) => {
        if (live) setEstimate(next);
      })
      .catch(() => {
        if (live) setEstimate(null);
      });
    return () => {
      live = false;
    };
  }, [inputPath, nodeCount]);

  const refresh = useCallback(() => {
    if (target) void loadHistory(target, render);
  }, [target, render]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const exportJson = useCallback(async () => {
    const doc = snapshot.state?.doc;
    if (!doc) return;
    setExportError(null);
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const path = await save({ defaultPath: exportName(fileName), filters: [{ name: "JSON", extensions: ["json"] }], title: "Export the processing history" });
      if (!path) return;
      const { writeTextFile } = await import("@tauri-apps/plugin-fs");
      await writeTextFile(path, `${JSON.stringify(doc, null, 2)}\n`);
    } catch (e: unknown) {
      setExportError(e instanceof Error ? e.message : String(e));
    }
  }, [snapshot.state?.doc, fileName]);

  const shown = exportError ? { ...snapshot, error: exportError } : snapshot;
  return (
    <HistoryPanelView
      snapshot={shown}
      estimate={estimate}
      fileName={fileName}
      onGoTo={(node) => void controller.goTo(node)}
      onRerun={(node, downstream) => void controller.rerunWith(node, downstream)}
      onDelete={(node) => void controller.deleteBranch(node)}
      onUndo={() => void controller.undo()}
      onRedo={() => void controller.redo()}
      onExport={() => void exportJson()}
      onRefresh={refresh}
    />
  );
}

export default memo(HistoryPanel);
