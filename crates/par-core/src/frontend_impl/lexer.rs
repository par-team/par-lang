use crate::location::{FileName, Point, Span};
use core::str::FromStr;
use winnow::{
    Parser, Result,
    error::ParserError,
    stream::{ParseSlice, TokenSlice},
    token::literal,
};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    LParen,
    RParen,
    LCurly,
    RCurly,
    LBrack,
    RBrack,
    Lt,
    Gt,
    LtEq,
    GtEq,

    Slash,
    SlashEq,
    At,
    Colon,
    Semicolon,
    Comma,
    Dot,
    Eq,
    EqEq,
    FatArrow,
    ThinArrow,
    Bang,
    BangEq,
    Quest,
    Star,
    StarEq,
    Plus,
    PlusEq,
    Minus,
    MinusEq,
    Link,

    Float,
    Integer,
    String,
    TemplateStart,
    TemplateEnd,
    TemplateText,
    TemplateStringStart,
    TemplateDataStart,

    InvalidString,
    InvalidChar,

    LowercaseIdentifier,
    UppercaseIdentifier,
    Begin,
    Box,
    Case,
    Catch,
    Chan,
    Choice,
    Dec,
    Def,
    Do,
    Dual,
    Either,
    Else,
    Export,
    If,
    Import,
    Is,
    In,
    Iterative,
    Let,
    And,
    As,
    Module,
    Neg,
    Or,
    Not,
    Loop,
    Poll,
    Repoll,
    Submit,
    Recursive,
    Self_,
    Throw,
    Try,
    Default,
    Type,
    Unfounded,
    External,
    Todo,

    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token<'i> {
    pub kind: TokenKind,
    pub raw: &'i str,
    pub span: Span,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum CommentKind {
    Line,
    Block,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Comment<'i> {
    pub kind: CommentKind,
    pub raw: &'i str,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub(crate) struct Lexed<'i> {
    pub tokens: Vec<Token<'i>>,
    pub comments: Vec<Comment<'i>>,
}

impl Token<'_> {
    pub fn span(&self) -> Span {
        self.span.clone()
    }
}

// More useful in winnow debug view
// impl core::fmt::Debug for Token<'_> {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         format!("{}", self.raw).fmt(f)
//         // f.debug_struct("Token").field("kind", &self.kind).field("raw", &self.raw).field("loc", &self.loc).field("span", &self.span).finish()
//     }
// }
impl PartialEq<TokenKind> for Token<'_> {
    fn eq(&self, other: &TokenKind) -> bool {
        self.kind == *other
    }
}

impl TokenKind {
    pub fn expected(&self) -> &'static str {
        match self {
            TokenKind::LParen => "(",
            TokenKind::RParen => ")",
            TokenKind::LCurly => "{",
            TokenKind::RCurly => "}",
            TokenKind::LBrack => "[",
            TokenKind::RBrack => "]",
            TokenKind::Lt => "<",
            TokenKind::Gt => ">",
            TokenKind::LtEq => "<=",
            TokenKind::GtEq => ">=",

            TokenKind::Slash => "/",
            TokenKind::SlashEq => "/=",
            TokenKind::At => "@",
            TokenKind::Colon => ":",
            TokenKind::Semicolon => ";",
            TokenKind::Comma => ",",
            TokenKind::Dot => ".",
            TokenKind::Eq => "=",
            TokenKind::EqEq => "==",
            TokenKind::FatArrow => "=>",
            TokenKind::ThinArrow => "->",
            TokenKind::Bang => "!",
            TokenKind::BangEq => "!=",
            TokenKind::Quest => "?",
            TokenKind::Star => "*",
            TokenKind::StarEq => "*=",
            TokenKind::Plus => "+",
            TokenKind::PlusEq => "+=",
            TokenKind::Minus => "-",
            TokenKind::MinusEq => "-=",
            TokenKind::Link => "<>",

            TokenKind::Float => "float",
            TokenKind::Integer => "integer",
            TokenKind::String => "string",
            TokenKind::TemplateStart => "`",
            TokenKind::TemplateEnd => "`",
            TokenKind::TemplateText => "template text",
            TokenKind::TemplateStringStart => "${",
            TokenKind::TemplateDataStart => "#{",

            TokenKind::InvalidString => "invalid string",
            TokenKind::InvalidChar => "invalid char",

            TokenKind::LowercaseIdentifier => "lower-case identifier",
            TokenKind::UppercaseIdentifier => "upper-case identifier",
            TokenKind::Begin => "begin",
            TokenKind::Box => "box",
            TokenKind::Case => "case",
            TokenKind::Catch => "catch",
            TokenKind::Chan => "chan",
            TokenKind::Choice => "choice",
            TokenKind::Dec => "dec",
            TokenKind::Def => "def",
            TokenKind::Do => "do",
            TokenKind::Dual => "dual",
            TokenKind::Either => "either",
            TokenKind::Else => "else",
            TokenKind::Export => "export",
            TokenKind::If => "if",
            TokenKind::Import => "import",
            TokenKind::Is => "is",
            TokenKind::In => "in",
            TokenKind::Iterative => "iterative",
            TokenKind::Let => "let",
            TokenKind::And => "and",
            TokenKind::As => "as",
            TokenKind::Module => "module",
            TokenKind::Neg => "neg",
            TokenKind::Or => "or",
            TokenKind::Not => "not",
            TokenKind::Loop => "loop",
            TokenKind::Poll => "poll",
            TokenKind::Repoll => "repoll",
            TokenKind::Submit => "submit",
            TokenKind::Recursive => "recursive",
            TokenKind::Self_ => "self",
            TokenKind::Throw => "throw",
            TokenKind::Try => "try",
            TokenKind::Default => "default",
            TokenKind::Type => "type",
            TokenKind::Unfounded => "unfounded",
            TokenKind::External => "external",
            TokenKind::Todo => "todo",

            TokenKind::Unknown => "???",
        }
    }
}

impl PartialEq<str> for Token<'_> {
    fn eq(&self, other: &str) -> bool {
        self.raw == other
    }
}
impl PartialEq<&str> for Token<'_> {
    fn eq(&self, &other: &&str) -> bool {
        self.raw == other
    }
}

impl<'i, E> Parser<Tokens<'i>, &'i Token<'i>, E> for TokenKind
where
    E: ParserError<Tokens<'i>>,
{
    fn parse_next(&mut self, input: &mut Tokens<'i>) -> Result<&'i Token<'i>, E> {
        literal(*self).parse_next(input).map(|t| &t[0])
    }
}

impl<'a, T: FromStr> ParseSlice<T> for Token<'a> {
    fn parse_slice(&self) -> Option<T> {
        self.raw.parse().ok()
    }
}
impl<'a, T: FromStr> ParseSlice<T> for &Token<'a> {
    fn parse_slice(&self) -> Option<T> {
        self.raw.parse().ok()
    }
}

pub(crate) type Tokens<'i> = TokenSlice<'i, Token<'i>>;
pub(crate) type Input<'a> = Tokens<'a>;

pub(crate) fn lex<'s>(input: &'s str, file: &FileName) -> Vec<Token<'s>> {
    lex_with_comments(input, file).tokens
}

fn is_identifier_start(c: char) -> bool {
    matches!(c, 'a'..='z' | 'A'..='Z' | '_')
}

fn is_identifier_continue(c: char) -> bool {
    is_identifier_start(c) || c.is_ascii_digit()
}

fn identifier_kind(raw: &str) -> Option<TokenKind> {
    let mut chars = raw.chars();
    let first = chars.next()?;
    if !is_identifier_start(first) || !chars.all(is_identifier_continue) {
        return None;
    }

    Some(match raw {
        "begin" => TokenKind::Begin,
        "box" => TokenKind::Box,
        "case" => TokenKind::Case,
        "catch" => TokenKind::Catch,
        "chan" => TokenKind::Chan,
        "choice" => TokenKind::Choice,
        "dec" => TokenKind::Dec,
        "def" => TokenKind::Def,
        "do" => TokenKind::Do,
        "dual" => TokenKind::Dual,
        "either" => TokenKind::Either,
        "else" => TokenKind::Else,
        "export" => TokenKind::Export,
        "if" => TokenKind::If,
        "import" => TokenKind::Import,
        "is" => TokenKind::Is,
        "in" => TokenKind::In,
        "iterative" => TokenKind::Iterative,
        "let" => TokenKind::Let,
        "and" => TokenKind::And,
        "as" => TokenKind::As,
        "module" => TokenKind::Module,
        "neg" => TokenKind::Neg,
        "or" => TokenKind::Or,
        "not" => TokenKind::Not,
        "loop" => TokenKind::Loop,
        "poll" => TokenKind::Poll,
        "repoll" => TokenKind::Repoll,
        "submit" => TokenKind::Submit,
        "recursive" => TokenKind::Recursive,
        "self" => TokenKind::Self_,
        "throw" => TokenKind::Throw,
        "try" => TokenKind::Try,
        "default" => TokenKind::Default,
        "type" => TokenKind::Type,
        "unfounded" => TokenKind::Unfounded,
        "external" => TokenKind::External,
        "todo" => TokenKind::Todo,
        raw if raw.starts_with(char::is_uppercase) => TokenKind::UppercaseIdentifier,
        _ => TokenKind::LowercaseIdentifier,
    })
}

pub(crate) fn is_lowercase_identifier(raw: &str) -> bool {
    identifier_kind(raw) == Some(TokenKind::LowercaseIdentifier)
}

fn scan_digit_run(input: &str, start: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    if !matches!(bytes.get(start), Some(b'0'..=b'9')) {
        return None;
    }

    let mut idx = start + 1;
    while let Some(&byte) = bytes.get(idx) {
        match byte {
            b'0'..=b'9' => idx += 1,
            b'_' if matches!(bytes.get(idx + 1), Some(b'0'..=b'9')) => idx += 1,
            _ => break,
        }
    }

    Some(idx)
}

fn scan_number_token(input: &str) -> Option<(&str, TokenKind, usize)> {
    let bytes = input.as_bytes();
    let mut idx = 0;

    if matches!(bytes.get(idx), Some(b'+' | b'-')) {
        idx += 1;
    }

    idx = scan_digit_run(input, idx)?;

    if matches!(bytes.get(idx), Some(b'.')) {
        let frac_start = idx + 1;
        if let Some(frac_end) = scan_digit_run(input, frac_start) {
            idx = frac_end;
            if matches!(bytes.get(idx), Some(b'e' | b'E')) {
                let exp_start = idx;
                idx += 1;
                if matches!(bytes.get(idx), Some(b'+' | b'-')) {
                    idx += 1;
                }
                if let Some(exp_end) = scan_digit_run(input, idx) {
                    idx = exp_end;
                } else {
                    idx = exp_start;
                }
            }
            let raw = &input[..idx];
            return Some((raw, TokenKind::Float, idx));
        } else if matches!(bytes.get(frac_start), Some(b'_')) {
            idx = frac_start + 1;
            while matches!(
                bytes.get(idx),
                Some(b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'_')
            ) {
                idx += 1;
            }
            let raw = &input[..idx];
            return Some((raw, TokenKind::Unknown, idx));
        }
    }

    let raw = &input[..idx];
    Some((raw, TokenKind::Integer, idx))
}

#[derive(Clone, Copy)]
enum LexMode {
    Normal,
    Template,
    Interpolation { brace_depth: usize },
}

struct LexState<'s, 'f> {
    file: &'f FileName,
    tokens: Vec<Token<'s>>,
    comments: Vec<Comment<'s>>,
    idx: usize,
    row: usize,
    column: usize,
    modes: Vec<LexMode>,
}

impl<'s> LexState<'s, '_> {
    fn new(file: &FileName) -> LexState<'s, '_> {
        LexState {
            file,
            tokens: Vec::new(),
            comments: Vec::new(),
            idx: 0,
            row: 0,
            column: 0,
            modes: vec![LexMode::Normal],
        }
    }

    fn finish(self) -> Lexed<'s> {
        Lexed {
            tokens: self.tokens,
            comments: self.comments,
        }
    }

    fn start_point(&self) -> Point {
        Point {
            offset: self.idx.try_into().expect("position too large"),
            row: self.row.try_into().expect("row too large"),
            column: self.column.try_into().expect("column too large"),
        }
    }

    fn advance_to(&mut self, end: Point) {
        self.idx = end.offset as usize;
        self.row = end.row as usize;
        self.column = end.column as usize;
    }

    fn advance(&mut self, raw: &str) {
        let end = end_point_for_raw(self.start_point(), raw);
        self.advance_to(end);
    }

    fn push_token(&mut self, kind: TokenKind, raw: &'s str) {
        self.push_token_consumed(kind, raw, raw);
    }

    fn push_token_consumed(&mut self, kind: TokenKind, raw: &'s str, consumed: &str) {
        let start = self.start_point();
        let end = end_point_for_raw(start, consumed);
        self.tokens.push(Token {
            kind,
            raw,
            span: Span::At {
                start,
                end,
                file: self.file.clone(),
            },
        });
        self.advance_to(end);
    }

    fn push_comment(&mut self, kind: CommentKind, raw: &'s str) {
        let start = self.start_point();
        let end = end_point_for_raw(start, raw);
        self.comments.push(Comment {
            kind,
            raw,
            span: Span::At {
                start,
                end,
                file: self.file.clone(),
            },
        });
        self.advance_to(end);
    }
}

fn scan_string_content(input: &str) -> (usize, bool) {
    let mut idx = 1;
    while idx < input.len() {
        let rest = &input[idx..];
        let c = rest.chars().next().unwrap();
        match c {
            '"' => return (idx - 1, true),
            '\\' => {
                idx += c.len_utf8();
                if let Some(next) = input[idx..].chars().next() {
                    idx += next.len_utf8();
                }
            }
            _ => idx += c.len_utf8(),
        }
    }
    (input.len().saturating_sub(1), false)
}

fn scan_line_comment(input: &str) -> usize {
    input.find('\n').unwrap_or(input.len())
}

fn scan_block_comment(input: &str) -> Option<usize> {
    if !input.starts_with("/*") {
        return None;
    }

    let mut idx = 2;
    let mut nesting = 0usize;
    while idx < input.len() {
        let rest = &input[idx..];
        if rest.starts_with("/*") {
            nesting += 1;
            idx += 2;
        } else if rest.starts_with("*/") {
            if nesting == 0 {
                return Some(idx + 2);
            }
            nesting -= 1;
            idx += 2;
        } else {
            idx += rest.chars().next().unwrap().len_utf8();
        }
    }
    None
}

fn scan_template_text(input: &str) -> usize {
    let mut idx = 0;
    while idx < input.len() {
        let rest = &input[idx..];
        if rest.starts_with('`') || rest.starts_with("${") || rest.starts_with("#{") {
            break;
        }

        let c = rest.chars().next().unwrap();
        idx += c.len_utf8();
        if c == '\\'
            && let Some(next) = input[idx..].chars().next()
        {
            idx += next.len_utf8();
        }
    }
    idx
}

pub(crate) fn unescape_template_text(raw: &str) -> unescaper::Result<String> {
    let mut rewritten = String::with_capacity(raw.len());
    let mut rest = raw;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("\\`") {
            rewritten.push_str("\\u{60}");
            rest = after;
        } else if let Some(after) = rest.strip_prefix("\\${") {
            rewritten.push_str("\\u{24}{");
            rest = after;
        } else if let Some(after) = rest.strip_prefix("\\#{") {
            rewritten.push_str("\\u{23}{");
            rest = after;
        } else {
            let c = rest.chars().next().unwrap();
            rewritten.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    unescaper::unescape(&rewritten)
}

pub(crate) fn lex_with_comments<'s>(input: &'s str, file: &FileName) -> Lexed<'s> {
    let mut state = LexState::new(file);

    while state.idx < input.len() {
        let rest = &input[state.idx..];

        if matches!(state.modes.last(), Some(LexMode::Template)) {
            if let Some(raw) = rest.strip_prefix('`').map(|_| &rest[..1]) {
                state.push_token(TokenKind::TemplateEnd, raw);
                state.modes.pop();
                continue;
            }
            if let Some(raw) = rest.strip_prefix("${").map(|_| &rest[..2]) {
                state.push_token(TokenKind::TemplateStringStart, raw);
                state.modes.push(LexMode::Interpolation { brace_depth: 0 });
                continue;
            }
            if let Some(raw) = rest.strip_prefix("#{").map(|_| &rest[..2]) {
                state.push_token(TokenKind::TemplateDataStart, raw);
                state.modes.push(LexMode::Interpolation { brace_depth: 0 });
                continue;
            }

            let len = scan_template_text(rest);
            if len == 0 {
                let raw = &rest[..rest.chars().next().unwrap().len_utf8()];
                state.push_token(TokenKind::Unknown, raw);
            } else {
                let raw = &rest[..len];
                let kind = if unescape_template_text(raw).is_ok() {
                    TokenKind::TemplateText
                } else {
                    TokenKind::InvalidString
                };
                state.push_token(kind, raw);
            }
            continue;
        }

        let c = rest.chars().next().unwrap();
        match c {
            '-' => {
                if rest.starts_with("->") {
                    let raw = &rest[..2];
                    state.push_token(TokenKind::ThinArrow, raw);
                } else if rest.starts_with("-=") {
                    let raw = &rest[..2];
                    state.push_token(TokenKind::MinusEq, raw);
                } else if let Some((raw, kind, len)) = scan_number_token(rest) {
                    state.push_token_consumed(kind, raw, &rest[..len]);
                } else {
                    let raw = &rest[..1];
                    state.push_token(TokenKind::Minus, raw);
                }
            }
            '0'..='9' | '+' => {
                if rest.starts_with("+=") {
                    let raw = &rest[..2];
                    state.push_token(TokenKind::PlusEq, raw);
                } else if let Some((raw, kind, len)) = scan_number_token(rest) {
                    state.push_token_consumed(kind, raw, &rest[..len]);
                } else {
                    let raw = &rest[..1];
                    state.push_token(TokenKind::Plus, raw);
                }
            }
            '"' => {
                let (content_len, is_closed) = scan_string_content(rest);
                let raw = &rest[1..1 + content_len];
                let consumed_len = raw.len() + 1 + usize::from(is_closed);
                let kind = if is_closed && unescaper::unescape(raw).is_ok() {
                    TokenKind::String
                } else {
                    TokenKind::InvalidString
                };
                state.push_token_consumed(kind, raw, &rest[..consumed_len]);
            }
            c if is_identifier_start(c) => {
                let len = rest
                    .char_indices()
                    .take_while(|(_, c)| is_identifier_continue(*c))
                    .last()
                    .map(|(idx, c)| idx + c.len_utf8())
                    .unwrap_or(0);
                let raw = &rest[..len];
                let kind = identifier_kind(raw).expect("scanned identifier should be valid");
                state.push_token(kind, raw);
            }
            '\n' | ' ' | '\t' | '\r' => {
                let raw = &rest[..c.len_utf8()];
                state.advance(raw);
            }
            '`' => {
                let raw = &rest[..1];
                state.push_token(TokenKind::TemplateStart, raw);
                state.modes.push(LexMode::Template);
            }
            ':' => {
                let raw = &rest[..1];
                state.push_token(TokenKind::Colon, raw);
            }
            ';' => {
                let raw = &rest[..1];
                state.push_token(TokenKind::Semicolon, raw);
            }
            '[' => {
                let raw = &rest[..1];
                state.push_token(TokenKind::LBrack, raw);
            }
            ']' => {
                let raw = &rest[..1];
                state.push_token(TokenKind::RBrack, raw);
            }
            '(' => {
                let raw = &rest[..1];
                state.push_token(TokenKind::LParen, raw);
            }
            ')' => {
                let raw = &rest[..1];
                state.push_token(TokenKind::RParen, raw);
            }
            '{' => {
                if let Some(LexMode::Interpolation { brace_depth }) = state.modes.last_mut() {
                    *brace_depth += 1;
                }
                let raw = &rest[..1];
                state.push_token(TokenKind::LCurly, raw);
            }
            '}' => {
                let should_close_interpolation = matches!(
                    state.modes.last(),
                    Some(LexMode::Interpolation { brace_depth: 0 })
                );
                if should_close_interpolation {
                    state.modes.pop();
                } else if let Some(LexMode::Interpolation { brace_depth }) = state.modes.last_mut()
                {
                    *brace_depth -= 1;
                }
                let raw = &rest[..1];
                state.push_token(TokenKind::RCurly, raw);
            }
            '<' => {
                let (kind, len) = if rest.starts_with("<>") {
                    (TokenKind::Link, 2)
                } else if rest.starts_with("<=") {
                    (TokenKind::LtEq, 2)
                } else {
                    (TokenKind::Lt, 1)
                };
                let raw = &rest[..len];
                state.push_token(kind, raw);
            }
            '>' => {
                let (kind, len) = if rest.starts_with(">=") {
                    (TokenKind::GtEq, 2)
                } else {
                    (TokenKind::Gt, 1)
                };
                let raw = &rest[..len];
                state.push_token(kind, raw);
            }
            '/' => {
                if rest.starts_with("//") {
                    let len = scan_line_comment(rest);
                    let raw = &rest[..len];
                    state.push_comment(CommentKind::Line, raw);
                } else if let Some(len) = scan_block_comment(rest) {
                    let raw = &rest[..len];
                    state.push_comment(CommentKind::Block, raw);
                } else {
                    let (kind, len) = if rest.starts_with("/=") {
                        (TokenKind::SlashEq, 2)
                    } else {
                        (TokenKind::Slash, 1)
                    };
                    let raw = &rest[..len];
                    state.push_token(kind, raw);
                }
            }
            '@' => {
                let raw = &rest[..1];
                state.push_token(TokenKind::At, raw);
            }
            ',' => {
                let raw = &rest[..1];
                state.push_token(TokenKind::Comma, raw);
            }
            '.' => {
                let raw = &rest[..1];
                state.push_token(TokenKind::Dot, raw);
            }
            '=' => {
                let (kind, len) = if rest.starts_with("=>") {
                    (TokenKind::FatArrow, 2)
                } else if rest.starts_with("==") {
                    (TokenKind::EqEq, 2)
                } else {
                    (TokenKind::Eq, 1)
                };
                let raw = &rest[..len];
                state.push_token(kind, raw);
            }
            '!' => {
                let (kind, len) = if rest.starts_with("!=") {
                    (TokenKind::BangEq, 2)
                } else {
                    (TokenKind::Bang, 1)
                };
                let raw = &rest[..len];
                state.push_token(kind, raw);
            }
            '?' => {
                let raw = &rest[..1];
                state.push_token(TokenKind::Quest, raw);
            }
            '*' => {
                let (kind, len) = if rest.starts_with("*=") {
                    (TokenKind::StarEq, 2)
                } else {
                    (TokenKind::Star, 1)
                };
                let raw = &rest[..len];
                state.push_token(kind, raw);
            }
            _ => {
                let len = c.len_utf8();
                let raw = &rest[..len];
                state.push_token(TokenKind::Unknown, raw);
            }
        }
    }

    state.finish()
}

fn end_point_for_raw(start: Point, raw: &str) -> Point {
    let newline_count = raw.bytes().filter(|&byte| byte == b'\n').count() as u32;
    let end_column = match raw.rsplit_once('\n') {
        Some((_, last_line)) => last_line.encode_utf16().count() as u32,
        None => start.column + raw.encode_utf16().count() as u32,
    };
    Point {
        offset: start.offset + raw.len() as u32,
        row: start.row + newline_count,
        column: end_column,
    }
}

#[cfg(test)]
mod lexer_test {
    use super::*;

    const FILE: FileName = FileName(arcstr::literal!("Test.par"));

    #[test]
    fn columns_count_utf16_code_units() {
        let start = Point {
            offset: 4,
            row: 2,
            column: 3,
        };
        assert_eq!(
            end_point_for_raw(start, "é😀"),
            Point {
                offset: 10,
                row: 2,
                column: 6,
            }
        );
        assert_eq!(
            end_point_for_raw(start, "é\n😀"),
            Point {
                offset: 11,
                row: 3,
                column: 2,
            }
        );
    }

    #[test]
    fn tok() {
        let tokens = lex(&mut "({[< ><>]}):Abc:abc_123: A\n_Ab", &FILE);
        assert_eq!(
            tokens.iter().map(|x| x.kind).collect::<Vec<_>>(),
            vec![
                TokenKind::LParen,
                TokenKind::LCurly,
                TokenKind::LBrack,
                TokenKind::Lt,
                TokenKind::Gt,
                TokenKind::Link,
                TokenKind::RBrack,
                TokenKind::RCurly,
                TokenKind::RParen,
                TokenKind::Colon,
                TokenKind::UppercaseIdentifier,
                TokenKind::Colon,
                TokenKind::LowercaseIdentifier,
                TokenKind::Colon,
                TokenKind::UppercaseIdentifier,
                TokenKind::LowercaseIdentifier,
            ]
        );
        eprintln!("{:#?}", tokens);
    }

    #[test]
    fn block_comment() {
        let tokens = lex(
            &mut "abc/*\n\n/* comment \n*///\n*/ not_a_comment /*  */ /",
            &FILE,
        );
        eprintln!("{:#?}", tokens);
        assert_eq!(
            tokens,
            vec![
                Token {
                    kind: TokenKind::LowercaseIdentifier,
                    raw: "abc",
                    span: Span::At {
                        start: Point {
                            offset: 0,
                            row: 0,
                            column: 0
                        },
                        end: Point {
                            offset: 3,
                            row: 0,
                            column: 3
                        },
                        file: FILE
                    },
                },
                Token {
                    kind: TokenKind::LowercaseIdentifier,
                    raw: "not_a_comment",
                    span: Span::At {
                        start: Point {
                            offset: 27,
                            row: 4,
                            column: 3
                        },
                        end: Point {
                            offset: 40,
                            row: 4,
                            column: 16
                        },
                        file: FILE
                    },
                },
                Token {
                    kind: TokenKind::Slash,
                    raw: "/",
                    span: Span::At {
                        start: Point {
                            offset: 48,
                            row: 4,
                            column: 24
                        },
                        end: Point {
                            offset: 49,
                            row: 4,
                            column: 25
                        },
                        file: FILE
                    },
                }
            ]
        )
    }

    #[test]
    fn float_literals_tokenize_as_single_tokens() {
        let tokens = lex("1.0 -0.5 +3.25 1_000.25 6.02e23 1.0e-6", &FILE);
        assert_eq!(
            tokens.iter().map(|token| token.kind).collect::<Vec<_>>(),
            vec![
                TokenKind::Float,
                TokenKind::Float,
                TokenKind::Float,
                TokenKind::Float,
                TokenKind::Float,
                TokenKind::Float,
            ]
        );
        assert_eq!(
            tokens.iter().map(|token| token.raw).collect::<Vec<_>>(),
            vec!["1.0", "-0.5", "+3.25", "1_000.25", "6.02e23", "1.0e-6"]
        );
    }

    #[test]
    fn invalid_fraction_underscore_stays_one_unknown_token() {
        let tokens = lex("1._0", &FILE);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, TokenKind::Unknown);
        assert_eq!(tokens[0].raw, "1._0");
    }

    #[test]
    fn operator_tokens_coexist_with_attached_signed_literals() {
        let tokens = lex(
            "x-1 x+1 x - 1 x + 1 x += 1 y -= 2 z *= 3 w /= 4 neg <= >= == !=",
            &FILE,
        );
        assert_eq!(
            tokens.iter().map(|token| token.kind).collect::<Vec<_>>(),
            vec![
                TokenKind::LowercaseIdentifier,
                TokenKind::Integer,
                TokenKind::LowercaseIdentifier,
                TokenKind::Integer,
                TokenKind::LowercaseIdentifier,
                TokenKind::Minus,
                TokenKind::Integer,
                TokenKind::LowercaseIdentifier,
                TokenKind::Plus,
                TokenKind::Integer,
                TokenKind::LowercaseIdentifier,
                TokenKind::PlusEq,
                TokenKind::Integer,
                TokenKind::LowercaseIdentifier,
                TokenKind::MinusEq,
                TokenKind::Integer,
                TokenKind::LowercaseIdentifier,
                TokenKind::StarEq,
                TokenKind::Integer,
                TokenKind::LowercaseIdentifier,
                TokenKind::SlashEq,
                TokenKind::Integer,
                TokenKind::Neg,
                TokenKind::LtEq,
                TokenKind::GtEq,
                TokenKind::EqEq,
                TokenKind::BangEq,
            ]
        );
        assert_eq!(
            tokens.iter().map(|token| token.raw).collect::<Vec<_>>(),
            vec![
                "x", "-1", "x", "+1", "x", "-", "1", "x", "+", "1", "x", "+=", "1", "y", "-=", "2",
                "z", "*=", "3", "w", "/=", "4", "neg", "<=", ">=", "==", "!=",
            ]
        );
    }

    #[test]
    fn template_strings_tokenize_text_and_interpolations() {
        let tokens = lex(r#"`hi ${name} #{1 + {2}} \` \${ \#{`"#, &FILE);
        assert_eq!(
            tokens.iter().map(|token| token.kind).collect::<Vec<_>>(),
            vec![
                TokenKind::TemplateStart,
                TokenKind::TemplateText,
                TokenKind::TemplateStringStart,
                TokenKind::LowercaseIdentifier,
                TokenKind::RCurly,
                TokenKind::TemplateText,
                TokenKind::TemplateDataStart,
                TokenKind::Integer,
                TokenKind::Plus,
                TokenKind::LCurly,
                TokenKind::Integer,
                TokenKind::RCurly,
                TokenKind::RCurly,
                TokenKind::TemplateText,
                TokenKind::TemplateEnd,
            ]
        );
        assert_eq!(
            tokens.iter().map(|token| token.raw).collect::<Vec<_>>(),
            vec![
                "`",
                "hi ",
                "${",
                "name",
                "}",
                " ",
                "#{",
                "1",
                "+",
                "{",
                "2",
                "}",
                "}",
                r#" \` \${ \#{"#,
                "`",
            ]
        );
    }

    #[test]
    fn template_text_tracks_multiline_spans() {
        let tokens = lex("`a\nb`", &FILE);
        assert_eq!(tokens[1].kind, TokenKind::TemplateText);
        assert_eq!(
            tokens[1].span,
            Span::At {
                start: Point {
                    offset: 1,
                    row: 0,
                    column: 1
                },
                end: Point {
                    offset: 4,
                    row: 1,
                    column: 1
                },
                file: FILE
            }
        );
        assert_eq!(
            tokens[2].span,
            Span::At {
                start: Point {
                    offset: 4,
                    row: 1,
                    column: 1
                },
                end: Point {
                    offset: 5,
                    row: 1,
                    column: 2
                },
                file: FILE
            }
        );
    }
}
