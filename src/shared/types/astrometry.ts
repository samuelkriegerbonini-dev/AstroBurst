export interface WcsInfo {
  center_ra: number;
  center_dec: number;
  pixel_scale_arcsec: number;
  fov_arcmin: [number, number];
  field_of_view_w_arcmin: number;
  field_of_view_h_arcmin: number;
  naxis1: number;
  naxis2: number;
  wcs_params?: {
    crpix1: number;
    crpix2: number;
    crval1: number;
    crval2: number;
    cd: number[];
    projection: string;
  };
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
