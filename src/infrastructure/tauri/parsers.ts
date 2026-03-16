export function parseRawPixelBuffer(arrayBuffer: ArrayBuffer) {
  const header = new DataView(arrayBuffer, 0, 16);
  return {
    width: header.getUint32(0, true),
    height: header.getUint32(4, true),
    dataMin: header.getFloat32(8, true),
    dataMax: header.getFloat32(12, true),
    pixels: new Float32Array(arrayBuffer, 16),
  };
}

export function toUint8Array(raw: any): Uint8Array {
  if (raw instanceof ArrayBuffer) return new Uint8Array(raw);
  if (raw instanceof Uint8Array) return raw;
  if (ArrayBuffer.isView(raw)) return new Uint8Array(raw.buffer, raw.byteOffset, raw.byteLength);
  if (Array.isArray(raw)) return new Uint8Array(raw);
  throw new Error(`Unexpected IPC response type: ${typeof raw} / ${raw?.constructor?.name}`);
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
