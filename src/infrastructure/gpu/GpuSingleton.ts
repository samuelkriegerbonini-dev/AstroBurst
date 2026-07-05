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

fn is_nan_bits(v: f32) -> bool {
    let bits = bitcast<u32>(v);
    return (bits & 0x7F800000u) == 0x7F800000u && (bits & 0x007FFFFFu) != 0u;
}

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let px = vec2<u32>(
        u32(clamp(uv.x * params.tex_w, 0.0, params.tex_w - 1.0)),
        u32(clamp(uv.y * params.tex_h, 0.0, params.tex_h - 1.0)),
    );
    let val = textureLoad(raw_tex, px, 0).r;
    if (is_nan_bits(val)) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    let norm = (val - params.data_min) / max(params.data_max - params.data_min, 1e-8);

    let range = params.highlight - params.shadow;
    var x = (norm - params.shadow) / max(range, 1e-8);
    x = clamp(x, 0.0, 1.0);

    let pixel_val = mtf(params.midtone, x);

    return vec4<f32>(pixel_val, pixel_val, pixel_val, 1.0);
}
`;

const RENDER_STF_RGB_SHADER = `
struct Uniforms {
    ch_r: vec4<f32>,
    ch_g: vec4<f32>,
    ch_b: vec4<f32>,
    hi: vec4<f32>,
    dims: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Uniforms;
@group(0) @binding(1) var tex_r: texture_2d<f32>;
@group(0) @binding(2) var tex_g: texture_2d<f32>;
@group(0) @binding(3) var tex_b: texture_2d<f32>;

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

fn is_nan_bits(v: f32) -> bool {
    let bits = bitcast<u32>(v);
    return (bits & 0x7F800000u) == 0x7F800000u && (bits & 0x007FFFFFu) != 0u;
}

// c = (data_min, data_max, shadow, midtone); applies the same normalize + STF as the mono shader.
fn stf_channel(c: vec4<f32>, high: f32, val: f32) -> f32 {
    if (is_nan_bits(val)) { return 0.0; }
    let norm = (val - c.x) / max(c.y - c.x, 1e-8);
    let range = high - c.z;
    var x = (norm - c.z) / max(range, 1e-8);
    x = clamp(x, 0.0, 1.0);
    return mtf(c.w, x);
}

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let px = vec2<u32>(
        u32(clamp(uv.x * params.dims.x, 0.0, params.dims.x - 1.0)),
        u32(clamp(uv.y * params.dims.y, 0.0, params.dims.y - 1.0)),
    );
    let r = stf_channel(params.ch_r, params.hi.x, textureLoad(tex_r, px, 0).r);
    let g = stf_channel(params.ch_g, params.hi.y, textureLoad(tex_g, px, 0).r);
    let b = stf_channel(params.ch_b, params.hi.z, textureLoad(tex_b, px, 0).r);
    return vec4<f32>(r, g, b, 1.0);
}
`;

export interface GpuResources {
  device: GPUDevice;
  pipeline: GPURenderPipeline;
  rgbPipeline: GPURenderPipeline;
  format: GPUTextureFormat;
}

let _gpuSingleton: GpuResources | null = null;
let _gpuInitPromise: Promise<GpuResources | null> | null = null;
let _gpuAvailable: boolean | null = null;
let _gpuReason: string | null = null;
let _gpuGeneration = 0;

type GpuLostListener = () => void;
const _gpuLostListeners = new Set<GpuLostListener>();

export function onGpuLost(listener: GpuLostListener): () => void {
  _gpuLostListeners.add(listener);
  return () => {
    _gpuLostListeners.delete(listener);
  };
}

function handleGpuLost(generation: number, reason: string, uiReason: string): void {
  if (generation !== _gpuGeneration) return;
  if (_gpuSingleton === null && _gpuInitPromise === null) return;
  console.warn(`[AstroBurst] WebGPU device unusable (${reason}); switching to CPU rendering.`);
  const device = _gpuSingleton?.device ?? null;
  _gpuSingleton = null;
  _gpuInitPromise = null;
  _gpuAvailable = null;
  _gpuReason = uiReason;
  try {
    device?.destroy();
  } catch {
  }
  for (const listener of _gpuLostListeners) {
    try {
      listener();
    } catch {
    }
  }
}

export function getGpuSingleton(): Promise<GpuResources | null> {
  if (_gpuInitPromise) return _gpuInitPromise;
  const generation = ++_gpuGeneration;
  _gpuInitPromise = (async () => {
    if (!navigator.gpu) {
      _gpuAvailable = false;
      _gpuReason = "WebGPU not supported by this browser";
      return null;
    }
    try {
      const adapter = await navigator.gpu.requestAdapter();
      if (!adapter) {
        _gpuAvailable = false;
        _gpuReason = "No compatible GPU adapter found";
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

      const rgbModule = device.createShaderModule({ code: RENDER_STF_RGB_SHADER });
      const rgbPipeline = device.createRenderPipeline({
        layout: "auto",
        vertex: { module: rgbModule, entryPoint: "vs_main" },
        fragment: {
          module: rgbModule,
          entryPoint: "fs_main",
          targets: [{ format }],
        },
        primitive: { topology: "triangle-list" },
      });

      device.addEventListener("uncapturederror", (e) => {
        const error = (e as GPUUncapturedErrorEvent).error;
        console.error("[AstroBurst] WebGPU uncaptured error:", error);
        if (error instanceof GPUValidationError) return;
        handleGpuLost(generation, "uncaptured error", "GPU error — using CPU");
      });

      device.lost.then((info) => {
        handleGpuLost(generation, `device lost: ${info?.reason ?? "unknown"}`, "GPU device lost — using CPU");
      });

      _gpuSingleton = { device, pipeline, rgbPipeline, format };
      _gpuAvailable = true;
      _gpuReason = null;
      return _gpuSingleton;
    } catch {
      _gpuAvailable = false;
      _gpuReason = "GPU initialization failed";
      return null;
    }
  })();
  return _gpuInitPromise;
}

export function isGpuAvailable(): boolean | null {
  return _gpuAvailable;
}

export function getGpuReason(): string | null {
  return _gpuReason;
}

export function probeGpu(): Promise<GpuResources | null> {
  return getGpuSingleton();
}

export function getGpuState(): GpuResources | null {
  return _gpuSingleton;
}
