export const RGB_FITS_NOTICE =
  "This is an RGB FITS. The Processing tools work on mono FITS files or on a colour composite built in Compose; they cannot process the planes of an RGB file yet.";

export function rgbFitsDisabledReason({ fileIsRgb, compositeMode }: { fileIsRgb: boolean; compositeMode: boolean }): string | null {
  return fileIsRgb && !compositeMode ? RGB_FITS_NOTICE : null;
}
