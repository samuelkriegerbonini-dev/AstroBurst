export const STF_SHADER = /* wgsl */ `
struct Uniforms {
  width:     u32,
  height:    u32,
  data_min:  f32,
  data_max:  f32,
  shadow:    f32,
  midtone:   f32,
  highlight: f32,
  _pad:      f32,
};

@group(0) @binding(0) var<uniform> params: Uniforms;
@group(0) @binding(1) var<storage, read> pixels: array<f32>;
@group(0) @binding(2) var<storage, read_write> output: array<u32>;

fn mtf(x: f32, m: f32) -> f32 {
  if (x <= 0.0) { return 0.0; }
  if (x >= 1.0) { return 1.0; }
  return (m - 1.0) * x / ((2.0 * m - 1.0) * x - m);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let idx = gid.x;
  let total = params.width * params.height;
  if (idx >= total) { return; }

  let raw = pixels[idx];
  var val: f32 = 0.0;

  if (raw == raw) {
    let range = max(params.data_max - params.data_min, 1e-20);
    let norm = clamp((raw - params.data_min) / range, 0.0, 1.0);
    let clip_range = max(params.highlight - params.shadow, 1e-10);
    let clipped = clamp((norm - params.shadow) / clip_range, 0.0, 1.0);
    val = mtf(clipped, params.midtone);
  }

  let byte_val = u32(clamp(val * 255.0, 0.0, 255.0));
  output[idx] = byte_val | (byte_val << 8u) | (byte_val << 16u) | (255u << 24u);
}
`;
