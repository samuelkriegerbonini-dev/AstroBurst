import { beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import type { ProcessedFile } from "../../../../shared/types";

vi.mock("react", async (importOriginal) => (await import("../../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../../hooks/useFileIngest", () => ({ ingestFiles: vi.fn() }));
vi.mock("../../../../hooks/usePointingOverlap", () => ({ usePointingOverlap: () => ({ disjointPairs: [], checking: false }) }));

import { hooks } from "../../../../test/hookRuntime";
import { INITIAL_STATE } from "../../../../utils/wizard";
import ChannelStep from "../ChannelStep";

type AnyProps = Record<string, unknown> & { children?: ReactNode };
const step = ChannelStep as unknown as (props: Parameters<typeof ChannelStep>[0]) => ReactNode;

function nodes(node: ReactNode): ReactElement<AnyProps>[] {
  if (Array.isArray(node)) return node.flatMap(nodes);
  if (!isValidElement<AnyProps>(node)) return [];
  return [node, ...nodes(node.props.children)];
}

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  return isValidElement<AnyProps>(node) ? textOf(node.props.children) : "";
}

function frame(name: string, header: Record<string, string>): ProcessedFile {
  const path = `C:/ngc7000/${name}`;
  return {
    id: name,
    name,
    path,
    sourcePath: path,
    imageRef: null,
    size: 1,
    status: "done",
    result: { png_path: `C:/out/${name}.png`, previewUrl: `asset://${name}.png`, dimensions: [8, 8], elapsed_ms: 1, header },
    error: null,
    startedAt: 1,
    finishedAt: 2,
  };
}

const LIGHT = frame("light_ha_1.fits", { IMAGETYP: "LIGHT", FILTER: "Ha" });
const FLAT = frame("flat_ha_1.fits", { IMAGETYP: "FLAT", FILTER: "Ha" });
const DARK = frame("dark_300s_1.fits", { IMAGETYP: "DARK", FILTER: "Ha" });

function pills(): Map<string, ReactElement<AnyProps>> {
  const tree = hooks.render(step, { state: INITIAL_STATE, doneFiles: [LIGHT, FLAT, DARK], onBinsChange: () => {} }) as ReactNode;
  return new Map(nodes(tree).filter((el) => el.props.draggable === true && typeof el.key === "string").map((el) => [el.key as string, el]));
}

describe("ChannelStep Unassigned pills", () => {
  beforeEach(() => hooks.mount());

  it("give a calibration frame its kind and no channel hint", () => {
    const found = pills();
    const flat = found.get(FLAT.path);
    const dark = found.get(DARK.path);
    expect(textOf(flat?.props.children)).toBe("flat_ha_1Haflat");
    expect(flat?.props.title).toBe(`${FLAT.path} [Ha] (flat)`);
    expect(textOf(dark?.props.children)).toBe("dark_300s_1Hadark");
    expect(String(dark?.props.title)).not.toContain("→");
  });

  it("keep the channel hint on a light", () => {
    const light = pills().get(LIGHT.path);
    expect(textOf(light?.props.children)).toBe("light_ha_1Ha→ha");
    expect(light?.props.title).toBe(`${LIGHT.path} [Ha] → ha`);
  });
});
