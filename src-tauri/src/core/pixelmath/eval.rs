use std::borrow::Cow;
use std::collections::HashMap;

use ndarray::Array2;
use rayon::prelude::*;

use super::parser::{parse, BinaryOp, Expr, Function, UnaryOp};
use super::{PixelMathError, Span};
use crate::math::median::{exact_mad_mut, exact_median_mut};

const CHUNK_PIXELS: usize = 16384;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OutputOptions {
    pub truncate: bool,
    pub rescale: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reducer {
    Mean,
    Med,
    Mdev,
    Sdev,
    Adev,
    Min,
    Max,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    Const(f32),
    Load(usize),
    Reduce(Reducer, usize),
    Neg,
    Invert,
    Not,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
    Abs,
    Sqrt,
    Exp,
    Ln,
    Log10,
    Log2,
    Floor,
    Ceil,
    Round,
    Trunc,
    Sign,
    MinN(usize),
    MaxN(usize),
    Clip,
    Rescale,
    Iif,
}

impl Op {
    fn stack_effect(self) -> (usize, usize) {
        match self {
            Op::Const(_) | Op::Load(_) | Op::Reduce(..) => (0, 1),
            Op::Neg
            | Op::Invert
            | Op::Not
            | Op::Abs
            | Op::Sqrt
            | Op::Exp
            | Op::Ln
            | Op::Log10
            | Op::Log2
            | Op::Floor
            | Op::Ceil
            | Op::Round
            | Op::Trunc
            | Op::Sign => (1, 1),
            Op::Add
            | Op::Sub
            | Op::Mul
            | Op::Div
            | Op::Rem
            | Op::Pow
            | Op::Lt
            | Op::Le
            | Op::Gt
            | Op::Ge
            | Op::Eq
            | Op::Ne
            | Op::And
            | Op::Or => (2, 1),
            Op::MinN(n) | Op::MaxN(n) => (n, 1),
            Op::Clip | Op::Iif => (3, 1),
            Op::Rescale => (5, 1),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Program {
    ops: Vec<Op>,
    slot_names: Vec<String>,
    stack_depth: usize,
}

impl Program {
    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    pub fn slot_names(&self) -> &[String] {
        &self.slot_names
    }

    pub fn stack_depth(&self) -> usize {
        self.stack_depth
    }

    pub fn referenced_slots(&self) -> Vec<bool> {
        let mut used = vec![false; self.slot_names.len()];
        for op in &self.ops {
            match *op {
                Op::Load(slot) | Op::Reduce(_, slot) => {
                    if let Some(flag) = used.get_mut(slot) {
                        *flag = true;
                    }
                }
                _ => {}
            }
        }
        used
    }
}

fn is_slot_identifier(name: &str) -> bool {
    let body = name.strip_prefix('$').unwrap_or(name);
    let mut chars = body.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn validate_slot_names(slot_names: &[String]) -> Result<(), PixelMathError> {
    for (i, name) in slot_names.iter().enumerate() {
        if name.is_empty() {
            return Err(PixelMathError::new(format!("slot {} has an empty name", i)));
        }
        if !is_slot_identifier(name) {
            return Err(PixelMathError::new(format!(
                "invalid slot name '{}': use letters, digits and underscores, starting with a letter",
                name
            )));
        }
        if slot_names[..i].iter().any(|other| other == name) {
            return Err(PixelMathError::new(format!("duplicate slot name '{}'", name)));
        }
    }
    Ok(())
}

fn resolve_symbol(name: &str, span: Span, slot_names: &[String]) -> Result<usize, PixelMathError> {
    slot_names.iter().position(|n| n == name).ok_or_else(|| {
        let available = if slot_names.is_empty() {
            "(none)".to_string()
        } else {
            slot_names.join(", ")
        };
        PixelMathError::at(format!("unknown symbol '{}'; available: {}", name, available), span)
    })
}

fn reducer_for(func: Function, args: &[Expr]) -> Option<Reducer> {
    let single_symbol = args.len() == 1 && matches!(args[0], Expr::Symbol { .. });
    match func {
        Function::Mean => Some(Reducer::Mean),
        Function::Med => Some(Reducer::Med),
        Function::Mdev => Some(Reducer::Mdev),
        Function::Sdev => Some(Reducer::Sdev),
        Function::Adev => Some(Reducer::Adev),
        Function::Min if single_symbol => Some(Reducer::Min),
        Function::Max if single_symbol => Some(Reducer::Max),
        _ => None,
    }
}

fn binary_op(op: BinaryOp) -> Op {
    match op {
        BinaryOp::Add => Op::Add,
        BinaryOp::Sub => Op::Sub,
        BinaryOp::Mul => Op::Mul,
        BinaryOp::Div => Op::Div,
        BinaryOp::Rem => Op::Rem,
        BinaryOp::Pow => Op::Pow,
        BinaryOp::Lt => Op::Lt,
        BinaryOp::Le => Op::Le,
        BinaryOp::Gt => Op::Gt,
        BinaryOp::Ge => Op::Ge,
        BinaryOp::Eq => Op::Eq,
        BinaryOp::Ne => Op::Ne,
        BinaryOp::And => Op::And,
        BinaryOp::Or => Op::Or,
    }
}

fn function_op(func: Function, arg_count: usize) -> Op {
    match func {
        Function::Abs => Op::Abs,
        Function::Sqrt => Op::Sqrt,
        Function::Exp => Op::Exp,
        Function::Ln => Op::Ln,
        Function::Log => Op::Log10,
        Function::Log2 => Op::Log2,
        Function::Pow => Op::Pow,
        Function::Min => Op::MinN(arg_count),
        Function::Max => Op::MaxN(arg_count),
        Function::Floor => Op::Floor,
        Function::Ceil => Op::Ceil,
        Function::Round => Op::Round,
        Function::Trunc => Op::Trunc,
        Function::Sign => Op::Sign,
        Function::Clip => Op::Clip,
        Function::Rescale => Op::Rescale,
        Function::Iif => Op::Iif,
        Function::Pi => Op::Const(std::f32::consts::PI),
        Function::E => Op::Const(std::f32::consts::E),
        Function::Mean | Function::Med | Function::Mdev | Function::Sdev | Function::Adev => {
            unreachable!("reducers are emitted as Op::Reduce")
        }
    }
}

fn emit(expr: &Expr, slot_names: &[String], ops: &mut Vec<Op>) -> Result<(), PixelMathError> {
    match expr {
        Expr::Number(value) => {
            let narrowed = *value as f32;
            if !narrowed.is_finite() {
                return Err(PixelMathError::new(format!(
                    "number {} is outside the range this engine can represent (max 3.4e38)",
                    value
                )));
            }
            ops.push(Op::Const(narrowed));
        }
        Expr::Symbol { name, span } => ops.push(Op::Load(resolve_symbol(name, *span, slot_names)?)),
        Expr::Unary { op, operand } => {
            emit(operand, slot_names, ops)?;
            ops.push(match op {
                UnaryOp::Neg => Op::Neg,
                UnaryOp::Invert => Op::Invert,
                UnaryOp::Not => Op::Not,
            });
        }
        Expr::Binary { op, lhs, rhs } => {
            emit(lhs, slot_names, ops)?;
            emit(rhs, slot_names, ops)?;
            ops.push(binary_op(*op));
        }
        Expr::Call { func, args, .. } => {
            if let Some(reducer) = reducer_for(*func, args) {
                let Expr::Symbol { name, span } = &args[0] else {
                    unreachable!("parser guarantees a symbol argument for reducers");
                };
                ops.push(Op::Reduce(reducer, resolve_symbol(name, *span, slot_names)?));
                return Ok(());
            }
            for arg in args {
                emit(arg, slot_names, ops)?;
            }
            ops.push(function_op(*func, args.len()));
        }
    }
    Ok(())
}

fn required_stack(ops: &[Op]) -> usize {
    let mut depth = 0usize;
    let mut max_depth = 0usize;
    for op in ops {
        let (pops, pushes) = op.stack_effect();
        depth = depth.saturating_sub(pops) + pushes;
        max_depth = max_depth.max(depth);
    }
    max_depth
}

pub fn compile(expr: &str, slot_names: &[String]) -> Result<Program, PixelMathError> {
    validate_slot_names(slot_names)?;
    let ast = parse(expr)?;
    let mut ops = Vec::new();
    emit(&ast, slot_names, &mut ops)?;
    let stack_depth = required_stack(&ops);
    Ok(Program { ops, slot_names: slot_names.to_vec(), stack_depth })
}

pub fn validate(expr: &str, slot_names: &[String]) -> Result<(), PixelMathError> {
    compile(expr, slot_names).map(|_| ())
}

fn collect_finite(values: &[f32]) -> Vec<f32> {
    values.par_iter().copied().filter(|v| v.is_finite()).collect()
}

fn needs_finite_buffer(reducer: Reducer) -> bool {
    matches!(reducer, Reducer::Med | Reducer::Mdev | Reducer::Adev)
}

fn finite_count_and_sum(values: &[f32]) -> (usize, f64) {
    values
        .par_iter()
        .fold(
            || (0usize, 0.0f64),
            |(n, s), &v| if v.is_finite() { (n + 1, s + v as f64) } else { (n, s) },
        )
        .reduce(|| (0usize, 0.0f64), |a, b| (a.0 + b.0, a.1 + b.1))
}

fn reduce_streaming(reducer: Reducer, values: &[f32]) -> f32 {
    match reducer {
        Reducer::Min => {
            let m = values.par_iter().copied().filter(|v| v.is_finite()).reduce(|| f32::INFINITY, f32::min);
            if m.is_finite() {
                m
            } else {
                f32::NAN
            }
        }
        Reducer::Max => {
            let m = values
                .par_iter()
                .copied()
                .filter(|v| v.is_finite())
                .reduce(|| f32::NEG_INFINITY, f32::max);
            if m.is_finite() {
                m
            } else {
                f32::NAN
            }
        }
        Reducer::Mean => {
            let (n, sum) = finite_count_and_sum(values);
            if n == 0 {
                f32::NAN
            } else {
                (sum / n as f64) as f32
            }
        }
        Reducer::Sdev => {
            let (n, sum) = finite_count_and_sum(values);
            if n == 0 {
                return f32::NAN;
            }
            if n < 2 {
                return 0.0;
            }
            let mean = sum / n as f64;
            let sum_sq = values
                .par_iter()
                .filter(|v| v.is_finite())
                .map(|&v| (v as f64 - mean).powi(2))
                .sum::<f64>();
            (sum_sq / (n as f64 - 1.0)).sqrt() as f32
        }
        _ => unreachable!("buffered reducers are handled by reduce_buffered"),
    }
}

fn reduce_buffered(reducer: Reducer, finite: &[f32]) -> f32 {
    if finite.is_empty() {
        return f32::NAN;
    }
    match reducer {
        Reducer::Med => {
            let mut buf = finite.to_vec();
            exact_median_mut(&mut buf) as f32
        }
        Reducer::Mdev => {
            let mut buf = finite.to_vec();
            let median = exact_median_mut(&mut buf) as f32;
            exact_mad_mut(&mut buf, median)
        }
        Reducer::Adev => {
            let mut buf = finite.to_vec();
            let median = exact_median_mut(&mut buf);
            (finite.par_iter().map(|&v| (v as f64 - median).abs()).sum::<f64>() / finite.len() as f64) as f32
        }
        _ => unreachable!("streaming reducers are handled by reduce_streaming"),
    }
}

#[cfg(test)]
fn reduce(reducer: Reducer, values: &[f32]) -> f32 {
    if needs_finite_buffer(reducer) {
        reduce_buffered(reducer, values)
    } else {
        reduce_streaming(reducer, values)
    }
}

fn resolve_reducers(ops: &[Op], inputs: &[&[f32]]) -> Vec<Op> {
    let mut values: HashMap<(Reducer, usize), f32> = HashMap::new();
    let mut finite_cache: HashMap<usize, Vec<f32>> = HashMap::new();
    ops.iter()
        .map(|op| match *op {
            Op::Reduce(reducer, slot) => {
                let value = *values.entry((reducer, slot)).or_insert_with(|| {
                    if needs_finite_buffer(reducer) {
                        let finite = finite_cache.entry(slot).or_insert_with(|| collect_finite(inputs[slot]));
                        reduce_buffered(reducer, finite)
                    } else {
                        reduce_streaming(reducer, inputs[slot])
                    }
                });
                Op::Const(value)
            }
            other => other,
        })
        .collect()
}

#[inline(always)]
fn truthy(v: f32) -> bool {
    v != 0.0 && !v.is_nan()
}

#[inline(always)]
fn flag(b: bool) -> f32 {
    if b {
        1.0
    } else {
        0.0
    }
}

#[inline(always)]
fn nan_min(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else {
        a.min(b)
    }
}

#[inline(always)]
fn nan_max(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else {
        a.max(b)
    }
}

#[inline(always)]
fn sign(v: f32) -> f32 {
    if v == 0.0 {
        0.0
    } else {
        v.signum()
    }
}

#[inline(always)]
fn clip(x: f32, lo: f32, hi: f32) -> f32 {
    if x.is_nan() {
        f32::NAN
    } else if x < lo {
        lo
    } else if x > hi {
        hi
    } else {
        x
    }
}

#[inline(always)]
fn rescale(x: f32, a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    let span = a1 - a0;
    if span == 0.0 {
        return if x.is_nan() { f32::NAN } else { b0 };
    }
    b0 + (x - a0) * (b1 - b0) / span
}

fn run_pixel(ops: &[Op], inputs: &[&[f32]], idx: usize, stack: &mut [f32]) -> f32 {
    let mut sp = 0usize;
    for op in ops {
        match *op {
            Op::Const(c) => {
                stack[sp] = c;
                sp += 1;
            }
            Op::Load(slot) => {
                stack[sp] = inputs[slot][idx];
                sp += 1;
            }
            Op::Reduce(..) => {
                stack[sp] = f32::NAN;
                sp += 1;
            }
            Op::Neg => stack[sp - 1] = -stack[sp - 1],
            Op::Invert => stack[sp - 1] = 1.0 - stack[sp - 1],
            Op::Not => stack[sp - 1] = flag(!truthy(stack[sp - 1])),
            Op::Abs => stack[sp - 1] = stack[sp - 1].abs(),
            Op::Sqrt => stack[sp - 1] = stack[sp - 1].sqrt(),
            Op::Exp => stack[sp - 1] = stack[sp - 1].exp(),
            Op::Ln => stack[sp - 1] = stack[sp - 1].ln(),
            Op::Log10 => stack[sp - 1] = stack[sp - 1].log10(),
            Op::Log2 => stack[sp - 1] = stack[sp - 1].log2(),
            Op::Floor => stack[sp - 1] = stack[sp - 1].floor(),
            Op::Ceil => stack[sp - 1] = stack[sp - 1].ceil(),
            Op::Round => stack[sp - 1] = stack[sp - 1].round(),
            Op::Trunc => stack[sp - 1] = stack[sp - 1].trunc(),
            Op::Sign => stack[sp - 1] = sign(stack[sp - 1]),
            Op::Add => {
                sp -= 1;
                stack[sp - 1] += stack[sp];
            }
            Op::Sub => {
                sp -= 1;
                stack[sp - 1] -= stack[sp];
            }
            Op::Mul => {
                sp -= 1;
                stack[sp - 1] *= stack[sp];
            }
            Op::Div => {
                sp -= 1;
                stack[sp - 1] /= stack[sp];
            }
            Op::Rem => {
                sp -= 1;
                stack[sp - 1] %= stack[sp];
            }
            Op::Pow => {
                sp -= 1;
                stack[sp - 1] = stack[sp - 1].powf(stack[sp]);
            }
            Op::Lt => {
                sp -= 1;
                stack[sp - 1] = flag(stack[sp - 1] < stack[sp]);
            }
            Op::Le => {
                sp -= 1;
                stack[sp - 1] = flag(stack[sp - 1] <= stack[sp]);
            }
            Op::Gt => {
                sp -= 1;
                stack[sp - 1] = flag(stack[sp - 1] > stack[sp]);
            }
            Op::Ge => {
                sp -= 1;
                stack[sp - 1] = flag(stack[sp - 1] >= stack[sp]);
            }
            Op::Eq => {
                sp -= 1;
                stack[sp - 1] = flag(stack[sp - 1] == stack[sp]);
            }
            Op::Ne => {
                sp -= 1;
                stack[sp - 1] = flag(stack[sp - 1] != stack[sp]);
            }
            Op::And => {
                sp -= 1;
                stack[sp - 1] = flag(truthy(stack[sp - 1]) && truthy(stack[sp]));
            }
            Op::Or => {
                sp -= 1;
                stack[sp - 1] = flag(truthy(stack[sp - 1]) || truthy(stack[sp]));
            }
            Op::MinN(n) => {
                let base = sp - n;
                let mut acc = stack[base];
                for &v in &stack[base + 1..sp] {
                    acc = nan_min(acc, v);
                }
                stack[base] = acc;
                sp = base + 1;
            }
            Op::MaxN(n) => {
                let base = sp - n;
                let mut acc = stack[base];
                for &v in &stack[base + 1..sp] {
                    acc = nan_max(acc, v);
                }
                stack[base] = acc;
                sp = base + 1;
            }
            Op::Clip => {
                sp -= 2;
                stack[sp - 1] = clip(stack[sp - 1], stack[sp], stack[sp + 1]);
            }
            Op::Rescale => {
                sp -= 4;
                stack[sp - 1] = rescale(stack[sp - 1], stack[sp], stack[sp + 1], stack[sp + 2], stack[sp + 3]);
            }
            Op::Iif => {
                sp -= 2;
                stack[sp - 1] = if truthy(stack[sp - 1]) { stack[sp] } else { stack[sp + 1] };
            }
        }
    }
    stack[0]
}

fn finite_range(values: &[f32]) -> (f32, f32) {
    values
        .par_iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(|| (f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)))
        .reduce(|| (f32::INFINITY, f32::NEG_INFINITY), |a, b| (a.0.min(b.0), a.1.max(b.1)))
}

fn finish_output(out: &mut [f32], opts: &OutputOptions) {
    let rescale = if opts.rescale {
        let (lo, hi) = finite_range(out);
        if lo.is_finite() {
            Some((lo as f64, hi as f64 - lo as f64))
        } else {
            None
        }
    } else {
        None
    };
    let truncate = opts.truncate;
    out.par_iter_mut().for_each(|v| {
        if v.is_nan() {
            return;
        }
        if v.is_infinite() {
            *v = if truncate {
                if *v > 0.0 {
                    1.0
                } else {
                    0.0
                }
            } else {
                f32::NAN
            };
            return;
        }
        if let Some((lo, range)) = rescale {
            *v = if range > 0.0 { ((*v as f64 - lo) / range) as f32 } else { 0.0 };
        }
        if truncate {
            *v = v.clamp(0.0, 1.0);
        }
    });
}

fn dims_label(dims: (usize, usize)) -> String {
    format!("{}x{}", dims.1, dims.0)
}

pub fn evaluate(
    program: &Program,
    slots: &[&Array2<f32>],
    opts: &OutputOptions,
) -> Result<Array2<f32>, PixelMathError> {
    if slots.len() != program.slot_names.len() {
        return Err(PixelMathError::new(format!(
            "expected {} image(s) for slots [{}] but got {}",
            program.slot_names.len(),
            program.slot_names.join(", "),
            slots.len()
        )));
    }
    let Some(first) = slots.first() else {
        return Err(PixelMathError::new("at least one image slot is required"));
    };
    let dims = first.dim();
    let referenced = program.referenced_slots();
    for (i, slot) in slots.iter().enumerate().skip(1) {
        if !referenced[i] {
            continue;
        }
        if slot.dim() != dims {
            return Err(PixelMathError::new(format!(
                "dimension mismatch: {} is {} but {} is {} (width x height)",
                program.slot_names[0],
                dims_label(dims),
                program.slot_names[i],
                dims_label(slot.dim())
            )));
        }
    }
    let flats: Vec<Cow<[f32]>> = slots
        .iter()
        .enumerate()
        .map(|(i, a)| {
            if !referenced[i] {
                return Cow::Borrowed(&[][..]);
            }
            match a.as_slice() {
                Some(s) => Cow::Borrowed(s),
                None => Cow::Owned(a.iter().copied().collect()),
            }
        })
        .collect();
    let inputs: Vec<&[f32]> = flats.iter().map(|c| c.as_ref()).collect();
    let ops = resolve_reducers(&program.ops, &inputs);
    let depth = program.stack_depth.max(1);
    let mut out = vec![0f32; dims.0 * dims.1];
    out.par_chunks_mut(CHUNK_PIXELS)
        .enumerate()
        .for_each(|(chunk_index, chunk)| {
            let base = chunk_index * CHUNK_PIXELS;
            let mut stack = vec![0f32; depth];
            for (offset, pixel) in chunk.iter_mut().enumerate() {
                *pixel = run_pixel(&ops, &inputs, base + offset, &mut stack);
            }
        });
    finish_output(&mut out, opts);
    Array2::from_shape_vec(dims, out).map_err(|e| PixelMathError::new(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn compiles_to_postfix_with_resolved_slots_and_reducers() {
        let program = compile("$T * (A / med(A)) - min(A)", &names(&["$T", "A"])).unwrap();
        assert_eq!(
            program.ops(),
            &[
                Op::Load(0),
                Op::Load(1),
                Op::Reduce(Reducer::Med, 1),
                Op::Div,
                Op::Mul,
                Op::Reduce(Reducer::Min, 1),
                Op::Sub,
            ]
        );
        assert_eq!(program.stack_depth(), 3);
        assert_eq!(program.slot_names(), &["$T".to_string(), "A".to_string()]);
    }

    #[test]
    fn stack_depth_covers_nary_calls_and_constants() {
        let program = compile("min(1, 2, 3, 4, 5, 6, 7, 8)", &names(&["$T"])).unwrap();
        assert_eq!(program.stack_depth(), 8);
        assert!(matches!(program.ops().last(), Some(Op::MinN(8))));
        let constants = compile("pi() + e()", &names(&["$T"])).unwrap();
        assert_eq!(constants.ops().len(), 3);
    }

    #[test]
    fn reducer_helpers_handle_edge_cases() {
        assert!(reduce(Reducer::Mean, &[]).is_nan());
        assert_eq!(reduce(Reducer::Sdev, &[3.0]), 0.0);
        assert_eq!(reduce(Reducer::Med, &[4.0, 1.0, 3.0, 2.0]), 2.5);
        assert_eq!(reduce(Reducer::Mdev, &[1.0, 2.0, 3.0, 4.0, 100.0]), 1.0);
        assert_eq!(reduce(Reducer::Adev, &[1.0, 3.0, 5.0]), 4.0 / 3.0);
        assert_eq!(reduce(Reducer::Sdev, &[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]), (32.0f64 / 7.0).sqrt() as f32);
    }

    #[test]
    fn rescale_helper_guards_degenerate_source_range() {
        assert_eq!(rescale(3.0, 1.0, 1.0, 0.0, 1.0), 0.0);
        assert!(rescale(f32::NAN, 1.0, 1.0, 0.0, 1.0).is_nan());
        assert_eq!(rescale(2.0, 0.0, 4.0, 10.0, 20.0), 15.0);
    }

    #[test]
    fn slot_identifier_rules() {
        assert!(is_slot_identifier("$T"));
        assert!(is_slot_identifier("ha"));
        assert!(is_slot_identifier("_x1"));
        assert!(!is_slot_identifier("1A"));
        assert!(!is_slot_identifier("$"));
        assert!(!is_slot_identifier("a-b"));
        assert!(!is_slot_identifier(""));
    }
}
