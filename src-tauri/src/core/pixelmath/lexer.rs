use super::{PixelMathError, Span};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TokenKind {
    Number(f64),
    Ident,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Tilde,
    Bang,
    Lt,
    Le,
    Gt,
    Ge,
    EqEq,
    Ne,
    AndAnd,
    OrOr,
    LParen,
    RParen,
    Comma,
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn text<'a>(&self, src: &'a str) -> &'a str {
        &src[self.span.start..self.span.start + self.span.len]
    }
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_ident_continue(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn number_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        let mut j = i + 1;
        if j < bytes.len() && (bytes[j] == b'+' || bytes[j] == b'-') {
            j += 1;
        }
        if j < bytes.len() && bytes[j].is_ascii_digit() {
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    i
}

fn two_char(bytes: &[u8], i: usize, second: u8) -> bool {
    i + 1 < bytes.len() && bytes[i + 1] == second
}

fn describe_char(c: char) -> String {
    if c.is_ascii_graphic() {
        format!("'{}'", c)
    } else {
        format!("U+{:04X}", c as u32)
    }
}

pub fn tokenize(src: &str) -> Result<Vec<Token>, PixelMathError> {
    let bytes = src.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        let (kind, len) = if c.is_ascii_digit() || (c == b'.' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit()) {
            let end = number_end(bytes, i);
            let text = &src[start..end];
            let value: f64 = text
                .parse()
                .map_err(|_| PixelMathError::at(format!("malformed number '{}'", text), Span::new(start, end - start)))?;
            (TokenKind::Number(value), end - start)
        } else if is_ident_start(c) || (c == b'$' && i + 1 < bytes.len() && is_ident_start(bytes[i + 1])) {
            let mut end = i + 1;
            while end < bytes.len() && is_ident_continue(bytes[end]) {
                end += 1;
            }
            (TokenKind::Ident, end - start)
        } else {
            match c {
                b'+' => (TokenKind::Plus, 1),
                b'-' => (TokenKind::Minus, 1),
                b'*' => (TokenKind::Star, 1),
                b'/' => (TokenKind::Slash, 1),
                b'%' => (TokenKind::Percent, 1),
                b'^' => (TokenKind::Caret, 1),
                b'~' => (TokenKind::Tilde, 1),
                b'(' => (TokenKind::LParen, 1),
                b')' => (TokenKind::RParen, 1),
                b',' => (TokenKind::Comma, 1),
                b'!' if two_char(bytes, i, b'=') => (TokenKind::Ne, 2),
                b'!' => (TokenKind::Bang, 1),
                b'<' if two_char(bytes, i, b'=') => (TokenKind::Le, 2),
                b'<' => (TokenKind::Lt, 1),
                b'>' if two_char(bytes, i, b'=') => (TokenKind::Ge, 2),
                b'>' => (TokenKind::Gt, 1),
                b'=' if two_char(bytes, i, b'=') => (TokenKind::EqEq, 2),
                b'=' => {
                    return Err(PixelMathError::at("unexpected '='; use '==' for equality", Span::new(start, 1)));
                }
                b'&' if two_char(bytes, i, b'&') => (TokenKind::AndAnd, 2),
                b'&' => {
                    return Err(PixelMathError::at("unexpected '&'; use '&&' for logical and", Span::new(start, 1)));
                }
                b'|' if two_char(bytes, i, b'|') => (TokenKind::OrOr, 2),
                b'|' => {
                    return Err(PixelMathError::at("unexpected '|'; use '||' for logical or", Span::new(start, 1)));
                }
                _ => {
                    let ch = src[start..].chars().next().unwrap_or('?');
                    return Err(PixelMathError::at(
                        format!("unexpected character {}", describe_char(ch)),
                        Span::new(start, 1),
                    ));
                }
            }
        };
        tokens.push(Token { kind, span: Span::new(start, len) });
        i = start + len;
    }
    tokens.push(Token { kind: TokenKind::Eof, span: Span::new(bytes.len(), 0) });
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        tokenize(src).unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn tokenizes_operators_numbers_and_symbols() {
        assert_eq!(
            kinds("$T + a_1 * 2.5e-3 <= 4 != 5 == 6 && !x || ~y ^ z % (w, v)"),
            vec![
                TokenKind::Ident,
                TokenKind::Plus,
                TokenKind::Ident,
                TokenKind::Star,
                TokenKind::Number(2.5e-3),
                TokenKind::Le,
                TokenKind::Number(4.0),
                TokenKind::Ne,
                TokenKind::Number(5.0),
                TokenKind::EqEq,
                TokenKind::Number(6.0),
                TokenKind::AndAnd,
                TokenKind::Bang,
                TokenKind::Ident,
                TokenKind::OrOr,
                TokenKind::Tilde,
                TokenKind::Ident,
                TokenKind::Caret,
                TokenKind::Ident,
                TokenKind::Percent,
                TokenKind::LParen,
                TokenKind::Ident,
                TokenKind::Comma,
                TokenKind::Ident,
                TokenKind::RParen,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn spans_are_byte_offsets() {
        let toks = tokenize("ab + $T").unwrap();
        assert_eq!(toks[0].span, Span::new(0, 2));
        assert_eq!(toks[1].span, Span::new(3, 1));
        assert_eq!(toks[2].span, Span::new(5, 2));
        assert_eq!(toks[2].text("ab + $T"), "$T");
        assert_eq!(toks[3].span, Span::new(7, 0));
    }

    #[test]
    fn numbers_with_trailing_dot_and_leading_dot() {
        assert_eq!(kinds("5. .5 1e5 1e"), vec![
            TokenKind::Number(5.0),
            TokenKind::Number(0.5),
            TokenKind::Number(1e5),
            TokenKind::Number(1.0),
            TokenKind::Ident,
            TokenKind::Eof,
        ]);
    }

    #[test]
    fn rejects_single_equals_and_unknown_characters() {
        let err = tokenize("a = b").unwrap_err();
        assert_eq!(err.position, Some(2));
        let err = tokenize("a & b").unwrap_err();
        assert_eq!(err.position, Some(2));
        let err = tokenize("a | b").unwrap_err();
        assert_eq!(err.position, Some(2));
        let err = tokenize("a # b").unwrap_err();
        assert_eq!(err.position, Some(2));
        assert_eq!(err.length, Some(1));
        let err = tokenize("$ + 1").unwrap_err();
        assert_eq!(err.position, Some(0));
        let err = tokenize("1 é 2").unwrap_err();
        assert_eq!(err.position, Some(2));
        assert_eq!(err.length, Some(1));
        assert!(err.message.contains("U+00E9"), "{}", err.message);
    }

    #[test]
    fn invisible_characters_are_named_by_code_point() {
        let err = tokenize("$T\u{00A0}+ 1").unwrap_err();
        assert_eq!(err.position, Some(2));
        assert_eq!(err.length, Some(1));
        assert!(err.message.contains("U+00A0"), "{}", err.message);
        let err = tokenize("$T\u{200B}+ 1").unwrap_err();
        assert!(err.message.contains("U+200B"), "{}", err.message);
    }
}
