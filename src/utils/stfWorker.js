const WORKER_CODE = `
self.onmessage = function(e) {
  const { type, id } = e.data;

  if (type === "render") {
    const { pixels, width, height, dataMin, dataMax, shadow, midtone, highlight } = e.data;
    const len = width * height;
    const rgba = new Uint8ClampedArray(len * 4);

    const range = Math.max(dataMax - dataMin, 1e-20);
    const invRange = 1.0 / range;
    const clipRange = Math.max(highlight - shadow, 1e-10);

    for (let i = 0; i < len; i++) {
      const raw = pixels[i];
      let val = 0;
      if (raw === raw) {
        const norm = Math.max(0, Math.min(1, (raw - dataMin) * invRange));
        const clipped = Math.max(0, Math.min(1, (norm - shadow) / clipRange));
        if (clipped <= 0) val = 0;
        else if (clipped >= 1) val = 1;
        else val = ((midtone - 1) * clipped) / ((2 * midtone - 1) * clipped - midtone);
      }
      const byte = (val * 255 + 0.5) | 0;
      const off = i * 4;
      rgba[off] = byte;
      rgba[off + 1] = byte;
      rgba[off + 2] = byte;
      rgba[off + 3] = 255;
    }

    const imgData = new ImageData(rgba, width, height);
    createImageBitmap(imgData).then(function(bitmap) {
      self.postMessage({ type: "rendered", id, bitmap, width, height }, [bitmap]);
    });
  }

  if (type === "downsampleAndRender") {
    const { pixels, srcWidth, srcHeight, dstWidth, dstHeight, dataMin, dataMax, shadow, midtone, highlight } = e.data;

    const len = dstWidth * dstHeight;
    const rgba = new Uint8ClampedArray(len * 4);
    const xRatio = srcWidth / dstWidth;
    const yRatio = srcHeight / dstHeight;

    const range = Math.max(dataMax - dataMin, 1e-20);
    const invRange = 1.0 / range;
    const clipRange = Math.max(highlight - shadow, 1e-10);

    for (let dy = 0; dy < dstHeight; dy++) {
      const sy = Math.min(Math.floor(dy * yRatio), srcHeight - 1);
      const srcRow = sy * srcWidth;
      const dstRow = dy * dstWidth;
      for (let dx = 0; dx < dstWidth; dx++) {
        const sx = Math.min(Math.floor(dx * xRatio), srcWidth - 1);
        const raw = pixels[srcRow + sx];
        let val = 0;
        if (raw === raw) {
          const norm = Math.max(0, Math.min(1, (raw - dataMin) * invRange));
          const clipped = Math.max(0, Math.min(1, (norm - shadow) / clipRange));
          if (clipped <= 0) val = 0;
          else if (clipped >= 1) val = 1;
          else val = ((midtone - 1) * clipped) / ((2 * midtone - 1) * clipped - midtone);
        }
        const byte = (val * 255 + 0.5) | 0;
        const off = (dstRow + dx) * 4;
        rgba[off] = byte;
        rgba[off + 1] = byte;
        rgba[off + 2] = byte;
        rgba[off + 3] = 255;
      }
    }

    const imgData = new ImageData(rgba, dstWidth, dstHeight);
    createImageBitmap(imgData).then(function(bitmap) {
      self.postMessage({ type: "rendered", id, bitmap, width: dstWidth, height: dstHeight }, [bitmap]);
    });
  }
};
`;

let _worker = null;
let _pendingCallbacks = new Map();
let _nextId = 0;

function getStfWorker() {
  if (_worker) return _worker;
  const blob = new Blob([WORKER_CODE], { type: "application/javascript" });
  const url = URL.createObjectURL(blob);
  _worker = new Worker(url);
  _worker.onmessage = (e) => {
    const { id, bitmap, width, height } = e.data;
    const cb = _pendingCallbacks.get(id);
    if (cb) {
      _pendingCallbacks.delete(id);
      cb({ bitmap, width, height });
    }
  };
  return _worker;
}

export function renderStfInWorker(params) {
  return new Promise((resolve) => {
    const worker = getStfWorker();
    const id = _nextId++;
    _pendingCallbacks.set(id, resolve);

    const { pixels, width, height, dataMin, dataMax, shadow, midtone, highlight, dstWidth, dstHeight } = params;

    if (dstWidth && dstHeight && (dstWidth !== width || dstHeight !== height)) {
      worker.postMessage({
        type: "downsampleAndRender",
        id,
        pixels,
        srcWidth: width,
        srcHeight: height,
        dstWidth,
        dstHeight,
        dataMin,
        dataMax,
        shadow,
        midtone,
        highlight,
      });
    } else {
      worker.postMessage({
        type: "render",
        id,
        pixels,
        width,
        height,
        dataMin,
        dataMax,
        shadow,
        midtone,
        highlight,
      });
    }
  });
}

export function cancelPendingRenders() {
  _pendingCallbacks.clear();
}

export function terminateStfWorker() {
  if (_worker) {
    _worker.terminate();
    _worker = null;
    _pendingCallbacks.clear();
    _nextId = 0;
  }
}
