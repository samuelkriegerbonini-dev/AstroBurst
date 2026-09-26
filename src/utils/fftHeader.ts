const FFT_HEADER_BYTES = 40;
const FFT_GRID_COLS_OFFSET = 20;
const FFT_GRID_ROWS_OFFSET = 24;
const FFT_FLAGS_OFFSET = 28;
const FFT_WINDOWED_FLAG = 1;
const FFT_DOWNSAMPLED_FLAG = 2;

export interface FftGrid {
  grid_width: number;
  grid_height: number;
  windowed: boolean;
  downsampled: boolean;
}

export function parseFftGrid(bytes: Uint8Array): FftGrid {
  if (bytes.length < FFT_HEADER_BYTES) {
    throw new Error(`FFT: response too small (${bytes.length} bytes)`);
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const flags = view.getUint32(FFT_FLAGS_OFFSET, true);
  return {
    grid_width: view.getUint32(FFT_GRID_COLS_OFFSET, true),
    grid_height: view.getUint32(FFT_GRID_ROWS_OFFSET, true),
    windowed: (flags & FFT_WINDOWED_FLAG) !== 0,
    downsampled: (flags & FFT_DOWNSAMPLED_FLAG) !== 0,
  };
}
