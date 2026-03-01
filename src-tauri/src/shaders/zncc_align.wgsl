
struct Uniforms {
    ref_width:   u32,
    ref_height:  u32,
    tgt_width:   u32,
    tgt_height:  u32,
    roi_x:       u32,
    roi_y:       u32,
    roi_w:       u32,
    roi_h:       u32,
    max_shift:   i32,
    search_size: u32,
    _pad0:       u32,
    _pad1:       u32,
}

struct ZnccResult {
    best_dx:     i32,
    best_dy:     i32,
    best_score:  f32,
    _pad:        u32,
}


@group(0) @binding(0) var<storage, read> ref_image: array<f32>;
@group(0) @binding(1) var<storage, read> tgt_image: array<f32>;


@group(1) @binding(0) var<uniform> params: Uniforms;



@group(2) @binding(0) var<storage, read_write> scores: array<f32>;
@group(2) @binding(1) var<storage, read_write> result: ZnccResult;


const WG_SIZE: u32 = 256u;

var<workgroup> sh_num:   array<f32, 256>;
var<workgroup> sh_r_var: array<f32, 256>;
var<workgroup> sh_t_var: array<f32, 256>;
var<workgroup> sh_r_sum: array<f32, 256>;
var<workgroup> sh_t_sum: array<f32, 256>;
var<workgroup> sh_count: array<u32,  256>;

fn ref_pixel(x: u32, y: u32) -> f32 {
    if x >= params.ref_width || y >= params.ref_height {
        return 0.0;
    }
    return ref_image[y * params.ref_width + x];
}

fn tgt_pixel(x: i32, y: i32) -> f32 {
    if x < 0 || y < 0 || u32(x) >= params.tgt_width || u32(y) >= params.tgt_height {
        return 0.0;
    }
    return tgt_image[u32(y) * params.tgt_width + u32(x)];
}

fn is_valid(v: f32) -> bool {
    return v > 1e-7 && !isNan(v) && !isInf(v);
}

fn isNan(v: f32) -> bool {
    return v != v;
}

fn isInf(v: f32) -> bool {
    return abs(v) > 3.4e+38;
}





@compute @workgroup_size(256, 1, 1)
fn zncc_compute(
    @builtin(workgroup_id)        wg_id:    vec3<u32>,
    @builtin(local_invocation_id) local_id: vec3<u32>,
) {
    let shift_idx_y = i32(wg_id.x) / i32(params.search_size);
    let shift_idx_x = i32(wg_id.x) % i32(params.search_size);
    let dy = shift_idx_y - params.max_shift;
    let dx = shift_idx_x - params.max_shift;

    let tid = local_id.x;
    let roi_pixels = params.roi_w * params.roi_h;


    var local_r_sum: f32 = 0.0;
    var local_t_sum: f32 = 0.0;
    var local_count: u32 = 0u;

    for (var i = tid; i < roi_pixels; i = i + WG_SIZE) {
        let rx = params.roi_x + (i % params.roi_w);
        let ry = params.roi_y + (i / params.roi_w);

        let rv = ref_pixel(rx, ry);
        let tx = i32(rx) + dx;
        let ty = i32(ry) + dy;
        let tv = tgt_pixel(tx, ty);

        if is_valid(rv) && is_valid(tv) {
            local_r_sum += rv;
            local_t_sum += tv;
            local_count += 1u;
        }
    }

    sh_r_sum[tid] = local_r_sum;
    sh_t_sum[tid] = local_t_sum;
    sh_count[tid] = local_count;
    workgroupBarrier();

    
    for (var stride = WG_SIZE / 2u; stride > 0u; stride = stride / 2u) {
        if tid < stride {
            sh_r_sum[tid] += sh_r_sum[tid + stride];
            sh_t_sum[tid] += sh_t_sum[tid + stride];
            sh_count[tid] += sh_count[tid + stride];
        }
        workgroupBarrier();
    }

    let total_count = sh_count[0];
    if total_count < 10u {
        if tid == 0u {
            scores[wg_id.x] = -2.0;
        }
        return;
    }

    let r_mean = sh_r_sum[0] / f32(total_count);
    let t_mean = sh_t_sum[0] / f32(total_count);
    workgroupBarrier();

    
    var local_num:   f32 = 0.0;
    var local_r_var: f32 = 0.0;
    var local_t_var: f32 = 0.0;

    for (var i = tid; i < roi_pixels; i = i + WG_SIZE) {
        let rx = params.roi_x + (i % params.roi_w);
        let ry = params.roi_y + (i / params.roi_w);

        let rv = ref_pixel(rx, ry);
        let tx = i32(rx) + dx;
        let ty = i32(ry) + dy;
        let tv = tgt_pixel(tx, ty);

        if is_valid(rv) && is_valid(tv) {
            let rd = rv - r_mean;
            let td = tv - t_mean;
            local_num   += rd * td;
            local_r_var += rd * rd;
            local_t_var += td * td;
        }
    }

    sh_num[tid]   = local_num;
    sh_r_var[tid] = local_r_var;
    sh_t_var[tid] = local_t_var;
    workgroupBarrier();

    for (var stride = WG_SIZE / 2u; stride > 0u; stride = stride / 2u) {
        if tid < stride {
            sh_num[tid]   += sh_num[tid + stride];
            sh_r_var[tid] += sh_r_var[tid + stride];
            sh_t_var[tid] += sh_t_var[tid + stride];
        }
        workgroupBarrier();
    }

    if tid == 0u {
        let denom = sqrt(sh_r_var[0] * sh_t_var[0]);
        var zncc: f32 = -2.0;
        if denom > 1e-10 {
            zncc = sh_num[0] / denom;
        }
        scores[wg_id.x] = zncc;
    }
}


@compute @workgroup_size(256, 1, 1)
fn find_best_offset(
    @builtin(local_invocation_id) local_id: vec3<u32>,
) {
    let tid = local_id.x;
    let total = params.search_size * params.search_size;

    var best_idx:   u32 = 0u;
    var best_score: f32 = -2.0;

    for (var i = tid; i < total; i = i + WG_SIZE) {
        let s = scores[i];
        if s > best_score {
            best_score = s;
            best_idx = i;
        }
    }

    sh_num[tid] = best_score;
    sh_count[tid] = best_idx;
    workgroupBarrier();

    for (var stride = WG_SIZE / 2u; stride > 0u; stride = stride / 2u) {
        if tid < stride {
            if sh_num[tid + stride] > sh_num[tid] {
                sh_num[tid] = sh_num[tid + stride];
                sh_count[tid] = sh_count[tid + stride];
            }
        }
        workgroupBarrier();
    }

    if tid == 0u {
        let idx = sh_count[0];
        let iy = i32(idx) / i32(params.search_size);
        let ix = i32(idx) % i32(params.search_size);
        result.best_dy = iy - params.max_shift;
        result.best_dx = ix - params.max_shift;
        result.best_score = sh_num[0];
    }
}
