export function parseRawPixelBuffer(raw: ArrayBuffer | ArrayBufferView) {
  const bytes = toUint8Array(raw);
  if (bytes.length < 16) {
    throw new Error(`raw pixels: response too small (${bytes.length} bytes)`);
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const width = view.getUint32(0, true);
  const height = view.getUint32(4, true);
  const dataMin = view.getFloat32(8, true);
  const dataMax = view.getFloat32(12, true);

  const npix = width * height;
  const expected = 16 + npix * 4;
  if (bytes.length < expected) {
    throw new Error(`raw pixels: expected ${expected} bytes, got ${bytes.length}`);
  }

  const pixelOffset = bytes.byteOffset + 16;
  const pixels = pixelOffset % 4 === 0
    ? new Float32Array(bytes.buffer, pixelOffset, npix)
    : new Float32Array(bytes.slice(16, expected).buffer);

  return { width, height, dataMin, dataMax, pixels };
}

export function parseRawRgbPixelBuffer(raw: ArrayBuffer | ArrayBufferView) {
  const bytes = toUint8Array(raw);
  if (bytes.length < 32) {
    throw new Error(`raw rgb pixels: response too small (${bytes.length} bytes)`);
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const width = view.getUint32(0, true);
  const height = view.getUint32(4, true);
  const rMin = view.getFloat32(8, true);
  const rMax = view.getFloat32(12, true);
  const gMin = view.getFloat32(16, true);
  const gMax = view.getFloat32(20, true);
  const bMin = view.getFloat32(24, true);
  const bMax = view.getFloat32(28, true);

  const npix = width * height;
  const blockBytes = npix * 4;
  const expected = 32 + blockBytes * 3;
  if (bytes.length < expected) {
    throw new Error(`raw rgb pixels: expected ${expected} bytes, got ${bytes.length}`);
  }

  const channel = (blockIndex: number, min: number, max: number) => {
    const start = bytes.byteOffset + 32 + blockIndex * blockBytes;
    const data =
      start % 4 === 0
        ? new Float32Array(bytes.buffer, start, npix)
        : new Float32Array(bytes.slice(32 + blockIndex * blockBytes, 32 + (blockIndex + 1) * blockBytes).buffer);
    return { data, min, max };
  };

  return {
    width,
    height,
    r: channel(0, rMin, rMax),
    g: channel(1, gMin, gMax),
    b: channel(2, bMin, bMax),
  };
}

export function toUint8Array(raw: unknown): Uint8Array {
  if (raw instanceof ArrayBuffer) return new Uint8Array(raw);
  if (raw instanceof Uint8Array) return raw;
  if (ArrayBuffer.isView(raw)) return new Uint8Array(raw.buffer, raw.byteOffset, raw.byteLength);
  if (Array.isArray(raw)) return new Uint8Array(raw);
  const ctorName = (raw as { constructor?: { name?: string } } | null | undefined)?.constructor?.name;
  throw new Error(`Unexpected IPC response type: ${typeof raw} / ${ctorName}`);
}

const FFT_HEADER_SIZE = 32;

export function parseFftBuffer(bytes: Uint8Array) {
  if (bytes.length < FFT_HEADER_SIZE) {
    throw new Error(`FFT: response too small (${bytes.length} bytes)`);
  }

  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const width = view.getUint32(0, true);
  const height = view.getUint32(4, true);

  const expectedLen = FFT_HEADER_SIZE + width * height;
  if (bytes.length < expectedLen) {
    throw new Error(`FFT: expected ${expectedLen} bytes, got ${bytes.length}`);
  }

  return {
    width,
    height,
    dc_magnitude: view.getFloat32(8, true),
    max_magnitude: view.getFloat32(12, true),
    elapsed_ms: view.getUint32(16, true),
    original_size: view.getUint32(20, true),
    windowed: view.getUint32(24, true) !== 0,
    pixels: new Uint8Array(bytes.buffer, bytes.byteOffset + FFT_HEADER_SIZE, width * height),
  };
}
