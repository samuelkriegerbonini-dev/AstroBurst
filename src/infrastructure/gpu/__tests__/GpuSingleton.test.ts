import { describe, it, expect } from "vitest";
import { RENDER_STF_SHADER, STF_UNIFORM_WORD } from "../GpuSingleton";

interface UniformField {
  name: string;
  type: string;
}

function uniformFields(shader: string): UniformField[] {
  const block = /struct Uniforms \{([\s\S]*?)\};/.exec(shader);
  if (!block) throw new Error("no Uniforms struct in the shader");
  return block[1]
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
    .map((line) => {
      const m = /^(\w+):\s*(\w+),?$/.exec(line);
      if (!m) throw new Error(`unparsed uniform field: ${line}`);
      return { name: m[1], type: m[2] };
    });
}

function fragmentBody(shader: string): string {
  const start = shader.indexOf("fn fs_main(");
  if (start < 0) throw new Error("no fs_main in the shader");
  return shader.slice(start);
}

describe("RENDER_STF_SHADER uniform layout", () => {
  const fields = uniformFields(RENDER_STF_SHADER);

  it("is sixteen 4-byte words: twelve f32 then four u32, 64 bytes as the renderer allocates", () => {
    expect(fields).toHaveLength(16);
    expect(fields.slice(0, 12).map((f) => f.type)).toEqual(Array(12).fill("f32"));
    expect(fields.slice(12).map((f) => f.type)).toEqual(Array(4).fill("u32"));
  });

  it("declares every named word at the index fillUniforms writes", () => {
    for (const [name, index] of Object.entries(STF_UNIFORM_WORD)) {
      expect(fields[index]?.name, `word ${index}`).toBe(name);
    }
    expect(Object.keys(STF_UNIFORM_WORD)).toHaveLength(14);
  });

  it("carries the no-data colour in words 9 to 11 where the padding words used to be", () => {
    expect(fields.slice(9, 12).map((f) => f.name)).toEqual(["nodata_r", "nodata_g", "nodata_b"]);
    expect(STF_UNIFORM_WORD.nodata_r).toBe(9);
    expect(STF_UNIFORM_WORD.nodata_b).toBe(11);
    expect(fields.some((f) => /^_pad[012]$/.test(f.name))).toBe(false);
  });
});

describe("RENDER_STF_SHADER fragment", () => {
  const body = fragmentBody(RENDER_STF_SHADER);

  it("returns the no-data colour on padding before the range, stretch and invert math", () => {
    const guard = body.indexOf("if (is_padding_bits(val))");
    const ret = body.indexOf("return vec4<f32>(params.nodata_r, params.nodata_g, params.nodata_b, 1.0);");
    const range = body.indexOf("params.vmax - params.vmin");
    const stretch = body.indexOf("stretch(n)");
    const invert = body.indexOf("params.invert == 1u");
    const lut = body.indexOf("textureLoad(lut_tex");
    expect(guard).toBeGreaterThan(0);
    expect(ret).toBeGreaterThan(guard);
    expect(range).toBeGreaterThan(ret);
    expect(stretch).toBeGreaterThan(range);
    expect(invert).toBeGreaterThan(stretch);
    expect(lut).toBeGreaterThan(invert);
  });

  it("classifies NaN, both infinities and exact zero as padding", () => {
    expect(RENDER_STF_SHADER).toContain("(bits & 0x7F800000u) == 0x7F800000u || (bits & 0x7FFFFFFFu) == 0u");
  });

  it("reads the LUT row through a 256-entry texture, the first 1024 bytes of the uploaded array", () => {
    expect(body).toContain("textureLoad(lut_tex, vec2<u32>(idx, 0u), 0).rgb");
    expect(RENDER_STF_SHADER).toContain("var lut_tex: texture_2d<f32>");
  });
});
