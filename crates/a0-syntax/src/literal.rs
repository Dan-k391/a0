//! Decoding literal tokens.

/// The value of a quoted string literal (`"a\tb"`, with its quotes). Escapes: `\n`, `\t`, `\r`,
/// `\0`, `\\`, `\"`, `\'` and `\u{hex}`. An unknown escape is an error naming it, ready to report.
pub fn unquote(literal: &str) -> Result<String, String> {
    let inner = literal
        .get(1..literal.len().saturating_sub(1))
        .unwrap_or("");
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        out.push(match chars.next() {
            Some('n') => '\n',
            Some('t') => '\t',
            Some('r') => '\r',
            Some('0') => '\0',
            Some('\\') => '\\',
            Some('"') => '"',
            Some('\'') => '\'',
            Some('u') => {
                let rest: String = chars.by_ref().take_while(|&c| c != '}').collect();
                let code = rest
                    .strip_prefix('{')
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .and_then(char::from_u32);
                match code {
                    Some(c) => c,
                    None => return Err(format!("invalid escape `\\u{rest}}}`")),
                }
            }
            Some(other) => return Err(format!("unknown escape `\\{other}`")),
            None => return Err("unknown escape `\\`".to_string()),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_are_decoded() {
        assert_eq!(unquote(r#""a\tb\n""#).unwrap(), "a\tb\n");
        assert_eq!(unquote(r#""\"q\" \\ \u{41}""#).unwrap(), "\"q\" \\ A");
        assert_eq!(unquote(r#""\q""#).unwrap_err(), "unknown escape `\\q`");
        assert_eq!(unquote(r#""""#).unwrap(), "");
    }
}
