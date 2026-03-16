export interface CubeInfo {
  naxis1: number;
  naxis2: number;
  naxis3: number;
  bitpix: number;
  bytes_per_pixel: number;
  frame_bytes: number;
  total_data_bytes: number;
  wavelengths: number[] | null;
}
