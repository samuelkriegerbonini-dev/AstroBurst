import type { DisplayTransfer } from "./displayTransfer";

export type { DisplayTransfer } from "./displayTransfer";

export interface RenderResult {
  bitmap: ImageBitmap;
  width: number;
  height: number;
}

interface RenderParams {
  pixels?: Float32Array;
  width?: number;
  height?: number;
  transfer: DisplayTransfer;
  lut: Uint8Array;
}

type Callback = (result: RenderResult | null, failure?: string) => void;

let _worker: Worker | null = null;
const _pendingCallbacks = new Map<number, Callback>();
let _nextId = 0;
let _hasPixels = false;

function failPending(reason: string): void {
  const callbacks = [..._pendingCallbacks.values()];
  _pendingCallbacks.clear();
  for (const cb of callbacks) cb(null, reason);
}

function getStfWorker(): Worker {
  if (_worker) return _worker;
  _worker = new Worker(
    new URL("./stfworker.worker.ts", import.meta.url),
    { type: "module" },
  );
  _worker.onmessage = (e: MessageEvent) => {
    const { type, id, bitmap, width, height, message } = e.data;
    if (type === "failed") {
      const cb = _pendingCallbacks.get(id);
      if (cb) {
        _pendingCallbacks.delete(id);
        cb(null, typeof message === "string" ? message : "STF render failed");
      }
      return;
    }
    if (type === "pixelsReady") {
      _hasPixels = true;
      const cb = _pendingCallbacks.get(id);
      if (cb) {
        _pendingCallbacks.delete(id);
        cb(null);
      }
      return;
    }
    if (type === "rendered") {
      const cb = _pendingCallbacks.get(id);
      if (cb) {
        _pendingCallbacks.delete(id);
        cb({ bitmap, width, height });
      } else if (bitmap) {
        (bitmap as ImageBitmap).close();
      }
    }
  };
  _worker.onerror = (e: ErrorEvent) => {
    failPending(`STF worker error: ${e?.message ?? "unknown"}`);
  };
  return _worker;
}

export function setWorkerPixels(
  pixels: Float32Array,
  width: number,
  height: number,
): Promise<void> {
  const promise = new Promise<void>((resolve) => {
    const worker = getStfWorker();
    const id = _nextId++;
    _hasPixels = false;
    _pendingCallbacks.set(id, () => {
      resolve();
    });
    const copy = new Float32Array(pixels);
    worker.postMessage(
      { type: "setPixels", id, pixels: copy, width, height },
      [copy.buffer],
    );
  });
  return promise;
}

export function renderStfInWorker(params: RenderParams): Promise<RenderResult> {
  return new Promise((resolve, reject) => {
    const worker = getStfWorker();
    const id = _nextId++;
    _pendingCallbacks.set(id, (result, failure) => {
      if (result) resolve(result);
      else reject(new Error(failure ?? "STF render returned no result"));
    });

    const { pixels, width, height, transfer, lut } = params;

    const useRetained = _hasPixels && !pixels;

    const msg: Record<string, unknown> = {
      type: "render",
      id,
      width,
      height,
      transfer,
      lut,
    };
    if (!useRetained && pixels) {
      msg.pixels = pixels;
    }
    worker.postMessage(msg);
  });
}

export function cancelPendingRenders(): void {
  failPending("STF render cancelled");
}

export function clearWorkerPixels(): void {
  _hasPixels = false;
  if (_worker) {
    _worker.postMessage({ type: "clearPixels" });
  }
}

export function terminateStfWorker(): void {
  if (_worker) {
    _worker.terminate();
    _worker = null;
    failPending("STF worker stopped");
    _nextId = 0;
    _hasPixels = false;
  }
}

export function presentWorkerBitmap(canvas: HTMLCanvasElement | null, result: RenderResult, current: boolean): void {
  if (!current || !canvas) {
    result.bitmap.close();
    return;
  }
  if (canvas.width !== result.width || canvas.height !== result.height) {
    canvas.width = result.width;
    canvas.height = result.height;
  }
  const ctx = canvas.getContext("bitmaprenderer");
  if (ctx) {
    ctx.transferFromImageBitmap(result.bitmap);
    return;
  }
  canvas.getContext("2d")?.drawImage(result.bitmap, 0, 0);
  result.bitmap.close();
}
