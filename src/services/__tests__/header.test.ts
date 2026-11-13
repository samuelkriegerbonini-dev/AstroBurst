import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock }));

import { getFullHeader, type FitsExtension, type NarrowbandFilterDetection, type SuggestedChannelDetection } from "../header";
import type { HeaderData } from "../../shared/types/header";
import type { PlaneInfo } from "../../shared/types/fits.types";
import type { BandPlaneEntry, BandPlaneInfo, LayerRole } from "../../shared/types/layers";
import { assignableChannel } from "../../utils/channelMapping";

describe("narrowband payload types", () => {
  it("palette detection declares only the FilterDetection fields Rust serializes", () => {
    expectTypeOf<keyof SuggestedChannelDetection>().toEqualTypeOf<
      "filter" | "confidence" | "matched_keyword" | "matched_value"
    >();
  });

  it("per-file detection keeps the hubble_channel that detection_json emits", () => {
    expectTypeOf<NarrowbandFilterDetection>().toHaveProperty("hubble_channel");
  });

  it("types the header explorer's hubble_channel as nullable, since a palette may map a filter to no channel", () => {
    expectTypeOf<NonNullable<HeaderData["filter_detection"]>["hubble_channel"]>().toEqualTypeOf<string | null>();
  });

  it("types the role and the unit of an extension as optional", () => {
    expectTypeOf<FitsExtension["role"]>().toEqualTypeOf<LayerRole | undefined>();
    expectTypeOf<FitsExtension["bunit"]>().toEqualTypeOf<string | null | undefined>();
  });

  it("types the bands of an extension and the band of the plane description", () => {
    expectTypeOf<FitsExtension["bands"]>().toEqualTypeOf<BandPlaneEntry[] | undefined>();
    expectTypeOf<PlaneInfo["band_plane"]>().toEqualTypeOf<BandPlaneInfo | null | undefined>();
  });
});

describe("getFullHeader", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    typedInvokeMock.mockResolvedValue({});
  });

  it("sends the selected narrowband palette so the channel follows it", async () => {
    await getFullHeader("/sii.fits", "HOO");
    expect(typedInvokeMock).toHaveBeenCalledWith("get_full_header", { path: "/sii.fits", palette: "HOO" });
  });

  it("sends null when no palette is chosen, which the backend reads as SHO", async () => {
    await getFullHeader("/ha.fits");
    expect(typedInvokeMock).toHaveBeenCalledWith("get_full_header", { path: "/ha.fits", palette: null });
  });
});

describe("assignableChannel", () => {
  it("offers an assignment only for a channel the palette maps the filter to", () => {
    expect(assignableChannel("G")).toBe("G");
    expect(assignableChannel(null)).toBeNull();
    expect(assignableChannel(undefined)).toBeNull();
  });
});
