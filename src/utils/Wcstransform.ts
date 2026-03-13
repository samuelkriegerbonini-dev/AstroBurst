export interface WcsParams {
  crpix1: number;
  crpix2: number;
  crval1: number;
  crval2: number;
  cd: [[number, number], [number, number]];
  projection: string;
}

export interface CelestialCoord {
  ra: number;
  dec: number;
}

const DEG2RAD = Math.PI / 180;
const RAD2DEG = 180 / Math.PI;

function deproject(
  xi_deg: number,
  eta_deg: number,
  crval1: number,
  crval2: number,
  projection: string,
): CelestialCoord {
  const xi = xi_deg * DEG2RAD;
  const eta = eta_deg * DEG2RAD;
  const ra0 = crval1 * DEG2RAD;
  const dec0 = crval2 * DEG2RAD;

  const cosDec0 = Math.cos(dec0);
  const sinDec0 = Math.sin(dec0);

  let ra: number;
  let dec: number;

  switch (projection) {
    case "TAN": {
      const denom = cosDec0 - eta * sinDec0;
      ra = ra0 + Math.atan2(xi, denom);
      dec = Math.atan2(
        sinDec0 + eta * cosDec0,
        Math.sqrt(xi * xi + denom * denom),
      );
      break;
    }
    case "SIN": {
      const cos_c = Math.sqrt(Math.max(0, 1 - xi * xi - eta * eta));
      dec = Math.asin(cos_c * sinDec0 + eta * cosDec0);
      ra = ra0 + Math.atan2(xi, cos_c * cosDec0 - eta * sinDec0);
      break;
    }
    case "ARC": {
      const rho = Math.sqrt(xi * xi + eta * eta);
      if (rho < 1e-15) {
        ra = ra0;
        dec = dec0;
      } else {
        const c = rho;
        dec = Math.asin(
          Math.cos(c) * sinDec0 + (eta / rho) * Math.sin(c) * cosDec0,
        );
        ra =
          ra0 +
          Math.atan2(
            xi * Math.sin(c),
            rho * cosDec0 * Math.cos(c) - eta * sinDec0 * Math.sin(c),
          );
      }
      break;
    }
    case "CAR":
      ra = ra0 + xi / cosDec0;
      dec = dec0 + eta;
      break;
    default: {
      const denom = cosDec0 - eta * sinDec0;
      ra = ra0 + Math.atan2(xi, denom);
      dec = Math.atan2(
        sinDec0 + eta * cosDec0,
        Math.sqrt(xi * xi + denom * denom),
      );
    }
  }

  let raDeg = ra * RAD2DEG;
  if (raDeg < 0) raDeg += 360;
  if (raDeg >= 360) raDeg -= 360;

  return { ra: raDeg, dec: dec * RAD2DEG };
}

export function pixelToWorld(params: WcsParams, x: number, y: number): CelestialCoord {
  const dx = x - params.crpix1 + 1;
  const dy = y - params.crpix2 + 1;

  const xi = params.cd[0][0] * dx + params.cd[0][1] * dy;
  const eta = params.cd[1][0] * dx + params.cd[1][1] * dy;

  return deproject(xi, eta, params.crval1, params.crval2, params.projection);
}
