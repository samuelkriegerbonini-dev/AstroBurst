import { describe, expect, it } from "vitest";
import {
  ERROR_KIND_OPTIONS,
  bandButtonLabel,
  fileLayerName,
  handSet,
  hduLayerName,
  layerLines,
  noDataText,
  originLabel,
  roleChip,
  rowActions,
  shownLayerText,
} from "../layerLabels";
import type { FitsExtension } from "../../services/header";
import type { PlaneInfo } from "../../shared/types/fits.types";

const BASE: PlaneInfo = {
  kind: "hdu",
  index: 1,
  key: null,
  extname: "IMAGE",
  extver: null,
  is_dq: false,
  is_err: false,
  dq_ref: null,
  err_ref: null,
  source_path: "C:/d/a.fits",
  dq_table: null,
};

type ChipInput = Parameters<typeof roleChip>[0];
const layer = (extname: string, role: FitsExtension["role"], patch: Partial<ChipInput> = {}): ChipInput => ({
  extname,
  is_dq: false,
  is_err: false,
  role,
  role_basis: `EXTNAME ${extname}`,
  role_assumed: false,
  ...patch,
});

const IN_USE: PlaneInfo = {
  ...BASE,
  err_ref: "C:/d/a.fits#hdu=3",
  dq_ref: "C:/d/a.fits#hdu=2",
  err_origin: { role: "variance", source: "EXTNAME VARIANCE, HDU 3: square root", assumed: false, n_no_data: 12, origin: "same_file" },
  dq_origin: "EXTNAME MASK, HDU 2",
  dq_origin_kind: "same_file",
};

const MISMATCH = "error layer HDU 2 (VARIANCE) is 2x2 and does not match the image 4x4: not used";

describe("roleChip", () => {
  it("gives each layer role its chip", () => {
    expect(roleChip(layer("VARIANCE", "variance"))).toEqual({ label: "VAR", title: "variance layer, from EXTNAME VARIANCE", tone: "err" });
    expect(roleChip(layer("WGT", "inverse_variance", { role_assumed: true }))).toEqual({
      label: "IVAR?",
      title: "inverse variance (assumed) layer, from EXTNAME WGT",
      tone: "err",
    });
    expect(roleChip(layer("IVAR", "inverse_variance"))).toEqual({ label: "IVAR", title: "inverse variance layer, from EXTNAME IVAR", tone: "err" });
    expect(roleChip(layer("RMS", "sigma"))).toEqual({ label: "SIGMA", title: "sigma (error) layer, from EXTNAME RMS", tone: "err" });
    expect(roleChip(layer("ERR", "sigma", { is_err: true }))?.label).toBe("ERR");
    expect(roleChip(layer("MASK", "mask"))).toEqual({ label: "MASK", title: "mask layer, from EXTNAME MASK", tone: "dq" });
    expect(roleChip(layer("DQ", "mask", { is_dq: true }))).toEqual({ label: "DQ", title: "mask layer, from EXTNAME DQ", tone: "dq" });
    expect(roleChip(layer("roman.dq", "mask"))?.label).toBe("DQ");
    expect(roleChip(layer(" dq ", "mask"))?.label).toBe("DQ");
    expect(roleChip(layer("IMAGE", "science"))).toBeNull();
    expect(roleChip(layer("WHT", "other"))).toBeNull();
    expect(roleChip(layer("DQ", undefined, { is_dq: true, role_basis: undefined, role_assumed: undefined }))).toEqual({ label: "DQ", title: "DQ layer", tone: "dq" });
    expect(roleChip(layer("ERR", undefined, { is_err: true, role_basis: undefined, role_assumed: undefined }))).toEqual({ label: "ERR", title: "ERR layer", tone: "err" });
    expect(roleChip(layer("SCI", undefined, { role_basis: undefined, role_assumed: undefined }))).toBeNull();
  });
});

describe("rowActions", () => {
  it("offers the actions that fit the role and marks the layer in use", () => {
    const ext = { kind: "hdu" as const, has_data: true, ref: "C:/d/a.fits#hdu=3", role: "variance" as const, role_basis: "EXTNAME VARIANCE" };
    const plane = { err_ref: null, dq_ref: null };
    expect(rowActions(ext, plane, false)).toEqual({ asErr: true, asDq: false, inUse: null });
    expect(rowActions({ ...ext, role: "mask" }, plane, false)).toEqual({ asErr: false, asDq: true, inUse: null });
    expect(rowActions({ ...ext, role: "other" }, plane, false)).toEqual({ asErr: true, asDq: true, inUse: null });
    expect(rowActions({ ...ext, role: undefined }, plane, false)).toEqual({ asErr: true, asDq: true, inUse: null });
    expect(rowActions({ ...ext, role: "science", role_basis: "EXTNAME SCI" }, plane, false)).toEqual({ asErr: false, asDq: false, inUse: null });
    expect(rowActions({ ...ext, role: "science", role_basis: "no layer name or card" }, plane, false)).toEqual({ asErr: true, asDq: true, inUse: null });
    expect(rowActions(ext, { err_ref: ext.ref, dq_ref: null }, false)).toEqual({ asErr: false, asDq: false, inUse: "err" });
    expect(rowActions({ ...ext, role: "other" }, { err_ref: null, dq_ref: ext.ref }, false)).toEqual({ asErr: true, asDq: false, inUse: "dq" });
    const none = { asErr: false, asDq: false, inUse: null };
    expect(rowActions(ext, plane, true)).toEqual(none);
    expect(rowActions(ext, null, false)).toEqual(none);
    expect(rowActions({ ...ext, kind: "array" }, plane, false)).toEqual(none);
    expect(rowActions({ ...ext, has_data: false }, plane, false)).toEqual(none);
  });

  it("a cube row is never offered as a layer and knows when one of its bands is the layer in use", () => {
    const invvar = {
      kind: "hdu" as const,
      has_data: true,
      ref: "C:/d/cut.fits#hdu=1",
      role: "inverse_variance" as const,
      role_basis: "IMAGETYP INVVAR",
      naxis: 3,
      naxis3: 4,
    };
    const mask = { kind: "hdu" as const, has_data: true, ref: "C:/d/cut.fits#hdu=2", role: "mask" as const, role_basis: "IMAGETYP MASKBITS", naxis: 2, naxis3: 0 };
    expect(rowActions(invvar, { err_ref: null, dq_ref: null }, false)).toEqual({ asErr: false, asDq: false, inUse: null });
    expect(rowActions(invvar, { err_ref: "C:/d/cut.fits#hdu=1&plane=1", dq_ref: null }, false)).toEqual({ asErr: false, asDq: false, inUse: "err" });
    expect(rowActions(mask, { err_ref: null, dq_ref: "C:/d/cut.fits#hdu=2" }, false)).toEqual({ asErr: false, asDq: false, inUse: "dq" });
    expect(rowActions(invvar, { err_ref: "C:/d/cut.fits#hdu=10", dq_ref: null }, false).inUse).toBeNull();
    expect(rowActions({ ...invvar, naxis3: 1 }, { err_ref: null, dq_ref: null }, false)).toEqual({ asErr: true, asDq: false, inUse: null });
  });
});

describe("bandButtonLabel", () => {
  it("labels a band by its name or by its plane", () => {
    expect(bandButtonLabel({ plane: 1, label: "r" })).toBe("r");
    expect(bandButtonLabel({ plane: 2, label: null })).toBe("plane 2");
    expect(bandButtonLabel({ plane: 2, label: "" })).toBe("plane 2");
  });
});

describe("layer names", () => {
  it("names an HDU and a file", () => {
    expect(hduLayerName({ index: 3, extname: "VARIANCE" })).toBe("HDU 3 (VARIANCE)");
    expect(hduLayerName({ index: 3, extname: null })).toBe("HDU 3");
    expect(fileLayerName("C:/d/des-dr2_A_r_extn3.fits")).toBe("des-dr2_A_r_extn3.fits");
    expect(fileLayerName("C:\\d\\a.fits#hdu=2")).toBe("a.fits HDU 2");
  });

  it("names a band of another file", () => {
    expect(fileLayerName("C:/d/cut.fits#hdu=1&plane=0")).toBe("cut.fits HDU 1 plane 0");
  });
});

describe("layerLines", () => {
  it("tells where the layers come from", () => {
    expect(layerLines(IN_USE)).toEqual([
      {
        role: "err",
        title: "Error",
        state: "in_use",
        text: "EXTNAME VARIANCE, HDU 3: square root",
        chip: "same file",
        detail: "12 pixels without data",
        canSwitchOff: true,
        canAutomatic: false,
      },
      { role: "dq", title: "Mask", state: "in_use", text: "EXTNAME MASK, HDU 2", chip: "same file", detail: null, canSwitchOff: true, canAutomatic: false },
    ]);
    expect(handSet(IN_USE)).toBe(false);
    expect(noDataText(1)).toBe("1 pixel without data");
    expect(noDataText(0)).toBeNull();
    expect(noDataText(null)).toBeNull();
    expect(noDataText(undefined)).toBeNull();
    expect(layerLines({ ...IN_USE, err_origin: null, dq_origin: null })[0].text).toBe("C:/d/a.fits#hdu=3");
    expect(layerLines({ ...IN_USE, err_origin: null, dq_origin: null })[1].text).toBe("C:/d/a.fits#hdu=2");
  });

  it("tells why a layer is not used and when the automatic choice can come back", () => {
    const [error, mask] = layerLines({ ...BASE, err_unavailable: MISMATCH });
    expect(error).toEqual({ role: "err", title: "Error", state: "not_used", text: MISMATCH, chip: null, detail: null, canSwitchOff: false, canAutomatic: false });
    expect(mask).toEqual({
      role: "dq",
      title: "Mask",
      state: "none",
      text: "none found in this file or beside it",
      chip: null,
      detail: null,
      canSwitchOff: false,
      canAutomatic: false,
    });
    expect(handSet({ ...BASE, err_unavailable: MISMATCH })).toBe(false);

    const off = { ...BASE, err_unavailable: "error layer switched off by the user" };
    expect(layerLines(off)[0]).toMatchObject({ state: "not_used", canSwitchOff: false, canAutomatic: true });
    expect(handSet(off)).toBe(true);
    const maskOff = { ...BASE, dq_unavailable: "mask layer switched off by the user" };
    expect(layerLines(maskOff)[1]).toMatchObject({ state: "not_used", canAutomatic: true });
    expect(handSet(maskOff)).toBe(true);

    for (const [origin, chip] of [["user", "set by hand"], ["recipe", "rebuilt"]] as const) {
      const plane = { ...IN_USE, err_origin: { ...IN_USE.err_origin!, origin } };
      expect(layerLines(plane)[0]).toMatchObject({ chip, canSwitchOff: true, canAutomatic: true });
      expect(handSet(plane)).toBe(true);
    }
    const userMask = { ...IN_USE, dq_origin_kind: "user" as const };
    expect(layerLines(userMask)[1]).toMatchObject({ chip: "set by hand", canAutomatic: true });
    expect(handSet(userMask)).toBe(true);
    expect(layerLines({ ...IN_USE, dq_origin_kind: "sibling" })[1].chip).toBe("sibling file");
    expect(originLabel("sibling")).toBe("sibling file");
    expect(originLabel(null)).toBeNull();
    expect(originLabel("elsewhere")).toBeNull();
  });
});

describe("shownLayerText", () => {
  it("says when the shown plane is itself a layer", () => {
    expect(shownLayerText({ layer: { role: "inverse_variance", basis: "IMTYPE INVVAR", assumed: false } })).toBe(
      "The shown plane is a layer itself: inverse variance (IMTYPE INVVAR). Layers are assigned to science images.",
    );
    expect(shownLayerText({ layer: { role: "science", basis: "EXTNAME SCI", assumed: false } })).toBeNull();
    expect(shownLayerText({ layer: { role: "other", basis: "EXTNAME WHT", assumed: false } })).toBeNull();
    expect(shownLayerText({ layer: null })).toBeNull();
    expect(shownLayerText({})).toBeNull();
  });
});

describe("ERROR_KIND_OPTIONS", () => {
  it("lists the automatic choice first, then the three error conventions", () => {
    expect(ERROR_KIND_OPTIONS).toEqual([
      { value: "auto", label: "Automatic (from its name or cards)" },
      { value: "sigma", label: "Sigma (as is)" },
      { value: "variance", label: "Variance (square root)" },
      { value: "inverse_variance", label: "Inverse variance (1 / square root)" },
    ]);
  });
});
