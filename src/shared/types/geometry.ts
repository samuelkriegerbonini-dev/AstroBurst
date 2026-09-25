export interface FrameGeometry {
  jd_utc: number | null;
  jd_tt: number | null;
  jd_tdb: number | null;
  bjd_tdb: number | null;
  hjd_utc: number | null;
  bjd_source: "computed" | "header" | null;
  lst_deg: number | null;
  hour_angle_deg: number | null;
  altitude_deg: number | null;
  azimuth_deg: number | null;
  airmass_computed: number | null;
  airmass_formula: string;
  parallactic_angle_deg: number | null;
  sun_altitude_deg: number | null;
  moon_altitude_deg: number | null;
  moon_illumination: number | null;
  moon_separation_deg: number | null;
  time_scale_notes: string[];
}

export interface GeometryTarget {
  ra_deg: number;
  dec_deg: number;
  source: string;
}

export interface GeometrySite {
  lon_deg: number;
  lat_deg: number;
  height_m: number;
  source: string;
}

export interface ObservationGeometryResult {
  geometry: FrameGeometry;
  target: GeometryTarget | null;
  site: GeometrySite | null;
  time_source: string | null;
  airmass_header: number | null;
  notes: string[];
  elapsed_ms: number;
}

export interface GeometryOverrides {
  targetRa?: number | null;
  targetDec?: number | null;
  siteLat?: number | null;
  siteLon?: number | null;
  siteHeight?: number | null;
}
