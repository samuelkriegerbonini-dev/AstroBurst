import { getPreviewUrl, typedInvoke, withPreview } from "../infrastructure/tauri";
import type {
  DeleteBranchResult,
  HeadView,
  HistoryScope,
  HistoryState,
  KernelRef,
  NodeEstimate,
  NodeView,
  OpSpec,
  RehydrateResult,
  RerunResult,
  RunOpResult,
  StepCopyResult,
} from "../shared/types/ops";
import { historyRerunStore } from "../utils/historyRerunStore";

export interface RunOpOptions {
  psfKernel?: number[][] | null;
}

export interface RerunOptions {
  op?: OpSpec;
  replayDownstream?: boolean;
  psfKernel?: number[][] | null;
}

const RUN_PREVIEWS: [string, string][] = [
  ["png_path", "previewUrl"],
  ["model_png", "modelPreviewUrl"],
];

function scopeArgs(scope: HistoryScope): Record<string, unknown> {
  return { imageKey: scope.imageKey, basePath: scope.basePath };
}

async function withNodePreview<T extends { node: NodeView | null }>(view: T): Promise<T> {
  if (!view.node) return view;
  return { ...view, node: { ...view.node, previewUrl: await getPreviewUrl(view.node.png_path) } };
}

export function runOp(scope: HistoryScope, inputPath: string, op: OpSpec, options: RunOpOptions = {}): Promise<RunOpResult> {
  const args = { ...scopeArgs(scope), inputPath, op, psfKernel: options.psfKernel ?? null };
  return withPreview<RunOpResult>("run_op_cmd", undefined, args, RUN_PREVIEWS);
}

export async function recordOp(scope: HistoryScope, inputPath: string, op: OpSpec, options: RunOpOptions = {}): Promise<RunOpResult> {
  const armed = historyRerunStore.takeArmed(scope, op.op);
  if (!armed) return runOp(scope, inputPath, op, options);
  const { run } = await rerun(scope, armed.node, { op, replayDownstream: true, psfKernel: options.psfKernel ?? null });
  if (!run.model_png) return run;
  return { ...run, modelPreviewUrl: await getPreviewUrl(run.model_png) };
}

export function getHistory(scope: HistoryScope): Promise<HistoryState> {
  return typedInvoke<HistoryState>("history_get_cmd", scopeArgs(scope));
}

export async function setHead(scope: HistoryScope, node: string | null): Promise<HeadView> {
  return withNodePreview(await typedInvoke<HeadView>("history_set_head_cmd", { ...scopeArgs(scope), node }));
}

export async function undo(scope: HistoryScope): Promise<HeadView> {
  return withNodePreview(await typedInvoke<HeadView>("history_undo_cmd", scopeArgs(scope)));
}

export async function redo(scope: HistoryScope): Promise<HeadView> {
  return withNodePreview(await typedInvoke<HeadView>("history_redo_cmd", scopeArgs(scope)));
}

export async function rerun(scope: HistoryScope, node: string, options: RerunOptions = {}): Promise<RerunResult> {
  const args = {
    ...scopeArgs(scope),
    node,
    op: options.op ?? null,
    replayDownstream: options.replayDownstream ?? false,
    psfKernel: options.psfKernel ?? null,
  };
  const result = await withNodePreview(await typedInvoke<RerunResult>("history_rerun_cmd", args));
  if (!result.run?.png_path) return result;
  return { ...result, run: { ...result.run, previewUrl: await getPreviewUrl(result.run.png_path) } };
}

export async function deleteBranch(scope: HistoryScope, node: string): Promise<DeleteBranchResult> {
  return withNodePreview(await typedInvoke<DeleteBranchResult>("history_delete_branch_cmd", { ...scopeArgs(scope), node }));
}

export function copyStepToOutput(scope: HistoryScope, node: string, outputDir?: string): Promise<StepCopyResult> {
  return withPreview<StepCopyResult>("history_copy_to_output_cmd", outputDir, { ...scopeArgs(scope), node }, []);
}

export async function rehydrate(scope: HistoryScope, node: string): Promise<RehydrateResult> {
  return withNodePreview(await typedInvoke<RehydrateResult>("history_rehydrate_cmd", { ...scopeArgs(scope), node }));
}

export function estimateNextNode(inputPath: string): Promise<NodeEstimate> {
  return typedInvoke<NodeEstimate>("history_estimate_cmd", { inputPath });
}

export async function getKernel(scope: HistoryScope, kernel: KernelRef): Promise<number[][]> {
  const res = await typedInvoke<{ hash: string; kernel: number[][] }>("history_kernel_cmd", { ...scopeArgs(scope), ...kernel });
  return res.kernel;
}
