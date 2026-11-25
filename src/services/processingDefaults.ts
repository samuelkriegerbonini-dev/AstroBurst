export const BACKGROUND_DEFAULTS = { gridSize: 8, polyDegree: 3, sigmaClip: 2.5, iterations: 3, mode: "subtract" } as const;

export const DECONV_DEFAULTS = {
  iterations: 20,
  psfSigma: 2.0,
  psfSize: 15,
  regularization: 0.001,
  deringing: true,
  deringThreshold: 0.1,
  useEmpiricalPsf: false,
  psfNumStars: 30,
  psfCutoutRadius: 15,
} as const;

export const WAVELET_DEFAULTS = { numScales: 5, thresholds: [3.0, 2.5, 2.0, 1.5, 1.0], linear: true } as const;
