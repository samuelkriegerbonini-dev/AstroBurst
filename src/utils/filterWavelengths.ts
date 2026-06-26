const FILTER_WAVELENGTHS_NM = new Map<string, number>([
  ["HA", 656], ["HALPHA", 656], ["H_ALPHA", 656],
  ["OIII", 501], ["O3", 501],
  ["SII", 673], ["S2", 673],
  ["NII", 658],
  ["HB", 486], ["HBETA", 486],
  ["F656N", 656], ["F657N", 657], ["F658N", 658],
  ["F673N", 673],
  ["F501N", 501], ["F502N", 501], ["F503N", 503],
  ["F487N", 487], ["F469N", 469], ["F631N", 631],
  ["F070W", 700], ["F090W", 900], ["F115W", 1150], ["F140M", 1400],
  ["F150W", 1500], ["F150W2", 1500], ["F162M", 1620], ["F164N", 1640],
  ["F182M", 1820], ["F187N", 1870], ["F200W", 2000], ["F210M", 2100],
  ["F212N", 2120], ["F250M", 2500], ["F277W", 2770], ["F300M", 3000],
  ["F322W2", 3220], ["F323N", 3230], ["F335M", 3350], ["F356W", 3560],
  ["F360M", 3600], ["F405N", 4050], ["F410M", 4100], ["F430M", 4300],
  ["F444W", 4440], ["F460M", 4600], ["F466N", 4660], ["F470N", 4700],
  ["F480M", 4800],
]);

export function filterToWavelengthNm(filter?: string | null): number | null {
  return filterCodeAndWavelengthNm(filter)?.nm ?? null;
}

export function filterCodeAndWavelengthNm(filter?: string | null): { code: string; nm: number } | null {
  if (!filter) return null;
  const upper = filter.toUpperCase();
  const trimmed = upper.trim();
  const direct = FILTER_WAVELENGTHS_NM.get(trimmed);
  if (direct != null) return { code: trimmed, nm: direct };
  for (const token of upper.split(/[^A-Z0-9]+/)) {
    if (!token || token === "CLEAR") continue;
    const nm = FILTER_WAVELENGTHS_NM.get(token);
    if (nm != null) return { code: token, nm };
  }
  return null;
}
