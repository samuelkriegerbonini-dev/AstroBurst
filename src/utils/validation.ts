const VALID_EXTENSIONS = [".fits", ".fit", ".fts", ".asdf", ".zip"];

export function isValidFitsFile(nameOrPath: string): boolean {
  const lower = nameOrPath.toLowerCase();
  return VALID_EXTENSIONS.some((ext) => lower.endsWith(ext));
}

const CALIB_REF_RE =
  /^jwst_[a-z]+_(distortion|filteroffset|sirskernel|photom|flat|dark|bias|readnoise|gain|linearity|saturation|superbias|ipc|area|specwcs|regions|wavelengthrange|trappars|mask|drizpars|throughput|psfmask)_\d+\.asdf$/i;

export function isCalibRefAsdf(name: string): boolean {
  return CALIB_REF_RE.test(name);
}
