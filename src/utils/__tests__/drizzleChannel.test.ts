import { describe, expect, it } from "vitest";
import { autoDrizzleChannel, monoDrizzleFilterLabel, unassignedFilterLine, withAutoChannels } from "../drizzleChannel";

function frame(name: string, header: Record<string, string> | null = null) {
  return { name, path: `C:/sx13/${name}`, result: header ? { header } : null };
}

const lightR = (name: string) => frame(name, { IMAGETYP: "LIGHT", FILTER: "R" });
const flatR = (name: string) => frame(name, { IMAGETYP: "FLAT", FILTER: "R" });

describe("autoDrizzleChannel", () => {
  it("reads the channel from the name of a plain FITS frame", () => {
    expect(autoDrizzleChannel(frame("m42_r.fits"))).toBe("r");
    expect(autoDrizzleChannel(frame("M42-Green.fit"))).toBe("g");
    expect(autoDrizzleChannel(frame("blue_m42.fts"))).toBe("b");
    expect(autoDrizzleChannel(frame("m42_lum.fits"))).toBeNull();
  });

  it("gives a gzip-compressed frame the channel of its plain file", () => {
    for (const base of ["m42_r", "M42-Green", "blue_m42", "m42.b", "plain"]) {
      for (const ext of [".fits", ".fit", ".fts"]) {
        expect(autoDrizzleChannel(frame(`${base}${ext}.gz`)), `${base}${ext}.gz`).toBe(autoDrizzleChannel(frame(`${base}${ext}`)));
      }
    }
  });

  it("FILTER=Red assigns r with no pattern in the name", () => {
    expect(autoDrizzleChannel(frame("frame_0001.fits", { IMAGETYP: "LIGHT", FILTER: "Red" }))).toBe("r");
    expect(autoDrizzleChannel(frame("frame_0002.fits", { FILTER: "G" }))).toBe("g");
    expect(autoDrizzleChannel(frame("frame_0003.fits", { IMAGETYP: "EXT", FILTER: "Blue" }))).toBe("b");
  });

  it("reads the FITS filter before the file name", () => {
    expect(autoDrizzleChannel(frame("m42_R_1.fits", { FILTER: "Green" }))).toBe("g");
  });

  it("IMAGETYP=FLAT with FILTER=R is not assigned", () => {
    expect(autoDrizzleChannel(flatR("flat_R_0001.fits"))).toBeNull();
    for (const imagetyp of ["DARK", "BIAS", "FLAT DARK"]) {
      expect(autoDrizzleChannel(frame("cal_R_0001.fits", { IMAGETYP: imagetyp, FILTER: "R" })), imagetyp).toBeNull();
    }
  });

  it("leaves Hα, OIII, SII, NII and L frames to the user", () => {
    for (const filter of ["Ha", "OIII", "SII", "NII", "L", "Luminance"]) {
      expect(autoDrizzleChannel(frame("frame_0001.fits", { IMAGETYP: "LIGHT", FILTER: filter })), filter).toBeNull();
    }
  });
});

describe("withAutoChannels", () => {
  it("puts the 6 FILTER=R lights named by sequence number in R and leaves the 2 R flats out", () => {
    const lights = ["frame_0001.fits", "frame_0002.fits", "frame_0003.fits", "frame_0004.fits", "frame_0005.fits", "frame_0006.fits"].map(lightR);
    const flats = ["frame_0007.fits", "frame_0008.fits"].map(flatR);
    expect(withAutoChannels({}, [...lights, ...flats])).toEqual(Object.fromEntries(lights.map((f) => [f.path, "r"])));
  });

  it("keeps what the user assigned by hand", () => {
    const light = frame("m42_R_1.fits");
    const flat = flatR("flat_0001.fits");
    const other = frame("m42_G_1.fits");
    expect(withAutoChannels({ [light.path]: "b", [flat.path]: "r" }, [light, flat, other])).toEqual({
      [light.path]: "b",
      [flat.path]: "r",
      [other.path]: "g",
    });
  });
});

describe("unassignedFilterLine", () => {
  it("counts the Hα frames Auto leaves to the user", () => {
    const ha = Array.from({ length: 8 }, (_, i) => frame(`ha_000${i}.fits`, { IMAGETYP: "LIGHT", FILTER: "Ha" }));
    expect(unassignedFilterLine(ha, {})).toBe("8 Hα frames not assigned: drizzle them in mono or assign by hand");
  });

  it("names every filter in a fixed order and stops counting a frame once it is assigned", () => {
    const files = [
      frame("l_1.fits", { FILTER: "L" }),
      frame("o_1.fits", { FILTER: "OIII" }),
      frame("o_2.fits", { FILTER: "OIII" }),
      frame("n_1.fits", { FILTER: "NII" }),
      frame("s_1.fits", { FILTER: "SII" }),
      frame("h_1.fits", { FILTER: "Ha" }),
      frame("h_2.fits", { FILTER: "Ha" }),
    ];
    expect(unassignedFilterLine(files, {})).toBe("2 Hα, 2 OIII, 1 SII, 1 NII and 1 L frames not assigned: drizzle each filter in mono or assign by hand");
    expect(unassignedFilterLine(files, { [files[5].path]: "r", [files[6].path]: "r" })).toBe(
      "2 OIII, 1 SII, 1 NII and 1 L frames not assigned: drizzle each filter in mono or assign by hand",
    );
  });

  it("uses the singular for one frame", () => {
    expect(unassignedFilterLine([frame("h_1.fits", { FILTER: "Ha" })], {})).toBe("1 Hα frame not assigned: assign it by hand");
  });

  it("leaves calibration frames, R, G and B frames and frames without a filter out of the count", () => {
    const files = [frame("flat_ha.fits", { IMAGETYP: "FLAT", FILTER: "Ha" }), lightR("frame_0001.fits"), frame("plain.fits")];
    expect(unassignedFilterLine(files, {})).toBeNull();
  });
});

describe("monoDrizzleFilterLabel", () => {
  it("names the filter the frames share, not the channel they sit in", () => {
    const ha = ["ha_1.fits", "ha_2.fits"].map((name) => frame(name, { IMAGETYP: "LIGHT", FILTER: "Ha" }));
    expect(monoDrizzleFilterLabel(ha, "r")).toBe("Ha");
    expect(monoDrizzleFilterLabel([frame("o_1.fits", { FILTER: "OIII" }), frame("o_2.fits", { FILTER: "O3" })], "g")).toBe("OIII");
    expect(monoDrizzleFilterLabel([lightR("frame_0001.fits"), lightR("frame_0002.fits")], "r")).toBe("R");
  });

  it("falls back to the channel when the frames mix filters or name none", () => {
    expect(monoDrizzleFilterLabel([frame("ha_1.fits", { FILTER: "Ha" }), frame("o_1.fits", { FILTER: "OIII" })], "b")).toBe("B");
    expect(monoDrizzleFilterLabel([frame("plain_1.fits"), frame("plain_2.fits")], "g")).toBe("G");
    expect(monoDrizzleFilterLabel([frame("light_0001.fits", { FILTER: "2" }), frame("light_0002.fits", { FILTER: "2" })], "r")).toBe("R");
    expect(monoDrizzleFilterLabel([], "r")).toBe("R");
  });
});
