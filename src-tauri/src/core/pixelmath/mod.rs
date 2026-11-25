pub mod eval;
pub mod lexer;
pub mod parser;
mod error;

#[cfg(test)]
mod tests;

pub use eval::{compile, evaluate, evaluate_unfinished, finish_plane, validate, OutputOptions, Program};
pub use error::{PixelMathError, Span};
