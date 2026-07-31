let _pixels: Float32Array | null = null;
let _width = 0;
let _height = 0;
let _rgba: Uint8ClampedArray | null = null;

function stfTransfer(
  raw: number,
  invRange: number,
  dataMin: number,
  shadow: number,
  clipRange: number,
  midtone: number,
): number {
  if (raw !== raw) return 0;
  const norm = (raw - dataMin) * invRange;
  const clipped = Math.max(0, Math.min(1, (norm - shadow) / clipRange));
  if (clipped <= 0) return 0;
  if (clipped >= 1) return 1;
  if (Math.abs(midtone - 0.5) < 1e-6) return clipped;
  const b = (2 * midtone - 1) * clipped - midtone;
  if (Math.abs(b) < 1e-8) return clipped;
  return ((midtone - 1) * clipped) / b;
}

function ensureRgba(len: number): Uint8ClampedArray {
  const needed = len * 4;
  if (!_rgba || _rgba.length < needed) {
    _rgba = new Uint8ClampedArray(needed);
  }
  return _rgba;
}

self.onmessage = function (e: MessageEvent) {
  const { type, id } = e.data;

  if (type === "setPixels") {
    _pixels = e.data.pixels as Float32Array;
    _width = e.data.width;
    _height = e.data.height;
    _rgba = null;
    self.postMessage({ type: "pixelsReady", id });
    return;
  }

  if (type === "clearPixels") {
    _pixels = null;
    _width = 0;
    _height = 0;
    _rgba = null;
    return;
  }

  if (type === "render") {
    const pixels: Float32Array | null = e.data.pixels || _pixels;
    const width: number = e.data.width || _width;
    const height: number = e.data.height || _height;
    if (!pixels) return;

    const { dataMin, dataMax, shadow, midtone, highlight } = e.data;
    const len = width * height;
    const rgba = ensureRgba(len);

    const range = Math.max(dataMax - dataMin, 1e-8);
    const invRange = 1.0 / range;
    const clipRange = Math.max(highlight - shadow, 1e-8);

    for (let i = 0; i < len; i++) {
      const val = stfTransfer(pixels[i], invRange, dataMin, shadow, clipRange, midtone);
      const byte = (val * 255 + 0.5) | 0;
      const off = i * 4;
      rgba[off] = byte;
      rgba[off + 1] = byte;
      rgba[off + 2] = byte;
      rgba[off + 3] = 255;
    }

    const imgData = new ImageData(rgba.subarray(0, len * 4) as Uint8ClampedArray<ArrayBuffer>, width, height);
    createImageBitmap(imgData).then((bitmap) => {
      self.postMessage({ type: "rendered", id, bitmap, width, height }, { transfer: [bitmap] });
    });
  }
};
