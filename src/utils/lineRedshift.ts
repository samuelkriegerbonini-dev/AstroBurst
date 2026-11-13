export interface RedshiftValue {
  z: number | null;
  zErr: number | null;
}

export interface LineRedshiftInput {
  restUm: number;
  centroidUm: number;
  centroidErrUm: number;
}

export interface CombinedRedshift {
  z: number;
  zErr: number;
  zErrFormal: number;
  chi2: number;
  dof: number;
  n: number;
}

interface UsableRedshift {
  z: number;
  zErr: number;
}

function usableRedshift(value: RedshiftValue): UsableRedshift | null {
  const { z, zErr } = value;
  if (z === null || zErr === null) return null;
  if (!Number.isFinite(z) || !Number.isFinite(zErr) || zErr <= 0) return null;
  return { z, zErr };
}

export function combineRedshifts(values: readonly RedshiftValue[]): CombinedRedshift | null {
  const usable = values
    .map(usableRedshift)
    .filter((value): value is UsableRedshift => value !== null);
  const n = usable.length;
  if (n === 0) return null;
  if (n === 1) {
    const [only] = usable;
    return { z: only.z, zErr: only.zErr, zErrFormal: only.zErr, chi2: 0, dof: 0, n };
  }
  const weights = usable.map(({ zErr }) => 1 / (zErr * zErr));
  const weightSum = weights.reduce((sum, weight) => sum + weight, 0);
  const z = usable.reduce((sum, value, i) => sum + weights[i] * value.z, 0) / weightSum;
  const zErrFormal = 1 / Math.sqrt(weightSum);
  const chi2 = usable.reduce((sum, value, i) => sum + weights[i] * (value.z - z) ** 2, 0);
  const dof = n - 1;
  const zErr = zErrFormal * Math.sqrt(Math.max(chi2 / dof, 1));
  return { z, zErr, zErrFormal, chi2, dof, n };
}

export function redshiftFromLines(lines: readonly LineRedshiftInput[]): CombinedRedshift | null {
  return combineRedshifts(
    lines
      .filter(({ restUm }) => Number.isFinite(restUm) && restUm > 0)
      .map(({ restUm, centroidUm, centroidErrUm }) => ({
        z: centroidUm / restUm - 1,
        zErr: centroidErrUm / restUm,
      })),
  );
}
