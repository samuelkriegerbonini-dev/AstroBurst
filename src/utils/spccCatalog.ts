export type SpccCatalogChoice = "gaia" | "builtin";

export function spccSyntheticNotice(requested: SpccCatalogChoice): string {
  return requested === "builtin"
    ? "Factors come from the synthetic built-in catalogue you chose and are not photometrically reliable. Choose Gaia DR3 and re-run for a real calibration."
    : "Gaia unavailable: factors come from the synthetic built-in catalogue and are not photometrically reliable. Check your connection and re-run for a real calibration.";
}
