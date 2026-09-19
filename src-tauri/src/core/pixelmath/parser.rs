use super::lexer::{tokenize, Token, TokenKind};
use super::{PixelMathError, Span};

const MAX_NESTING: usize = 256;
const PREFIX_BINDING_POWER: u8 = 13;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Invert,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Function {
    Abs,
    Sqrt,
    Exp,
    Ln,
    Log,
    Log2,
    Pow,
    Min,
    Max,
    Floor,
    Ceil,
    Round,
    Trunc,
    Sign,
    Clip,
    Rescale,
    Iif,
    Pi,
    E,
    Mean,
    Med,
    Mdev,
    Sdev,
    Adev,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arity {
    Exact(usize),
    AtLeast(usize),
}

impl Function {
    pub const ALL: [Function; 24] = [
        Function::Abs,
        Function::Sqrt,
        Function::Exp,
        Function::Ln,
        Function::Log,
        Function::Log2,
        Function::Pow,
        Function::Min,
        Function::Max,
        Function::Floor,
        Function::Ceil,
        Function::Round,
        Function::Trunc,
        Function::Sign,
        Function::Clip,
        Function::Rescale,
        Function::Iif,
        Function::Pi,
        Function::E,
        Function::Mean,
        Function::Med,
        Function::Mdev,
        Function::Sdev,
        Function::Adev,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Function::Abs => "abs",
            Function::Sqrt => "sqrt",
            Function::Exp => "exp",
            Function::Ln => "ln",
            Function::Log => "log",
            Function::Log2 => "log2",
            Function::Pow => "pow",
            Function::Min => "min",
            Function::Max => "max",
            Function::Floor => "floor",
            Function::Ceil => "ceil",
            Function::Round => "round",
            Function::Trunc => "trunc",
            Function::Sign => "sign",
            Function::Clip => "clip",
            Function::Rescale => "rescale",
            Function::Iif => "iif",
            Function::Pi => "pi",
            Function::E => "e",
            Function::Mean => "mean",
            Function::Med => "med",
            Function::Mdev => "mdev",
            Function::Sdev => "sdev",
            Function::Adev => "adev",
        }
    }

    pub fn from_name(name: &str) -> Option<Function> {
        Function::ALL.iter().copied().find(|f| f.name() == name)
    }

    pub fn arity(self) -> Arity {
        match self {
            Function::Pi | Function::E => Arity::Exact(0),
            Function::Pow => Arity::Exact(2),
            Function::Clip | Function::Iif => Arity::Exact(3),
            Function::Rescale => Arity::Exact(5),
            Function::Min | Function::Max => Arity::AtLeast(1),
            _ => Arity::Exact(1),
        }
    }

    pub fn is_reducer(self) -> bool {
        matches!(
            self,
            Function::Mean | Function::Med | Function::Mdev | Function::Sdev | Function::Adev
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    Symbol { name: String, span: Span },
    Unary { op: UnaryOp, operand: Box<Expr> },
    Binary { op: BinaryOp, lhs: Box<Expr>, rhs: Box<Expr> },
    Call { func: Function, args: Vec<Expr>, span: Span },
}

fn binary_binding(kind: TokenKind) -> Option<(BinaryOp, u8, u8)> {
    Some(match kind {
        TokenKind::OrOr => (BinaryOp::Or, 1, 2),
        TokenKind::AndAnd => (BinaryOp::And, 3, 4),
        TokenKind::EqEq => (BinaryOp::Eq, 5, 6),
        TokenKind::Ne => (BinaryOp::Ne, 5, 6),
        TokenKind::Lt => (BinaryOp::Lt, 7, 8),
        TokenKind::Le => (BinaryOp::Le, 7, 8),
        TokenKind::Gt => (BinaryOp::Gt, 7, 8),
        TokenKind::Ge => (BinaryOp::Ge, 7, 8),
        TokenKind::Plus => (BinaryOp::Add, 9, 10),
        TokenKind::Minus => (BinaryOp::Sub, 9, 10),
        TokenKind::Star => (BinaryOp::Mul, 11, 12),
        TokenKind::Slash => (BinaryOp::Div, 11, 12),
        TokenKind::Percent => (BinaryOp::Rem, 11, 12),
        TokenKind::Caret => (BinaryOp::Pow, 15, 14),
        _ => return None,
    })
}

struct Parser<'a> {
    src: &'a str,
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Token {
        self.tokens[self.pos]
    }

    fn advance(&mut self) -> Token {
        let tok = self.tokens[self.pos];
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn unexpected(&self, tok: Token) -> PixelMathError {
        if tok.kind == TokenKind::Eof {
            PixelMathError::at("unexpected end of expression", tok.span)
        } else {
            PixelMathError::at(format!("unexpected token '{}'", tok.text(self.src)), tok.span)
        }
    }

    fn enter(&mut self, span: Span) -> Result<(), PixelMathError> {
        self.depth += 1;
        if self.depth > MAX_NESTING {
            return Err(PixelMathError::at("expression nesting too deep", span));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn parse_expr(&mut self, min_bp: u8) -> Result<Expr, PixelMathError> {
        let mut lhs = self.parse_prefix()?;
        loop {
            let tok = self.peek();
            let Some((op, left_bp, right_bp)) = binary_binding(tok.kind) else {
                break;
            };
            if left_bp < min_bp {
                break;
            }
            self.advance();
            self.enter(tok.span)?;
            let rhs = self.parse_expr(right_bp)?;
            self.leave();
            lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) };
        }
        Ok(lhs)
    }

    fn parse_prefix(&mut self) -> Result<Expr, PixelMathError> {
        let tok = self.advance();
        match tok.kind {
            TokenKind::Number(value) => Ok(Expr::Number(value)),
            TokenKind::Ident => {
                if self.peek().kind == TokenKind::LParen {
                    self.parse_call(tok)
                } else {
                    Ok(Expr::Symbol { name: tok.text(self.src).to_string(), span: tok.span })
                }
            }
            TokenKind::LParen => {
                self.enter(tok.span)?;
                let inner = self.parse_expr(0)?;
                self.leave();
                let close = self.advance();
                if close.kind != TokenKind::RParen {
                    return Err(self.unexpected(close));
                }
                Ok(inner)
            }
            TokenKind::Minus => self.parse_unary(UnaryOp::Neg, tok.span),
            TokenKind::Tilde => self.parse_unary(UnaryOp::Invert, tok.span),
            TokenKind::Bang => self.parse_unary(UnaryOp::Not, tok.span),
            _ => Err(self.unexpected(tok)),
        }
    }

    fn parse_unary(&mut self, op: UnaryOp, span: Span) -> Result<Expr, PixelMathError> {
        self.enter(span)?;
        let operand = self.parse_expr(PREFIX_BINDING_POWER)?;
        self.leave();
        Ok(Expr::Unary { op, operand: Box::new(operand) })
    }

    fn parse_call(&mut self, name_tok: Token) -> Result<Expr, PixelMathError> {
        let name = name_tok.text(self.src);
        let func = Function::from_name(name)
            .ok_or_else(|| PixelMathError::at(format!("unknown function '{}'", name), name_tok.span))?;
        self.advance();
        let mut args = Vec::new();
        if self.peek().kind == TokenKind::RParen {
            self.advance();
        } else {
            loop {
                self.enter(name_tok.span)?;
                args.push(self.parse_expr(0)?);
                self.leave();
                let next = self.advance();
                match next.kind {
                    TokenKind::Comma => continue,
                    TokenKind::RParen => break,
                    _ => return Err(self.unexpected(next)),
                }
            }
        }
        check_arguments(func, &args, name_tok.span)?;
        Ok(Expr::Call { func, args, span: name_tok.span })
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn check_arguments(func: Function, args: &[Expr], span: Span) -> Result<(), PixelMathError> {
    match func.arity() {
        Arity::Exact(0) if !args.is_empty() => {
            return Err(PixelMathError::at(
                format!("{}() takes no arguments, got {}", func.name(), args.len()),
                span,
            ));
        }
        Arity::Exact(n) if args.len() != n => {
            return Err(PixelMathError::at(
                format!("{} expects {} argument{}, got {}", func.name(), n, plural(n), args.len()),
                span,
            ));
        }
        Arity::AtLeast(n) if args.len() < n => {
            return Err(PixelMathError::at(
                format!("{} expects at least {} argument{}, got {}", func.name(), n, plural(n), args.len()),
                span,
            ));
        }
        _ => {}
    }
    let single_symbol = args.len() == 1 && matches!(args[0], Expr::Symbol { .. });
    if func.is_reducer() && !single_symbol {
        return Err(PixelMathError::at(
            format!("{} expects an image symbol as its argument", func.name()),
            span,
        ));
    }
    if matches!(func, Function::Min | Function::Max) && args.len() == 1 && !single_symbol {
        return Err(PixelMathError::at(
            format!("{} with a single argument expects an image symbol", func.name()),
            span,
        ));
    }
    Ok(())
}

pub fn parse(src: &str) -> Result<Expr, PixelMathError> {
    let tokens = tokenize(src)?;
    if tokens.len() == 1 {
        return Err(PixelMathError::at("empty expression", Span::new(0, 0)));
    }
    let mut parser = Parser { src, tokens, pos: 0, depth: 0 };
    let expr = parser.parse_expr(0)?;
    let trailing = parser.peek();
    if trailing.kind != TokenKind::Eof {
        return Err(parser.unexpected(trailing));
    }
    Ok(expr)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(v: f64) -> Box<Expr> {
        Box::new(Expr::Number(v))
    }

    #[test]
    fn builds_precedence_tree() {
        let expr = parse("1 + 2 * 3").unwrap();
        assert_eq!(
            expr,
            Expr::Binary {
                op: BinaryOp::Add,
                lhs: num(1.0),
                rhs: Box::new(Expr::Binary { op: BinaryOp::Mul, lhs: num(2.0), rhs: num(3.0) }),
            }
        );
        let pow = parse("-2^2").unwrap();
        assert_eq!(
            pow,
            Expr::Unary {
                op: UnaryOp::Neg,
                operand: Box::new(Expr::Binary { op: BinaryOp::Pow, lhs: num(2.0), rhs: num(2.0) }),
            }
        );
        let right = parse("2^3^2").unwrap();
        assert_eq!(
            right,
            Expr::Binary {
                op: BinaryOp::Pow,
                lhs: num(2.0),
                rhs: Box::new(Expr::Binary { op: BinaryOp::Pow, lhs: num(3.0), rhs: num(2.0) }),
            }
        );
    }

    #[test]
    fn calls_carry_function_and_span() {
        let expr = parse("  max($T, A, 2)").unwrap();
        match expr {
            Expr::Call { func, args, span } => {
                assert_eq!(func, Function::Max);
                assert_eq!(args.len(), 3);
                assert_eq!(span, Span::new(2, 3));
                assert_eq!(args[0], Expr::Symbol { name: "$T".into(), span: Span::new(6, 2) });
            }
            other => panic!("unexpected {:?}", other),
        }
        assert!(matches!(parse("pi()").unwrap(), Expr::Call { func: Function::Pi, .. }));
    }

    #[test]
    fn arity_errors_point_at_the_function_name() {
        let err = parse("1 + pow(2)").unwrap_err();
        assert_eq!(err.position, Some(4));
        assert_eq!(err.length, Some(3));
        assert!(err.message.contains("pow expects 2 arguments"), "{}", err.message);
        let err = parse("min()").unwrap_err();
        assert!(err.message.contains("at least 1 argument"), "{}", err.message);
        let err = parse("e(1)").unwrap_err();
        assert!(err.message.contains("takes no arguments"), "{}", err.message);
        let err = parse("sdev($T + 1)").unwrap_err();
        assert!(err.message.contains("image symbol"), "{}", err.message);
        assert!(parse("min($T)").is_ok());
        assert!(parse("min($T, 1)").is_ok());
    }

    #[test]
    fn rejects_trailing_tokens_and_bad_commas() {
        let err = parse("1 2").unwrap_err();
        assert_eq!(err.position, Some(2));
        let err = parse("max(1,)").unwrap_err();
        assert_eq!(err.position, Some(6));
        let err = parse("max(1 2)").unwrap_err();
        assert_eq!(err.position, Some(6));
        let err = parse("()").unwrap_err();
        assert_eq!(err.position, Some(1));
    }

    #[test]
    fn deep_nesting_is_rejected_not_overflowed() {
        let deep = format!("{}1{}", "(".repeat(2000), ")".repeat(2000));
        let err = parse(&deep).unwrap_err();
        assert!(err.message.contains("nesting"), "{}", err.message);
        let ok = format!("{}1{}", "(".repeat(200), ")".repeat(200));
        assert!(parse(&ok).is_ok());
    }
}
