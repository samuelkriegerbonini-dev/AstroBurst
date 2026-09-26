import { describe, it, expect } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ToolHostContext, useToolHost } from "../ToolHostContext";

function Probe() {
  const { active, gpuDisplay } = useToolHost();
  return createElement("span", null, `active=${active} gpu=${gpuDisplay}`);
}

describe("ToolHostContext", () => {
  it("defaults to an active tool over the GPU display so unwrapped panels behave as before", () => {
    expect(renderToStaticMarkup(createElement(Probe))).toBe("<span>active=true gpu=true</span>");
  });

  it("passes the host state down to the tool body", () => {
    const tree = createElement(ToolHostContext.Provider, { value: { active: false, gpuDisplay: false } }, createElement(Probe));
    expect(renderToStaticMarkup(tree)).toBe("<span>active=false gpu=false</span>");
  });
});
