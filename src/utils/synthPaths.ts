const FITS_EXTENSION = /\.(fits?|fts)$/i;

export interface SynthOutputPaths {
  fits: string;
  catalog: string;
  groundTruth: string;
}

export function synthOutputPaths(chosen: string): SynthOutputPaths {
  const fits = FITS_EXTENSION.test(chosen) ? chosen : `${chosen}.fits`;
  const stem = fits.replace(FITS_EXTENSION, "");
  return { fits, catalog: `${stem}_catalog.csv`, groundTruth: `${stem}_groundtruth.fits` };
}
