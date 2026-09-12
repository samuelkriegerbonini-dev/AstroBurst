import { renderRgba, type DisplayTransfer } from "./displayTransfer";

let _pixels: Float32Array | null = null;
let _width = 0;
let _height = 0;
let _rgba: Uint8ClampedArray | null = null;

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

    const transfer = e.data.transfer as DisplayTransfer;
    const lut = e.data.lut as Uint8Array;
    const len = width * height;
    const rgba = ensureRgba(len);

    renderRgba(pixels.subarray(0, len), transfer, lut, rgba);

    const imgData = new ImageData(rgba.subarray(0, len * 4) as Uint8ClampedArray<ArrayBuffer>, width, height);
    createImageBitmap(imgData).then((bitmap) => {
      self.postMessage({ type: "rendered", id, bitmap, width, height }, { transfer: [bitmap] });
    });
  }
};
