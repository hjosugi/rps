//! A small backtracking regex engine for the operators #26 needs.
//!
//! It deliberately supports a subset: literals, `.`, `^`, `$`, character
//! classes, `\d`/`\w`/`\s`, capturing groups, non-capturing groups, `*`, `+`,
//! `?` and alternation. Anything else is refused by the caller, never
//! approximated.

#[derive(Debug, Clone)]
pub struct Regex {
    body: Node,
    groups: usize,
}

#[derive(Debug, Clone)]
enum Node {
    Alt(Vec<Node>),
    Seq(Vec<Node>),
    Repeat(Box<Node>, usize, Option<usize>),
    Capture(usize, Box<Node>),
    Start,
    End,
    Char(char),
    Any,
    Class(Vec<ClassItem>, bool),
    Digit(bool),
    Word(bool),
    Space(bool),
}

#[derive(Debug, Clone)]
enum ClassItem {
    Char(char),
    Range(char, char),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub start: usize,
    pub end: usize,
    pub groups: Vec<Option<(usize, usize)>>,
}

impl Regex {
    pub fn new(pattern: &str, case_sensitive: bool) -> Result<Regex, &'static str> {
        let mut parser = Parser {
            chars: pattern.chars().collect(),
            at: 0,
            groups: 0,
            case_sensitive,
        };
        let body = parser.alt()?;
        if parser.at != parser.chars.len() {
            return Err("unexpected ')'");
        }
        Ok(Regex {
            body,
            groups: parser.groups,
        })
    }

    pub fn find(&self, text: &str) -> Option<Match> {
        let chars: Vec<char> = text.chars().collect();
        for start in 0..=chars.len() {
            let captures = vec![None; self.groups + 1];
            if let Some(states) = match_node(&self.body, &chars, start, &captures)
                && let Some((end, mut groups)) = states.into_iter().next()
            {
                groups[0] = Some((start, end));
                return Some(Match { start, end, groups });
            }
        }
        None
    }

    pub fn replace_all(&self, text: &str, replacement: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        let mut offset = 0;
        while let Some(found) = self.find(rest) {
            let start_byte = byte_index(rest, found.start);
            let end_byte = byte_index(rest, found.end);
            out.push_str(&rest[..start_byte]);
            out.push_str(&expand(
                replacement,
                &found,
                &rest.chars().collect::<Vec<_>>(),
            ));
            if end_byte == 0 {
                if let Some(ch) = rest.chars().next() {
                    out.push(ch);
                    rest = &rest[ch.len_utf8()..];
                    offset += ch.len_utf8();
                } else {
                    break;
                }
            } else {
                rest = &rest[end_byte..];
                offset += end_byte;
            }
            let _ = offset;
        }
        out.push_str(rest);
        out
    }

    pub fn split(&self, text: &str) -> Vec<String> {
        let mut parts = Vec::new();
        let mut rest = text;
        while let Some(found) = self.find(rest) {
            let start_byte = byte_index(rest, found.start);
            let end_byte = byte_index(rest, found.end);
            parts.push(rest[..start_byte].to_owned());
            if end_byte == 0 {
                break;
            }
            rest = &rest[end_byte..];
        }
        parts.push(rest.to_owned());
        parts
    }
}

fn byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(byte, _)| byte)
}

fn expand(replacement: &str, found: &Match, text: &[char]) -> String {
    let mut out = String::new();
    let mut chars = replacement.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' {
            out.push(ch);
            continue;
        }
        match chars.peek() {
            Some('$') => {
                chars.next();
                out.push('$');
            }
            Some(digit) if digit.is_ascii_digit() => {
                let index = digit.to_digit(10).unwrap_or(0) as usize;
                chars.next();
                if let Some(Some((start, end))) = found.groups.get(index) {
                    out.extend(&text[*start..*end]);
                }
            }
            _ => out.push('$'),
        }
    }
    out
}

type Captures = Vec<Option<(usize, usize)>>;
type States = Vec<(usize, Captures)>;

fn match_node(
    node: &Node,
    text: &[char],
    pos: usize,
    caps: &[Option<(usize, usize)>],
) -> Option<States> {
    let mut out = Vec::new();
    match node {
        Node::Char(expected) => {
            if text.get(pos) == Some(expected) {
                out.push((pos + 1, caps.to_vec()));
            }
        }
        Node::Any => {
            if pos < text.len() && text[pos] != '\n' {
                out.push((pos + 1, caps.to_vec()));
            }
        }
        Node::Start => {
            if pos == 0 {
                out.push((pos, caps.to_vec()));
            }
        }
        Node::End => {
            if pos == text.len() {
                out.push((pos, caps.to_vec()));
            }
        }
        Node::Digit(negated) => {
            if let Some(ch) = text.get(pos)
                && ch.is_ascii_digit() ^ *negated
            {
                out.push((pos + 1, caps.to_vec()));
            }
        }
        Node::Word(negated) => {
            if let Some(ch) = text.get(pos)
                && ((ch.is_alphanumeric() || *ch == '_') ^ *negated)
            {
                out.push((pos + 1, caps.to_vec()));
            }
        }
        Node::Space(negated) => {
            if let Some(ch) = text.get(pos)
                && ch.is_whitespace() ^ *negated
            {
                out.push((pos + 1, caps.to_vec()));
            }
        }
        Node::Class(items, negated) => {
            if let Some(ch) = text.get(pos) {
                let mut inside = false;
                for item in items {
                    match item {
                        ClassItem::Char(c) if c == ch => inside = true,
                        ClassItem::Range(a, b) if a <= ch && ch <= b => inside = true,
                        _ => {}
                    }
                }
                if inside ^ negated {
                    out.push((pos + 1, caps.to_vec()));
                }
            }
        }
        Node::Capture(index, inner) => {
            for (end, mut groups) in match_node(inner, text, pos, caps)? {
                groups[*index] = Some((pos, end));
                out.push((end, groups));
            }
        }
        Node::Alt(branches) => {
            for branch in branches {
                if let Some(states) = match_node(branch, text, pos, caps) {
                    out.extend(states);
                }
            }
        }
        Node::Seq(parts) => {
            let mut states = vec![(pos, caps.to_vec())];
            for part in parts {
                let mut next = Vec::new();
                for (at, groups) in states {
                    if let Some(states) = match_node(part, text, at, &groups) {
                        next.extend(states);
                    }
                }
                states = next;
                if states.is_empty() {
                    break;
                }
            }
            out.extend(states);
        }
        Node::Repeat(inner, min, max) => {
            let limit = max.unwrap_or_else(|| text.len() + 1).min(text.len() + 1);
            let mut levels = vec![vec![(pos, caps.to_vec())]];
            for _ in 0..limit {
                let mut next = Vec::new();
                for (at, groups) in levels.last().cloned().unwrap_or_default() {
                    if let Some(states) = match_node(inner, text, at, &groups) {
                        for state in states {
                            if state.0 != at {
                                next.push(state);
                            }
                        }
                    }
                }
                if next.is_empty() {
                    break;
                }
                levels.push(next);
            }
            for count in (0..levels.len()).rev() {
                if count >= *min {
                    out.extend(levels[count].clone());
                }
            }
        }
    }
    Some(out)
}

struct Parser {
    chars: Vec<char>,
    at: usize,
    groups: usize,
    case_sensitive: bool,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn next(&mut self) -> Option<char> {
        let ch = self.peek();
        if ch.is_some() {
            self.at += 1;
        }
        ch
    }

    fn alt(&mut self) -> Result<Node, &'static str> {
        let mut branches = vec![self.seq()?];
        while self.peek() == Some('|') {
            self.at += 1;
            branches.push(self.seq()?);
        }
        Ok(if branches.len() == 1 {
            branches.remove(0)
        } else {
            Node::Alt(branches)
        })
    }

    fn seq(&mut self) -> Result<Node, &'static str> {
        let mut parts = Vec::new();
        while let Some(ch) = self.peek() {
            if ch == '|' || ch == ')' {
                break;
            }
            let atom = self.atom()?;
            let atom = self.quantifier(atom)?;
            parts.push(atom);
        }
        Ok(Node::Seq(parts))
    }

    fn quantifier(&mut self, atom: Node) -> Result<Node, &'static str> {
        let (min, max) = match self.peek() {
            Some('*') => {
                self.at += 1;
                (0, None)
            }
            Some('+') => {
                self.at += 1;
                (1, None)
            }
            Some('?') => {
                self.at += 1;
                (0, Some(1))
            }
            _ => return Ok(atom),
        };
        Ok(Node::Repeat(Box::new(atom), min, max))
    }

    fn atom(&mut self) -> Result<Node, &'static str> {
        match self.next() {
            Some('^') => Ok(Node::Start),
            Some('$') => Ok(Node::End),
            Some('.') => Ok(Node::Any),
            Some('(') => {
                let capture = self.peek() != Some('?');
                if !capture {
                    if self.chars.get(self.at + 1) == Some(&':') {
                        self.at += 2;
                    } else {
                        return Err("unsupported group");
                    }
                }
                let index = if capture {
                    self.groups += 1;
                    Some(self.groups)
                } else {
                    None
                };
                let inner = self.alt()?;
                if self.next() != Some(')') {
                    return Err("unclosed group");
                }
                Ok(match index {
                    Some(index) => Node::Capture(index, Box::new(inner)),
                    None => inner,
                })
            }
            Some('[') => {
                let negated = self.peek() == Some('^');
                if negated {
                    self.at += 1;
                }
                let mut items = Vec::new();
                while let Some(ch) = self.next() {
                    if ch == ']' {
                        return Ok(Node::Class(items, negated));
                    }
                    if ch == '\\' {
                        let escaped = self.next().ok_or("trailing escape")?;
                        items.push(ClassItem::Char(self.fold(escaped)));
                        continue;
                    }
                    if self.peek() == Some('-')
                        && self.chars.get(self.at + 1).is_some_and(|c| *c != ']')
                    {
                        self.at += 1;
                        let end = self.next().ok_or("bad range")?;
                        items.push(ClassItem::Range(self.fold(ch), self.fold(end)));
                    } else {
                        items.push(ClassItem::Char(self.fold(ch)));
                    }
                }
                Err("unclosed class")
            }
            Some('\\') => {
                let escaped = self.next().ok_or("trailing escape")?;
                Ok(match escaped {
                    'd' => Node::Digit(false),
                    'D' => Node::Digit(true),
                    'w' => Node::Word(false),
                    'W' => Node::Word(true),
                    's' => Node::Space(false),
                    'S' => Node::Space(true),
                    other => Node::Char(self.fold(other)),
                })
            }
            Some(ch) => Ok(Node::Char(self.fold(ch))),
            None => Ok(Node::Seq(Vec::new())),
        }
    }

    fn fold(&self, ch: char) -> char {
        if self.case_sensitive {
            ch
        } else {
            ch.to_ascii_lowercase()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_groups() {
        let regex = Regex::new("^(a)(b)", true).unwrap();
        let found = regex.find("abc").unwrap();
        assert_eq!(found.groups[0], Some((0, 2)));
        assert_eq!(found.groups[1], Some((0, 1)));
        assert_eq!(found.groups[2], Some((1, 2)));
    }

    #[test]
    fn replaces_and_splits() {
        let regex = Regex::new("b", true).unwrap();
        assert_eq!(regex.replace_all("abc", "X"), "aXc");
        let regex = Regex::new(",", true).unwrap();
        assert_eq!(regex.split("a,b,c"), ["a", "b", "c"]);
    }

    #[test]
    fn case_insensitive_by_default() {
        let regex = Regex::new("A", false).unwrap();
        assert!(regex.find("a").is_some());
    }
}
