const RENDER_STF_SHADER = `
struct Uniforms {
    data_min: f32,
    data_max: f32,
    shadow: f32,
    midtone: f32,
    highlight: f32,
    tex_w: f32,
    tex_h: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> params: Uniforms;
@group(0) @binding(1) var raw_tex: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let pos = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>( 1.0, -1.0), vec2<f32>(-1.0,  1.0),
        vec2<f32>(-1.0,  1.0), vec2<f32>( 1.0, -1.0), vec2<f32>( 1.0,  1.0)
    );
    let uv = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 0.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(1.0, 0.0)
    );

    var out: VertexOutput;
    out.position = vec4<f32>(pos[vertex_index], 0.0, 1.0);
    out.uv = uv[vertex_index];
    return out;
}

fn mtf(m: f32, x: f32) -> f32 {
    if (x <= 0.0) { return 0.0; }
    if (x >= 1.0) { return 1.0; }
    if (abs(m - 0.5) < 1e-6) { return x; }
    let a = (m - 1.0) * x;
    let b = (2.0 * m - 1.0) * x - m;
    if (abs(b) < 1e-8) { return x; }
    return a / b;
}

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let px = vec2<u32>(
        u32(clamp(uv.x * params.tex_w, 0.0, params.tex_w - 1.0)),
        u32(clamp(uv.y * params.tex_h, 0.0, params.tex_h - 1.0)),
    );
    let val = textureLoad(raw_tex, px, 0).r;

    let norm = (val - params.data_min) / max(params.data_max - params.data_min, 1e-8);

    let range = params.highlight - params.shadow;
    var x = (norm - params.shadow) / max(range, 1e-8);
    x = clamp(x, 0.0, 1.0);

    let pixel_val = mtf(params.midtone, x);

    return vec4<f32>(pixel_val, pixel_val, pixel_val, 1.0);
}
`;

export interface GpuResources {
  device: GPUDevice;
  pipeline: GPURenderPipeline;
  format: GPUTextureFormat;
}

let _gpuSingleton: GpuResources | null = null;
let _gpuInitPromise: Promise<GpuResources | null> | null = null;
let _gpuAvailable: boolean | null = null;

export function getGpuSingleton(): Promise<GpuResources | null> {
  if (_gpuInitPromise) return _gpuInitPromise;
  _gpuInitPromise = (async () => {
    if (!navigator.gpu) {
      _gpuAvailable = false;
      return null;
    }
    try {
      const adapter = await navigator.gpu.requestAdapter();
      if (!adapter) {
        _gpuAvailable = false;
        return null;
      }
      const device = await adapter.requestDevice();
      const format = navigator.gpu.getPreferredCanvasFormat();

      const module = device.createShaderModule({ code: RENDER_STF_SHADER });
      const pipeline = device.createRenderPipeline({
        layout: "auto",
        vertex: { module, entryPoint: "vs_main" },
        fragment: {
          module,
          entryPoint: "fs_main",
          targets: [{ format }],
        },
        primitive: { topology: "triangle-list" },
      });

      device.lost.then(() => {
        _gpuSingleton = null;
        _gpuInitPromise = null;
        _gpuAvailable = null;
      });

      _gpuSingleton = { device, pipeline, format };
      _gpuAvailable = true;
      return _gpuSingleton;
    } catch {
      _gpuAvailable = false;
      return null;
    }
  })();
  return _gpuInitPromise;
}

export function isGpuAvailable(): boolean | null {
  return _gpuAvailable;
}

export function probeGpu(): Promise<GpuResources | null> {
  return getGpuSingleton();
}

export function getGpuState(): GpuResources | null {
  return _gpuSingleton;
}
