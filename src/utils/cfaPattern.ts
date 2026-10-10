const BAYER_PATTERNS: ReadonlySet<string> = new Set(["RGGB", "BGGR", "GRBG", "GBRG"]);

export function cfaPatternFromHeader(header: Record<string, string> | null | undefined): string | null {
  const raw = header?.BAYERPAT ?? header?.COLORTYP;
  if (raw == null) return null;
  const value = String(raw).trim().replace(/^'+|'+$/g, "").trim().toUpperCase();
  return BAYER_PATTERNS.has(value) ? value : null;
}

export function cosmeticCfaValue(touched: boolean, checked: boolean): boolean | null {
  return touched ? checked : null;
}
