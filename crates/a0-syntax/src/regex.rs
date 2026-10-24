//! A small regular-expression engine with longest-match semantics, used by the lexer.
//!
//! Patterns compile to one Thompson NFA. Matching from a position returns the longest match and, among
//! patterns matching that longest text, the one added first. That is exactly what a lexer needs
//! (maximal munch with priority by declaration order).
//!
//! Supported syntax: literal characters, `.` (any character except newline), classes `[a-z_]`,
//! negated classes `[^"]`, escapes (`\n \t \r \\ \/ \.` and so on, `\d \w \s`, `\xHH`, `\u{...}`),
//! grouping `( )`, alternation `|`, and quantifiers `* + ?` `{n}` `{n,}` `{n,m}`.

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Node {
    Empty,
    Char(char),
    Class(Class),
    Any,
    Concat(Vec<Node>),
    Alt(Vec<Node>),
    Repeat(Box<Node>, u32, Option<u32>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Class {
    ranges: Vec<(char, char)>,
    negated: bool,
}

impl Class {
    fn matches(&self, c: char) -> bool {
        let inside = self.ranges.iter().any(|&(lo, hi)| lo <= c && c <= hi);
        inside != self.negated
    }
}

const MAX_REPEAT: u32 = 1000;

/// Parses a pattern (without the surrounding slashes).
pub(crate) fn parse(pattern: &str) -> Result<Node, String> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut p = RegexParser { chars, pos: 0 };
    let node = p.alt()?;
    if p.pos < p.chars.len() {
        return Err(format!("unexpected '{}' in pattern", p.chars[p.pos]));
    }
    Ok(node)
}

struct RegexParser {
    chars: Vec<char>,
    pos: usize,
}

impl RegexParser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn alt(&mut self) -> Result<Node, String> {
        let mut options = vec![self.concat()?];
        while self.peek() == Some('|') {
            self.pos += 1;
            options.push(self.concat()?);
        }
        Ok(if options.len() == 1 {
            options.pop().unwrap()
        } else {
            Node::Alt(options)
        })
    }

    fn concat(&mut self) -> Result<Node, String> {
        let mut items = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom()?;
            items.push(self.quantified(atom)?);
        }
        Ok(match items.len() {
            0 => Node::Empty,
            1 => items.pop().unwrap(),
            _ => Node::Concat(items),
        })
    }

    fn quantified(&mut self, mut atom: Node) -> Result<Node, String> {
        loop {
            match self.peek() {
                Some('*') => {
                    self.pos += 1;
                    atom = Node::Repeat(Box::new(atom), 0, None);
                }
                Some('+') => {
                    self.pos += 1;
                    atom = Node::Repeat(Box::new(atom), 1, None);
                }
                Some('?') => {
                    self.pos += 1;
                    atom = Node::Repeat(Box::new(atom), 0, Some(1));
                }
                Some('{') => {
                    self.pos += 1;
                    let min = self.number()?;
                    let max = if self.peek() == Some(',') {
                        self.pos += 1;
                        if self.peek() == Some('}') {
                            None
                        } else {
                            Some(self.number()?)
                        }
                    } else {
                        Some(min)
                    };
                    if self.peek() != Some('}') {
                        return Err("expected '}' to close a repetition".to_string());
                    }
                    self.pos += 1;
                    if max.is_some_and(|m| m < min) || min > MAX_REPEAT || max.is_some_and(|m| m > MAX_REPEAT) {
                        return Err("invalid repetition bounds".to_string());
                    }
                    atom = Node::Repeat(Box::new(atom), min, max);
                }
                _ => return Ok(atom),
            }
        }
    }

    fn number(&mut self) -> Result<u32, String> {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        let s: String = self.chars[start..self.pos].iter().collect();
        s.parse().map_err(|_| "expected a number in a repetition".to_string())
    }

    fn atom(&mut self) -> Result<Node, String> {
        let c = self.peek().unwrap();
        self.pos += 1;
        match c {
            '(' => {
                let inner = self.alt()?;
                if self.peek() != Some(')') {
                    return Err("expected ')'".to_string());
                }
                self.pos += 1;
                Ok(inner)
            }
            '[' => self.class(),
            '.' => Ok(Node::Any),
            '\\' => match self.escape(false)? {
                Escaped::Char(c) => Ok(Node::Char(c)),
                Escaped::Class(cl) => Ok(Node::Class(cl)),
            },
            '*' | '+' | '?' | '{' => Err(format!("'{c}' has nothing to repeat")),
            c => Ok(Node::Char(c)),
        }
    }

    fn class(&mut self) -> Result<Node, String> {
        let mut negated = false;
        if self.peek() == Some('^') {
            negated = true;
            self.pos += 1;
        }
        let mut ranges = Vec::new();
        let mut first = true;
        loop {
            let Some(c) = self.peek() else {
                return Err("unterminated character class".to_string());
            };
            self.pos += 1;
            if c == ']' && !first {
                break;
            }
            first = false;
            let lo = if c == '\\' {
                match self.escape(true)? {
                    Escaped::Char(c) => c,
                    Escaped::Class(cl) => {
                        ranges.extend(cl.ranges);
                        continue;
                    }
                }
            } else {
                c
            };
            if self.peek() == Some('-') && self.chars.get(self.pos + 1).is_some_and(|&n| n != ']') {
                self.pos += 1;
                let hi_c = self.peek().unwrap();
                self.pos += 1;
                let hi = if hi_c == '\\' {
                    match self.escape(true)? {
                        Escaped::Char(c) => c,
                        Escaped::Class(_) => return Err("a class shorthand cannot end a range".to_string()),
                    }
                } else {
                    hi_c
                };
                if hi < lo {
                    return Err(format!("range {lo}-{hi} is reversed"));
                }
                ranges.push((lo, hi));
            } else {
                ranges.push((lo, lo));
            }
        }
        Ok(Node::Class(Class { ranges, negated }))
    }

    fn escape(&mut self, _in_class: bool) -> Result<Escaped, String> {
        let Some(c) = self.peek() else {
            return Err("pattern ends with '\\'".to_string());
        };
        self.pos += 1;
        let class = |ranges: Vec<(char, char)>| Escaped::Class(Class { ranges, negated: false });
        Ok(match c {
            'n' => Escaped::Char('\n'),
            't' => Escaped::Char('\t'),
            'r' => Escaped::Char('\r'),
            '0' => Escaped::Char('\0'),
            'd' => class(vec![('0', '9')]),
            'w' => class(vec![('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')]),
            's' => class(vec![(' ', ' '), ('\t', '\t'), ('\n', '\n'), ('\r', '\r'), ('\x0b', '\x0c')]),
            'x' => {
                let hex: String = self.chars.get(self.pos..self.pos + 2).unwrap_or(&[]).iter().collect();
                self.pos += 2;
                let v = u32::from_str_radix(&hex, 16).map_err(|_| "expected two hex digits after \\x".to_string())?;
                Escaped::Char(char::from_u32(v).unwrap())
            }
            'u' => {
                if self.peek() != Some('{') {
                    return Err("expected '{' after \\u".to_string());
                }
                let close = self.chars[self.pos..]
                    .iter()
                    .position(|&c| c == '}')
                    .ok_or("expected '}' after \\u{...")?;
                let hex: String = self.chars[self.pos + 1..self.pos + close].iter().collect();
                self.pos += close + 1;
                let v = u32::from_str_radix(&hex, 16).map_err(|_| "invalid \\u{...} escape".to_string())?;
                Escaped::Char(char::from_u32(v).ok_or("invalid character in \\u{...}")?)
            }
            c if c.is_ascii_alphanumeric() => return Err(format!("unknown escape \\{c}")),
            c => Escaped::Char(c),
        })
    }
}

enum Escaped {
    Char(char),
    Class(Class),
}

/// Whether a pattern can match the empty string (such tokens would never advance the lexer).
pub(crate) fn matches_empty(node: &Node) -> bool {
    match node {
        Node::Empty => true,
        Node::Char(_) | Node::Class(_) | Node::Any => false,
        Node::Concat(items) => items.iter().all(matches_empty),
        Node::Alt(options) => options.iter().any(matches_empty),
        Node::Repeat(inner, min, _) => *min == 0 || matches_empty(inner),
    }
}

#[derive(Clone, Debug)]
enum State {
    Char(char, usize),
    Class(usize, usize),
    Any(usize),
    Split(usize, usize),
    Jump(usize),
    Match(u32),
}

/// A set of patterns compiled into one NFA.
#[derive(Clone, Debug, Default)]
pub(crate) struct Matcher {
    states: Vec<State>,
    classes: Vec<Class>,
    starts: Vec<usize>,
}

impl Matcher {
    pub(crate) fn new() -> Matcher {
        Matcher::default()
    }

    /// Adds a pattern that reports `id` when it matches. Earlier patterns win ties.
    pub(crate) fn add(&mut self, node: &Node, id: u32) {
        let accept = self.push(State::Match(id));
        let start = self.compile(node, accept);
        self.starts.push(start);
    }

    /// Adds a pattern matching `text` literally.
    pub(crate) fn add_literal(&mut self, text: &str, id: u32) {
        let node = Node::Concat(text.chars().map(Node::Char).collect());
        self.add(&node, id);
    }

    fn push(&mut self, s: State) -> usize {
        self.states.push(s);
        self.states.len() - 1
    }

    /// Compiles `node` so that it continues to `next`; returns its entry state.
    fn compile(&mut self, node: &Node, next: usize) -> usize {
        match node {
            Node::Empty => self.push(State::Jump(next)),
            Node::Char(c) => self.push(State::Char(*c, next)),
            Node::Any => self.push(State::Any(next)),
            Node::Class(cl) => {
                self.classes.push(cl.clone());
                let id = self.classes.len() - 1;
                self.push(State::Class(id, next))
            }
            Node::Concat(items) => {
                let mut entry = next;
                for item in items.iter().rev() {
                    entry = self.compile(item, entry);
                }
                entry
            }
            Node::Alt(options) => {
                let entries: Vec<usize> = options.iter().map(|o| self.compile(o, next)).collect();
                let mut entry = entries[entries.len() - 1];
                for &e in entries[..entries.len() - 1].iter().rev() {
                    entry = self.push(State::Split(e, entry));
                }
                entry
            }
            Node::Repeat(inner, min, max) => {
                // Optional or unbounded tail first, then the mandatory copies in front of it.
                let mut entry = match max {
                    None => {
                        // loop: split(inner -> loop, next)
                        let split = self.push(State::Split(usize::MAX, next));
                        let body = self.compile(inner, split);
                        self.states[split] = State::Split(body, next);
                        split
                    }
                    Some(max) => {
                        let mut entry = next;
                        for _ in *min..*max {
                            let body = self.compile(inner, entry);
                            entry = self.push(State::Split(body, next));
                            // Each optional copy may stop early, so it can also skip straight to `next`.
                        }
                        entry
                    }
                };
                for _ in 0..*min {
                    entry = self.compile(inner, entry);
                }
                entry
            }
        }
    }

    fn add_closure(&self, state: usize, set: &mut Vec<usize>, seen: &mut [bool]) {
        if seen[state] {
            return;
        }
        seen[state] = true;
        match self.states[state] {
            State::Split(a, b) => {
                self.add_closure(a, set, seen);
                self.add_closure(b, set, seen);
            }
            State::Jump(a) => self.add_closure(a, set, seen),
            _ => set.push(state),
        }
    }

    /// The longest non-empty match starting at byte offset `pos`: `(end offset, pattern id)`.
    pub(crate) fn longest_match(&self, text: &str, pos: usize) -> Option<(usize, u32)> {
        let mut seen = vec![false; self.states.len()];
        let mut current = Vec::new();
        for &s in &self.starts {
            self.add_closure(s, &mut current, &mut seen);
        }
        let mut best = None;
        let mut next = Vec::new();
        for (offset, c) in text[pos..].char_indices() {
            seen.iter_mut().for_each(|s| *s = false);
            next.clear();
            for &s in &current {
                let target = match self.states[s] {
                    State::Char(want, n) if want == c => Some(n),
                    State::Class(cl, n) if self.classes[cl].matches(c) => Some(n),
                    State::Any(n) if c != '\n' => Some(n),
                    _ => None,
                };
                if let Some(n) = target {
                    self.add_closure(n, &mut next, &mut seen);
                }
            }
            if next.is_empty() {
                break;
            }
            std::mem::swap(&mut current, &mut next);
            let accepted = current
                .iter()
                .filter_map(|&s| match self.states[s] {
                    State::Match(id) => Some(id),
                    _ => None,
                })
                .min();
            if let Some(id) = accepted {
                best = Some((pos + offset + c.len_utf8(), id));
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(patterns: &[&str]) -> Matcher {
        let mut matcher = Matcher::new();
        for (i, p) in patterns.iter().enumerate() {
            matcher.add(&parse(p).unwrap(), i as u32);
        }
        matcher
    }

    #[test]
    fn longest_match_wins_then_first_pattern() {
        let mx = m(&["[0-9]+", "[0-9]+\\.[0-9]+", "[a-z]+", "if"]);
        assert_eq!(mx.longest_match("123.45x", 0), Some((6, 1)));
        assert_eq!(mx.longest_match("123.x", 0), Some((3, 0)));
        assert_eq!(mx.longest_match("ifx", 0), Some((3, 2)));
        // Both [a-z]+ and "if" match "if"; the earlier pattern wins.
        assert_eq!(mx.longest_match("if ", 0), Some((2, 2)));
        assert_eq!(mx.longest_match("  ", 0), None);
    }

    #[test]
    fn classes_escapes_and_quantifiers() {
        let mx = m(&["\"[^\"]*\""]);
        assert_eq!(mx.longest_match("\"a b\" rest", 0), Some((5, 0)));
        assert_eq!(mx.longest_match("\"open", 0), None);
        let mx = m(&["\\/\\/[^\\n]*"]);
        assert_eq!(mx.longest_match("// hi\nx", 0), Some((5, 0)));
        let mx = m(&["a{2,3}"]);
        assert_eq!(mx.longest_match("aaaa", 0), Some((3, 0)));
        assert_eq!(mx.longest_match("a", 0), None);
        let mx = m(&["[\\w-]+"]);
        assert_eq!(mx.longest_match("a_b-c!", 0), Some((5, 0)));
        let mx = m(&["(ab|a)(bc)?"]);
        assert_eq!(mx.longest_match("abc", 0), Some((3, 0)));
        let mx = m(&["x\\u{e9}+"]);
        assert_eq!(mx.longest_match("xéé!", 0), Some((5, 0)));
    }

    #[test]
    fn match_from_offset_and_any() {
        let mx = m(&["."]);
        assert_eq!(mx.longest_match("ab", 1), Some((2, 0)));
        assert_eq!(mx.longest_match("\n", 0), None);
    }

    #[test]
    fn errors() {
        assert!(parse("a{3,2}").is_err());
        assert!(parse("*a").is_err());
        assert!(parse("[a-").is_err());
        assert!(parse("(a").is_err());
        assert!(parse("\\q").is_err());
        assert!(matches_empty(&parse("a*").unwrap()));
        assert!(!matches_empty(&parse("a+").unwrap()));
    }
}
