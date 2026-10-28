import type { ChainBase, ChainEntry, ChainStep, ProcessedResult, ProcessingChain } from "../shared/types/preview";
import type { HistoryDoc, HistoryNode, KernelRef, OpName, OpSpec } from "../shared/types/ops";
import { samePath, withStep } from "./processingChain";

const OP_STEPS: Record<OpName, ChainStep> = {
  background: "background",
  wavelet_denoise: "denoise",
  rl_deconv: "deconv",
  arcsinh: "stretch",
  ghs: "stretch",
  masked_stretch: "maskedStretch",
  lhe: "localContrast",
  hdrmt: "localContrast",
  tone: "tone",
  pixelmath: "pixelMath",
};

const OP_LABELS: Record<OpName, string> = {
  background: "Background Extraction",
  wavelet_denoise: "Wavelet Denoise",
  rl_deconv: "Deconvolution",
  arcsinh: "Arcsinh Stretch",
  ghs: "GHS Stretch",
  masked_stretch: "Masked Stretch",
  lhe: "LHE",
  hdrmt: "HDRMT",
  tone: "Curves",
  pixelmath: "PixelMath",
};

export function opToStep(op: OpSpec): ChainStep {
  return OP_STEPS[op.op];
}

export function opLabel(op: OpSpec): string {
  if (op.op !== "background") return OP_LABELS[op.op];
  if (op.model === "spline") return "DBE";
  return op.model === "polynomial" ? OP_LABELS.background : "Deband";
}

const SUMMARY_MAX = 40;

function num(n: number): string {
  return String(Number(n.toPrecision(4)));
}

function clip(text: string): string {
  return text.length > SUMMARY_MAX ? `${text.slice(0, SUMMARY_MAX - 1)}…` : text;
}

function deconvPsf(op: Extract<OpSpec, { op: "rl_deconv" }>): string {
  const psf = op.psf;
  if (psf.kind === "kernel") return `provided PSF ${psf.rows}×${psf.cols}`;
  if (psf.kind === "empirical") return `empirical PSF (${psf.numStars} stars)`;
  return `Gaussian PSF σ ${num(psf.sigma)}`;
}

function backgroundSummary(op: Extract<OpSpec, { op: "background" }>): string {
  if (op.model === "polynomial") return `grid ${op.config.gridSize} · degree ${op.config.polyDegree} · ${op.config.mode}`;
  if (op.model === "spline") return `tolerance ${num(op.config.tolerance)} · smoothing ${num(op.config.smoothing)} · ${op.config.mode}`;
  return `${op.model.replace("deband_", "")} axis · σ ${num(op.config.sigmaClip)}`;
}

export function opSummary(op: OpSpec): string {
  switch (op.op) {
    case "background":
      return backgroundSummary(op);
    case "wavelet_denoise":
      return `thresholds ${op.thresholds.map(num).join(" / ")}${op.linearDenoise ? "" : " · non-linear"}`;
    case "rl_deconv":
      return `${op.iterations} iterations · ${deconvPsf(op)}`;
    case "arcsinh":
      return `factor ${num(op.factor)}`;
    case "ghs":
      return `D ${num(op.d)} · b ${num(op.b)} · SP ${num(op.sp)}`;
    case "masked_stretch":
      return `${op.iterations} iterations · target ${num(op.targetBackground)}`;
    case "lhe":
      return `radius ${op.kernelRadius} · limit ${num(op.contrastLimit)} · amount ${num(op.amount)}`;
    case "hdrmt":
      return `${op.layers} layers · ${op.iterations} ${op.iterations === 1 ? "iteration" : "iterations"}`;
    case "tone": {
      const parts = [
        op.levels ? `levels ${num(op.levels.black)} / ${num(op.levels.gamma)} / ${num(op.levels.white)}` : null,
        op.curve ? `curve ${op.curve.points.length} points` : null,
      ];
      return parts.filter((p): p is string => p !== null).join(" · ");
    }
    case "pixelmath":
      return clip(op.expression);
  }
}

export function nodeNumber(id: string): number {
  const n = Number.parseInt(id.replace(/^n/, ""), 10);
  return Number.isFinite(n) ? n : -1;
}

function nodesById(doc: HistoryDoc): Map<string, HistoryNode> {
  return new Map(doc.nodes.map((n) => [n.id, n]));
}

export function ancestry(doc: HistoryDoc, id: string): HistoryNode[] {
  const byId = nodesById(doc);
  const path: HistoryNode[] = [];
  let cursor = byId.get(id);
  while (cursor && path.length < doc.nodes.length) {
    path.push(cursor);
    cursor = cursor.parent === null ? undefined : byId.get(cursor.parent);
  }
  return path.reverse();
}

export function childrenOf(doc: HistoryDoc, parent: string | null): HistoryNode[] {
  return doc.nodes.filter((n) => n.parent === parent).sort((a, b) => nodeNumber(b.id) - nodeNumber(a.id));
}

function newestDescent(doc: HistoryDoc, from: string | null): HistoryNode[] {
  const tail: HistoryNode[] = [];
  let next = childrenOf(doc, from)[0];
  while (next && tail.length < doc.nodes.length) {
    tail.push(next);
    next = childrenOf(doc, next.id)[0];
  }
  return tail;
}

export function activePath(doc: HistoryDoc): HistoryNode[] {
  return doc.head === null ? [] : ancestry(doc, doc.head);
}

export function redoTail(doc: HistoryDoc): HistoryNode[] {
  return newestDescent(doc, doc.head);
}

export interface BranchMarker {
  at: string | null;
  branches: HistoryNode[][];
}

export function branchMarkers(doc: HistoryDoc): BranchMarker[] {
  const shown = [...activePath(doc), ...redoTail(doc)];
  const onPath = new Set(shown.map((n) => n.id));
  const forks: (string | null)[] = [null, ...shown.map((n) => n.id)];
  return forks
    .map((at) => ({
      at,
      branches: childrenOf(doc, at)
        .filter((child) => !onPath.has(child.id))
        .map((child) => [child, ...newestDescent(doc, child.id)]),
    }))
    .filter((marker) => marker.branches.length > 0);
}

const RECORDED_AT = /^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})(?:\.(\d+))?Z$/;

export function nodePreviewVersion(node: HistoryNode): number | null {
  const match = RECORDED_AT.exec(node.at);
  if (!match) return null;
  const millis = (match[2] ?? "").padEnd(3, "0").slice(0, 3);
  const at = Date.parse(`${match[1]}.${millis}Z`);
  return Number.isFinite(at) ? at : null;
}

export function nodeFile(dir: string, name: string): string {
  const separator = dir.includes("\\") && !dir.includes("/") ? "\\" : "/";
  const trimmed = dir.endsWith("\\") || dir.endsWith("/") ? dir.slice(0, -1) : dir;
  return `${trimmed}${separator}${name}`;
}

export function nodeAtPath(doc: HistoryDoc, dir: string, path: string): HistoryNode | null {
  return doc.nodes.find((n) => samePath(nodeFile(dir, n.output.path), path)) ?? null;
}

export function headKernelRef(doc: HistoryDoc, headId: string | null): KernelRef | null {
  if (headId === null) return null;
  const path = ancestry(doc, headId);
  for (let i = path.length - 1; i >= 0; i--) {
    const op = path[i].op;
    if (op.op === "rl_deconv" && op.psf.kind === "kernel") return { hash: op.psf.hash, rows: op.psf.rows, cols: op.psf.cols };
  }
  return null;
}

export type PreviewLookup = (pngPath: string) => string | null;

const NO_PREVIEW: PreviewLookup = () => null;

export interface ProjectOptions {
  dir: string;
  base: ChainBase | null;
  psfKernel?: number[][] | null;
  kernels?: ReadonlyMap<string, number[][]>;
  previewOf?: PreviewLookup;
}

function entryOf(node: HistoryNode, dir: string, previewOf: PreviewLookup): ChainEntry {
  const op = node.op;
  return {
    fitsPath: nodeFile(dir, node.output.path),
    previewUrl: previewOf(nodeFile(dir, node.output.png)),
    dimensions: [node.output.dims[0], node.output.dims[1]],
    ...(op.op === "rl_deconv" ? { psfUsed: op.psf.kind === "kernel" } : {}),
  };
}

export function projectChain(doc: HistoryDoc, headId: string | null, options: ProjectOptions): ProcessingChain {
  const previewOf = options.previewOf ?? NO_PREVIEW;
  const path = headId === null ? [] : ancestry(doc, headId);
  let chain: ProcessingChain = { steps: {}, psfKernel: null, base: options.base };
  for (const node of path) chain = withStep(chain, opToStep(node.op), entryOf(node, options.dir, previewOf));
  const ref = headKernelRef(doc, headId);
  const fallback = options.psfKernel ?? null;
  const psfKernel = ref ? options.kernels?.get(ref.hash) ?? fallback : fallback;
  return { ...chain, psfKernel };
}

export interface RecordRoot {
  dir: string;
  rootPath: string;
  rootPreviewUrl: string | null;
  previewOf?: PreviewLookup;
}

export function nodeRecord(doc: HistoryDoc, id: string, root: RecordRoot): ProcessedResult | null {
  const node = doc.nodes.find((n) => n.id === id);
  if (!node) return null;
  const previewOf = root.previewOf ?? NO_PREVIEW;
  const parent = node.parent === null ? undefined : doc.nodes.find((n) => n.id === node.parent);
  const inputPath = parent ? nodeFile(root.dir, parent.output.path) : root.rootPath;
  const record: ProcessedResult = {
    fitsPath: nodeFile(root.dir, node.output.path),
    previewUrl: previewOf(nodeFile(root.dir, node.output.png)),
    dimensions: [node.output.dims[0], node.output.dims[1]],
    label: opLabel(node.op),
    kind: node.op.op === "pixelmath" ? "pixelmath" : "processing",
    inputPath,
    planes: node.output.planes,
    domain: node.output.domain,
  };
  if (node.op.op !== "pixelmath") return record;
  const inputPreviewUrl = parent ? previewOf(nodeFile(root.dir, parent.output.png)) : root.rootPreviewUrl;
  return { ...record, inputPreviewUrl };
}
