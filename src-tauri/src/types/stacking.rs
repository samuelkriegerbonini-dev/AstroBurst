use ndarray::Array2;

#[derive(Debug, Clone)]
pub struct StackConfig {
    pub sigma_low: f32,
    pub sigma_high: f32,
    pub max_iterations: usize,
    pub align: bool,
}

impl Default for StackConfig {
    fn default() -> Self {
        Self {
            sigma_low: 3.0,
            sigma_high: 3.0,
            max_iterations: 5,
            align: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StackResult {
    pub image: Array2<f32>,
    pub frame_count: usize,
    pub rejected_pixels: u64,
    pub offsets: Vec<(i32, i32)>,
}

#[derive(Debug, Clone)]
pub struct DrizzleConfig {
    pub scale: f64,
    pub pixfrac: f64,
    pub kernel: DrizzleKernel,
    pub sigma_low: f32,
    pub sigma_high: f32,
    pub sigma_iterations: usize,
    pub align: bool,
}

impl Default for DrizzleConfig {
    fn default() -> Self {
        Self {
            scale: 2.0,
            pixfrac: 0.7,
            kernel: DrizzleKernel::Square,
            sigma_low: 3.0,
            sigma_high: 3.0,
            sigma_iterations: 5,
            align: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DrizzleKernel {
    Square,
    Gaussian,
    Lanczos3,
}

#[derive(Debug, Clone)]
pub struct DrizzleResult {
    pub image: Array2<f32>,
    pub weight_map: Array2<f32>,
    pub frame_count: usize,
    pub output_scale: f64,
    pub input_dims: (usize, usize),
    pub output_dims: (usize, usize),
    pub offsets: Vec<(f64, f64)>,
    pub rejected_pixels: u64,
}

#[derive(Debug, Clone)]
pub struct RLConfig {
    pub iterations: usize,
    pub psf_sigma: f64,
    pub psf_size: usize,
    pub regularization: f64,
    pub deringing: bool,
    pub deringing_threshold: f32,
}

impl Default for RLConfig {
    fn default() -> Self {
        Self {
            iterations: 20,
            psf_sigma: 2.0,
            psf_size: 15,
            regularization: 0.001,
            deringing: true,
            deringing_threshold: 0.1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RLResult {
    pub image: Array2<f32>,
    pub iterations_run: usize,
    pub convergence: f64,
    pub elapsed_ms: u64,
}
