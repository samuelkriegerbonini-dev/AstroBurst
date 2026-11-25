use std::fmt;

const REDACTED: &str = "***";

#[derive(Clone, PartialEq, Eq)]
pub struct SecretToken(String);

impl SecretToken {
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(Self(trimmed.to_owned()))
        }
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl fmt::Display for SecretToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARKER: &str = "AB-TEST-TOKEN-0123456789abcdef";

    #[test]
    fn secret_token_debug_and_display_are_redacted() {
        let token = SecretToken::new(format!("  {MARKER} ")).unwrap();
        assert_eq!(format!("{token:?}"), REDACTED);
        assert_eq!(format!("{token}"), REDACTED);
        assert_eq!(format!("{:?}", Some(token.clone())), format!("Some({REDACTED})"));
        assert_eq!(token.expose(), MARKER);
        assert!(SecretToken::new("   ").is_none());
    }
}
