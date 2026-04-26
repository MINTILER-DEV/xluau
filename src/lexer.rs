use crate::diagnostic::{Diagnostic, Span};
use crate::source::SourceFile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub lexeme: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Identifier,
    Keyword(Keyword),
    Number,
    String,
    BacktickString,
    TripleString,
    RawTripleString,
    Comment,
    Whitespace,
    Newline,
    Symbol(Symbol),
    Unknown,
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    And,
    Break,
    Case,
    Class,
    Const,
    Continue,
    Default,
    Do,
    Else,
    ElseIf,
    End,
    Enum,
    Export,
    False,
    For,
    Function,
    If,
    Import,
    In,
    Let,
    Local,
    Macro,
    Nil,
    Not,
    Or,
    Fallthrough,
    Repeat,
    Return,
    Switch,
    Then,
    True,
    Type,
    Until,
    While,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Symbol {
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    LeftBracket,
    RightBracket,
    Comma,
    Dot,
    Colon,
    DoubleColon,
    Semicolon,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Hash,
    Assign,
    Equals,
    NotEquals,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Question,
    QuestionDot,
    QuestionQuestion,
    QuestionQuestionEqual,
    Pipe,
    PipeGreater,
    FatArrow,
    DotDot,
    DotDotDot,
    At,
}

#[derive(Debug)]
pub struct Lexer<'a> {
    source: &'a SourceFile,
    input: &'a str,
    cursor: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a SourceFile) -> Self {
        Self {
            source,
            input: &source.text,
            cursor: 0,
        }
    }

    pub fn lex(mut self, diagnostics: &mut Vec<Diagnostic>) -> Vec<Token> {
        let mut tokens = Vec::new();

        while self.cursor < self.input.len() {
            let start = self.cursor;
            let ch = self.peek_char().expect("cursor within bounds");

            let token = match ch {
                ' ' | '\t' => self.lex_whitespace(),
                '\n' => {
                    self.bump_char();
                    Token::new(TokenKind::Newline, "\n".to_owned(), start, self.cursor)
                }
                '\r' => {
                    self.bump_char();
                    if self.peek_char() == Some('\n') {
                        self.bump_char();
                    }
                    Token::new(TokenKind::Newline, "\n".to_owned(), start, self.cursor)
                }
                '-' if self.peek_str("--") => self.lex_comment(),
                'r' if self.peek_str("r\"\"\"") => self.lex_triple_string(true, diagnostics),
                '"' if self.peek_str("\"\"\"") => self.lex_triple_string(false, diagnostics),
                '"' => self.lex_string('"', TokenKind::String, diagnostics),
                '\'' => self.lex_string('\'', TokenKind::String, diagnostics),
                '`' => self.lex_string('`', TokenKind::BacktickString, diagnostics),
                '[' if self.looks_like_long_bracket(self.cursor) => {
                    self.lex_long_bracket_string(diagnostics)
                }
                ch if is_identifier_start(ch) => self.lex_identifier_or_keyword(),
                ch if ch.is_ascii_digit() => self.lex_number(),
                _ => self.lex_symbol_or_unknown(diagnostics),
            };

            tokens.push(token);
        }

        tokens.push(Token::new(
            TokenKind::Eof,
            String::new(),
            self.cursor,
            self.cursor,
        ));

        tokens
    }

    fn lex_whitespace(&mut self) -> Token {
        let start = self.cursor;
        while matches!(self.peek_char(), Some(' ' | '\t')) {
            self.bump_char();
        }
        self.token(TokenKind::Whitespace, start)
    }

    fn lex_comment(&mut self) -> Token {
        let start = self.cursor;
        self.bump_str("--");

        if self.looks_like_long_bracket(self.cursor) {
            let _ = self.consume_long_bracket();
            return self.token(TokenKind::Comment, start);
        }

        while let Some(ch) = self.peek_char() {
            if ch == '\n' || ch == '\r' {
                break;
            }
            self.bump_char();
        }

        self.token(TokenKind::Comment, start)
    }

    fn lex_string(
        &mut self,
        delimiter: char,
        kind: TokenKind,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Token {
        let start = self.cursor;
        self.bump_char();
        let mut escaped = false;
        let mut terminated = false;

        while let Some(ch) = self.peek_char() {
            self.bump_char();

            if escaped {
                escaped = false;
                continue;
            }

            if ch == '\\' && delimiter != '`' {
                escaped = true;
                continue;
            }

            if ch == delimiter {
                terminated = true;
                break;
            }
        }

        if !terminated {
            diagnostics.push(Diagnostic::error(
                Some(&self.source.path),
                Some(Span::new(start, self.cursor)),
                "unterminated string literal",
            ));
        }

        self.token(kind, start)
    }

    fn lex_triple_string(&mut self, raw: bool, diagnostics: &mut Vec<Diagnostic>) -> Token {
        let start = self.cursor;
        if raw {
            self.bump_str("r\"\"\"");
        } else {
            self.bump_str("\"\"\"");
        }

        let terminator = "\"\"\"";
        let mut terminated = false;
        while self.cursor < self.input.len() {
            if self.peek_str(terminator) {
                self.bump_str(terminator);
                terminated = true;
                break;
            }
            self.bump_char();
        }

        if !terminated {
            diagnostics.push(Diagnostic::error(
                Some(&self.source.path),
                Some(Span::new(start, self.cursor)),
                "unterminated triple string literal",
            ));
        }

        let kind = if raw {
            TokenKind::RawTripleString
        } else {
            TokenKind::TripleString
        };
        self.token(kind, start)
    }

    fn lex_long_bracket_string(&mut self, diagnostics: &mut Vec<Diagnostic>) -> Token {
        let start = self.cursor;
        if !self.consume_long_bracket() {
            diagnostics.push(Diagnostic::error(
                Some(&self.source.path),
                Some(Span::new(start, self.cursor)),
                "unterminated long bracket literal",
            ));
        }
        self.token(TokenKind::String, start)
    }

    fn lex_identifier_or_keyword(&mut self) -> Token {
        let start = self.cursor;
        self.bump_char();
        while let Some(ch) = self.peek_char() {
            if is_identifier_continue(ch) {
                self.bump_char();
            } else {
                break;
            }
        }

        let lexeme = &self.input[start..self.cursor];
        let kind = keyword_for(lexeme)
            .map(TokenKind::Keyword)
            .unwrap_or(TokenKind::Identifier);
        self.token(kind, start)
    }

    fn lex_number(&mut self) -> Token {
        let start = self.cursor;
        while let Some(ch) = self.peek_char() {
            if ch.is_ascii_alphanumeric()
                || matches!(ch, '.' | '_' | '+' | '-') && !matches!(self.peek_str(".."), true)
            {
                self.bump_char();
            } else {
                break;
            }
        }

        self.token(TokenKind::Number, start)
    }

    fn lex_symbol_or_unknown(&mut self, diagnostics: &mut Vec<Diagnostic>) -> Token {
        const SYMBOLS: &[(&str, Symbol)] = &[
            ("??=", Symbol::QuestionQuestionEqual),
            ("?.", Symbol::QuestionDot),
            ("??", Symbol::QuestionQuestion),
            ("|>", Symbol::PipeGreater),
            ("=>", Symbol::FatArrow),
            ("::", Symbol::DoubleColon),
            ("...", Symbol::DotDotDot),
            ("==", Symbol::Equals),
            ("~=", Symbol::NotEquals),
            ("<=", Symbol::LessEqual),
            (">=", Symbol::GreaterEqual),
            ("..", Symbol::DotDot),
        ];

        let start = self.cursor;
        for (text, symbol) in SYMBOLS {
            if self.peek_str(text) {
                self.bump_str(text);
                return self.token(TokenKind::Symbol(*symbol), start);
            }
        }

        let ch = self.bump_char().expect("char available");
        let kind = match ch {
            '(' => TokenKind::Symbol(Symbol::LeftParen),
            ')' => TokenKind::Symbol(Symbol::RightParen),
            '{' => TokenKind::Symbol(Symbol::LeftBrace),
            '}' => TokenKind::Symbol(Symbol::RightBrace),
            '[' => TokenKind::Symbol(Symbol::LeftBracket),
            ']' => TokenKind::Symbol(Symbol::RightBracket),
            ',' => TokenKind::Symbol(Symbol::Comma),
            '.' => TokenKind::Symbol(Symbol::Dot),
            ':' => TokenKind::Symbol(Symbol::Colon),
            ';' => TokenKind::Symbol(Symbol::Semicolon),
            '+' => TokenKind::Symbol(Symbol::Plus),
            '-' => TokenKind::Symbol(Symbol::Minus),
            '*' => TokenKind::Symbol(Symbol::Star),
            '/' => TokenKind::Symbol(Symbol::Slash),
            '%' => TokenKind::Symbol(Symbol::Percent),
            '^' => TokenKind::Symbol(Symbol::Caret),
            '#' => TokenKind::Symbol(Symbol::Hash),
            '=' => TokenKind::Symbol(Symbol::Assign),
            '<' => TokenKind::Symbol(Symbol::Less),
            '>' => TokenKind::Symbol(Symbol::Greater),
            '?' => TokenKind::Symbol(Symbol::Question),
            '|' => TokenKind::Symbol(Symbol::Pipe),
            '@' => TokenKind::Symbol(Symbol::At),
            _ => {
                diagnostics.push(Diagnostic::warning(
                    Some(&self.source.path),
                    Some(Span::new(start, self.cursor)),
                    format!("unrecognized token `{ch}`"),
                ));
                TokenKind::Unknown
            }
        };

        self.token(kind, start)
    }

    fn token(&self, kind: TokenKind, start: usize) -> Token {
        Token::new(
            kind,
            self.input[start..self.cursor].to_owned(),
            start,
            self.cursor,
        )
    }

    fn peek_char(&self) -> Option<char> {
        self.input[self.cursor..].chars().next()
    }

    fn bump_char(&mut self) -> Option<char> {
        let ch = self.peek_char()?;
        self.cursor += ch.len_utf8();
        Some(ch)
    }

    fn peek_str(&self, expected: &str) -> bool {
        self.input[self.cursor..].starts_with(expected)
    }

    fn bump_str(&mut self, expected: &str) {
        debug_assert!(self.peek_str(expected));
        self.cursor += expected.len();
    }

    fn looks_like_long_bracket(&self, start: usize) -> bool {
        let bytes = self.input.as_bytes();
        if bytes.get(start) != Some(&b'[') {
            return false;
        }

        let mut index = start + 1;
        while bytes.get(index) == Some(&b'=') {
            index += 1;
        }

        bytes.get(index) == Some(&b'[')
    }

    fn consume_long_bracket(&mut self) -> bool {
        let bytes = self.input.as_bytes();
        let start = self.cursor;
        if bytes.get(start) != Some(&b'[') {
            return false;
        }

        let mut index = start + 1;
        while bytes.get(index) == Some(&b'=') {
            index += 1;
        }
        if bytes.get(index) != Some(&b'[') {
            return false;
        }

        let equals_count = index - start - 1;
        self.cursor = index + 1;
        let close = format!("]{}]", "=".repeat(equals_count));

        while self.cursor < self.input.len() {
            if self.peek_str(&close) {
                self.bump_str(&close);
                return true;
            }
            let _ = self.bump_char();
        }

        false
    }
}

impl Token {
    fn new(kind: TokenKind, lexeme: String, start: usize, end: usize) -> Self {
        Self {
            kind,
            lexeme,
            span: Span::new(start, end),
        }
    }

    pub fn is_trivia(&self) -> bool {
        matches!(
            self.kind,
            TokenKind::Whitespace | TokenKind::Newline | TokenKind::Comment
        )
    }
}

fn keyword_for(lexeme: &str) -> Option<Keyword> {
    Some(match lexeme {
        "and" => Keyword::And,
        "break" => Keyword::Break,
        "case" => Keyword::Case,
        "class" => Keyword::Class,
        "const" => Keyword::Const,
        "continue" => Keyword::Continue,
        "default" => Keyword::Default,
        "do" => Keyword::Do,
        "else" => Keyword::Else,
        "elseif" => Keyword::ElseIf,
        "end" => Keyword::End,
        "enum" => Keyword::Enum,
        "export" => Keyword::Export,
        "false" => Keyword::False,
        "for" => Keyword::For,
        "fallthrough" => Keyword::Fallthrough,
        "function" => Keyword::Function,
        "if" => Keyword::If,
        "import" => Keyword::Import,
        "in" => Keyword::In,
        "let" => Keyword::Let,
        "local" => Keyword::Local,
        "macro" => Keyword::Macro,
        "nil" => Keyword::Nil,
        "not" => Keyword::Not,
        "or" => Keyword::Or,
        "repeat" => Keyword::Repeat,
        "return" => Keyword::Return,
        "switch" => Keyword::Switch,
        "then" => Keyword::Then,
        "true" => Keyword::True,
        "type" => Keyword::Type,
        "until" => Keyword::Until,
        "while" => Keyword::While,
        _ => return None,
    })
}

fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_identifier_continue(ch: char) -> bool {
    is_identifier_start(ch) || ch.is_ascii_digit()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Keyword, Lexer, Symbol, TokenKind};
    use crate::source::{SourceFile, SourceKind};

    #[test]
    fn lexer_recognizes_phase_two_extension_tokens() {
        let source = SourceFile {
            path: PathBuf::from("test.xl"),
            kind: SourceKind::XLuau,
            text: "import foo\nvalue?.bar ?? fallback |> transform".to_owned(),
        };
        let tokens = Lexer::new(&source).lex(&mut Vec::new());

        assert!(
            tokens
                .iter()
                .any(|token| token.kind == TokenKind::Keyword(Keyword::Import))
        );
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == TokenKind::Symbol(Symbol::QuestionDot))
        );
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == TokenKind::Symbol(Symbol::QuestionQuestion))
        );
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == TokenKind::Symbol(Symbol::PipeGreater))
        );
    }
}
