use crate::types::header::HduHeader;

pub const HISTORY_TEXT_BYTES: usize = 72;

pub fn wrap_history(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let mut rest = word;
        while rest.len() > HISTORY_TEXT_BYTES {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            let mut cut = HISTORY_TEXT_BYTES;
            while !rest.is_char_boundary(cut) {
                cut -= 1;
            }
            lines.push(rest[..cut].to_string());
            rest = &rest[cut..];
        }
        if rest.is_empty() {
            continue;
        }
        if current.is_empty() {
            current.push_str(rest);
        } else if current.len() + 1 + rest.len() <= HISTORY_TEXT_BYTES {
            current.push(' ');
            current.push_str(rest);
        } else {
            lines.push(std::mem::replace(&mut current, rest.to_string()));
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

pub fn push_history(header: &mut HduHeader, text: &str) {
    for line in wrap_history(text) {
        header.cards.push(("HISTORY".to_string(), line));
    }
}
