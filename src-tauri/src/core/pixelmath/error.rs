#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub len: usize,
}

impl Span {
    pub fn new(start: usize, len: usize) -> Self {
        Self { start, len }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PixelMathError {
    pub message: String,
    pub position: Option<usize>,
    pub length: Option<usize>,
}

impl PixelMathError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            position: None,
            length: None,
        }
    }

    pub fn at(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            position: Some(span.start),
            length: Some(span.len.max(1)),
        }
    }
}
