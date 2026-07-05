export interface WcsInfo {
  center_ra: number;
  center_dec: number;
  pixel_scale_arcsec: number;
  fov_arcmin: [number, number];
  field_of_view_w_arcmin: number;
  field_of_view_h_arcmin: number;
  naxis1: number;
  naxis2: number;
}

/** Result of `pixel_to_world_cmd`: one `[ra, dec]` pair (degrees) per input
 * pixel, or `null` where that pixel has no valid sky position. */
export interface PixelToWorldResult {
  points: ([number, number] | null)[];
}

export interface PlateSolveOptions {
  apiKey?: string;
  scaleLower?: number;
  scaleUpper?: number;
  scaleUnits?: string;
  downsampleFactor?: number;
  centerRa?: number;
  centerDec?: number;
  radius?: number;
}
