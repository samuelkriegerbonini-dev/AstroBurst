import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement, ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const backend = vi.hoisted(() => ({ geometry: vi.fn() }));

vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../services/geometry", () => ({ getObservationGeometry: backend.geometry }));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, findElement, textOf } from "../../../test/elementTree";
import { localSmallButtonLines } from "../../../test/panelRules";
import { CopyCsvButton, ErrorAlert } from "../../ui";
import { NO_SITE_HINT, NO_TARGET_HINT, geometryText } from "../../../utils/observationGeometry";
import type { ObservationGeometryResult } from "../../../shared/types/geometry";
import ObservationGeometryPanel from "../ObservationGeometryPanel";
import source from "../ObservationGeometryPanel.tsx?raw";

type Props = Record<string, unknown>;

const RESULT: ObservationGeometryResult = {
  geometry: {
    jd_utc: 2461120.923611,
    jd_tt: 2461120.924412,
    jd_tdb: 2461120.924412,
    bjd_tdb: 2461120.9251234,
    hjd_utc: 2461120.9243211,
    bjd_source: "computed",
    lst_deg: 197.693195,
    hour_angle_deg: 15,
    altitude_deg: 70.1812345,
    azimuth_deg: 180.00005,
    airmass_computed: 1.0626,
    airmass_formula: "Kasten & Young 1989",
    parallactic_angle_deg: -12.3456789,
    sun_altitude_deg: -40.5,
    moon_altitude_deg: 12.25,
    moon_illumination: 0.6786,
    moon_separation_deg: 91.12345,
    time_scale_notes: ["time: DATE-OBS + EXPTIME/2 (600 s)", "TIMESYS absent: UTC assumed"],
  },
  target: { ra_deg: 150, dec_deg: 2, source: "WCS at image centre (49.5, 49.5)" },
  site: { lon_deg: -155.47, lat_deg: 19.82, height_m: 0, source: "SITELAT/SITELONG" },
  time_source: "DATE-OBS + EXPTIME/2 (600 s)",
  airmass_header: 1.07,
  notes: ["site from SITELAT/SITELONG: lon -155.47000 lat 19.82000 height 0 m (east-positive longitude)"],
  elapsed_ms: 2,
};

const panel = (ObservationGeometryPanel as unknown as { type: (p: Props) => ReactNode }).type;

async function show(): Promise<ReactNode> {
  hooks.render(panel, { filePath: "C:/data/frame.fits" });
  await new Promise((r) => setTimeout(r, 0));
  return hooks.render(panel, { filePath: "C:/data/frame.fits" }) as ReactNode;
}

function withProp(tree: ReactNode, name: string): ReactElement<Props> | undefined {
  return findElement<Props>(tree, (e) => e.props[name] === true);
}

beforeEach(() => {
  hooks.mount();
  backend.geometry.mockReset().mockResolvedValue(RESULT);
});

afterEach(() => hooks.mount());

describe("ObservationGeometryPanel", () => {
  it("prints the target and the site in use as text", async () => {
    const tree = await show();
    const target = withProp(tree, "data-geometry-target");
    const site = withProp(tree, "data-geometry-site");
    expect(target && textOf(target)).toBe("RA 150.000000 deg, Dec 2.000000 deg, from WCS at image centre (49.5, 49.5)");
    expect(target?.props.className).toBe("text-[9px] text-zinc-500 break-words");
    expect(site && textOf(site)).toBe("lat 19.8200 deg, lon -155.4700 deg (E+), height 0 m, from SITELAT/SITELONG");
  });

  it("keeps the two hints for a file with no target and no site", async () => {
    backend.geometry.mockResolvedValue({ ...RESULT, target: null, site: null });
    const tree = await show();
    expect(textOf(tree)).toContain(NO_TARGET_HINT);
    expect(textOf(tree)).toContain(NO_SITE_HINT);
    expect(withProp(tree, "data-geometry-target")).toBeUndefined();
    expect(withProp(tree, "data-geometry-site")).toBeUndefined();
  });

  it("draws the BJD source chip at the size of its row", async () => {
    const tree = await show();
    const row = findElement<Props>(tree, (e) => e.props.chip === "computed");
    expect(row).toBeDefined();
    const markup = renderToStaticMarkup(row as ReactElement);
    expect(markup).toContain("text-[9px]");
    expect(markup).not.toContain("text-[8px]");
  });

  it("confirms the copy through the shared button and shows a refused clipboard write", async () => {
    const tree = await show();
    const button = findElement<Props>(tree, (e) => e.type === CopyCsvButton);
    expect(button).toBeDefined();
    const props = (button as ReactElement<Props>).props;
    expect(props.label).toBe("Copy");
    expect(props.title).toBe("Copy the geometry values as text");
    expect((props.text as () => string | null)()).toBe(geometryText(RESULT));
    (props.onError as (m: string) => void)("Clipboard copy failed: Document is not focused.");
    const after = hooks.render(panel, { filePath: "C:/data/frame.fits" }) as ReactNode;
    const alert = elementsOf<Props>(after).find((e) => e.type === ErrorAlert);
    expect(alert?.props.message).toBe("Clipboard copy failed: Document is not focused.");
  });

  it("keeps no private copy of the small button class", () => {
    expect(localSmallButtonLines(source)).toEqual([]);
    expect(source).toMatch(/import \{[^}]*\bSMALL_BUTTON_CLASS\b[^}]*\} from "\.\.\/ui";/);
  });
});
