interface RenderResult {
  bitmap: ImageBitmap;
  width: number;
  height: number;
}

interface RenderParams {
  pixels?: Float32Array;
  width?: number;
  height?: number;
  dstWidth?: number;
  dstHeight?: number;
  dataMin: number;
  dataMax: number;
  shadow: number;
  midtone: number;
  highlight: number;
}

type Callback = (result: RenderResult | null) => void;

let _worker: Worker | null = null;
let _pendingCallbacks = new Map<number, Callback>();
let _nextId = 0;
let _hasPixels = false;
let _pixelsGeneration = 0;
let _pendingPixelsPromise: Promise<void> | null = null;

function getStfWorker(): Worker {
  if (_worker) return _worker;
  _worker = new Worker(
    new URL("./stfworker.worker.ts", import.meta.url),
    { type: "module" },
  );
  _worker.onmessage = (e: MessageEvent) => {
    const { type, id, bitmap, width, height } = e.data;
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
      }
    }
  };
  return _worker;
}

export function setWorkerPixels(
  pixels: Float32Array,
  width: number,
  height: number,
): Promise<void> {
  const gen = ++_pixelsGeneration;
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
  _pendingPixelsPromise = promise;
  return promise;
}

export function renderStfInWorker(params: RenderParams): Promise<RenderResult> {
  return new Promise((resolve) => {
    const worker = getStfWorker();
    const id = _nextId++;
    _pendingCallbacks.set(id, (result) => {
      if (result) resolve(result);
    });

    const {
      pixels, width, height,
      dataMin, dataMax, shadow, midtone, highlight,
      dstWidth, dstHeight,
    } = params;

    const useRetained = _hasPixels && !pixels;

    if (dstWidth && dstHeight && (dstWidth !== (width || 0) || dstHeight !== (height || 0))) {
      const msg: Record<string, unknown> = {
        type: "downsampleAndRender",
        id,
        srcWidth: width,
        srcHeight: height,
        dstWidth,
        dstHeight,
        dataMin,
        dataMax,
        shadow,
        midtone,
        highlight,
      };
      if (!useRetained && pixels) {
        msg.pixels = pixels;
      }
      worker.postMessage(msg);
    } else {
      const msg: Record<string, unknown> = {
        type: "render",
        id,
        width,
        height,
        dataMin,
        dataMax,
        shadow,
        midtone,
        highlight,
      };
      if (!useRetained && pixels) {
        msg.pixels = pixels;
      }
      worker.postMessage(msg);
    }
  });
}

export function cancelPendingRenders(): void {
  _pendingCallbacks.clear();
}

export function terminateStfWorker(): void {
  if (_worker) {
    _worker.terminate();
    _worker = null;
    _pendingCallbacks.clear();
    _nextId = 0;
    _hasPixels = false;
    _pixelsGeneration = 0;
    _pendingPixelsPromise = null;
  }
}
