use crate::ast::{Program, Statement, StatementKind};
use crate::diagnostic::{Diagnostic, Span};
use crate::lexer::{Keyword, Symbol, Token, TokenKind};
use crate::source::SourceFile;

#[derive(Debug)]
pub struct Parser<'a> {
    source: &'a SourceFile,
    tokens: &'a [Token],
}

impl<'a> Parser<'a> {
    pub fn new(source: &'a SourceFile, tokens: &'a [Token]) -> Self {
        Self { source, tokens }
    }

    pub fn parse(&self, diagnostics: &mut Vec<Diagnostic>) -> Program {
        self.validate_delimiters(diagnostics);

        let mut statements = Vec::new();
        let mut start = 0usize;
        let mut paren_depth = 0usize;
        let mut brace_depth = 0usize;
        let mut bracket_depth = 0usize;
        let mut block_depth = 0usize;

        for (index, token) in self.tokens.iter().enumerate() {
            match token.kind {
                TokenKind::Symbol(Symbol::LeftParen) => paren_depth += 1,
                TokenKind::Symbol(Symbol::RightParen) => {
                    paren_depth = paren_depth.saturating_sub(1);
                }
                TokenKind::Symbol(Symbol::LeftBrace) => brace_depth += 1,
                TokenKind::Symbol(Symbol::RightBrace) => {
                    brace_depth = brace_depth.saturating_sub(1);
                }
                TokenKind::Symbol(Symbol::LeftBracket) => bracket_depth += 1,
                TokenKind::Symbol(Symbol::RightBracket) => {
                    bracket_depth = bracket_depth.saturating_sub(1);
                }
                TokenKind::Keyword(keyword) => match keyword {
                    Keyword::Function
                    | Keyword::If
                    | Keyword::For
                    | Keyword::While
                    | Keyword::Repeat
                    | Keyword::Switch => block_depth += 1,
                    Keyword::End | Keyword::Until => {
                        block_depth = block_depth.saturating_sub(1);
                    }
                    _ => {}
                },
                _ => {}
            }

            let at_top_level =
                paren_depth == 0 && brace_depth == 0 && bracket_depth == 0 && block_depth == 0;
            let is_boundary = at_top_level
                && matches!(
                    token.kind,
                    TokenKind::Newline | TokenKind::Symbol(Symbol::Semicolon) | TokenKind::Eof
                );

            if is_boundary {
                let end = if matches!(token.kind, TokenKind::Eof) {
                    index
                } else {
                    index + 1
                };

                if let Some(statement) = self.make_statement(start, end) {
                    statements.push(statement);
                }
                start = index + 1;
            }
        }

        Program {
            source_kind: self.source.kind,
            span: Span::new(0, self.source.text.len()),
            statements,
        }
    }

    fn validate_delimiters(&self, diagnostics: &mut Vec<Diagnostic>) {
        let mut stack: Vec<(Symbol, Span)> = Vec::new();

        for token in self.tokens {
            match token.kind {
                TokenKind::Symbol(Symbol::LeftParen)
                | TokenKind::Symbol(Symbol::LeftBrace)
                | TokenKind::Symbol(Symbol::LeftBracket) => {
                    if let TokenKind::Symbol(symbol) = token.kind {
                        stack.push((symbol, token.span));
                    }
                }
                TokenKind::Symbol(Symbol::RightParen) => {
                    self.match_symbol(Symbol::LeftParen, token.span, &mut stack, diagnostics)
                }
                TokenKind::Symbol(Symbol::RightBrace) => {
                    self.match_symbol(Symbol::LeftBrace, token.span, &mut stack, diagnostics)
                }
                TokenKind::Symbol(Symbol::RightBracket) => {
                    self.match_symbol(Symbol::LeftBracket, token.span, &mut stack, diagnostics)
                }
                _ => {}
            }
        }

        for (symbol, span) in stack {
            diagnostics.push(Diagnostic::error(
                Some(&self.source.path),
                Some(span),
                format!("unclosed delimiter `{}`", symbol_text(symbol)),
            ));
        }
    }

    fn match_symbol(
        &self,
        expected: Symbol,
        span: Span,
        stack: &mut Vec<(Symbol, Span)>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        match stack.pop() {
            Some((found, _)) if found == expected => {}
            Some((found, found_span)) => diagnostics.push(Diagnostic::error(
                Some(&self.source.path),
                Some(found_span),
                format!(
                    "mismatched delimiter: expected `{}`, found `{}`",
                    symbol_text(found),
                    symbol_text(expected)
                ),
            )),
            None => diagnostics.push(Diagnostic::error(
                Some(&self.source.path),
                Some(span),
                format!("unexpected closing delimiter `{}`", symbol_text(expected)),
            )),
        }
    }

    fn make_statement(&self, start: usize, end: usize) -> Option<Statement> {
        let slice = &self.tokens[start..end];
        if slice.is_empty() {
            return None;
        }

        let raw_text = slice
            .iter()
            .map(|token| token.lexeme.as_str())
            .collect::<String>();
        let span = Span::new(slice.first()?.span.start, slice.last()?.span.end);
        let kind = classify_statement(slice);

        Some(Statement {
            kind,
            raw_text,
            span,
        })
    }
}

fn classify_statement(tokens: &[Token]) -> StatementKind {
    let significant: Vec<&Token> = tokens.iter().filter(|token| !token.is_trivia()).collect();
    if significant.is_empty() {
        return StatementKind::Whitespace;
    }

    if significant
        .iter()
        .all(|token| matches!(token.kind, TokenKind::Comment))
    {
        return StatementKind::Comment;
    }

    let first = significant[0];
    match first.kind {
        TokenKind::Keyword(Keyword::Import) => StatementKind::ImportDeclaration,
        TokenKind::Keyword(Keyword::Export) => StatementKind::ExportDeclaration,
        TokenKind::Keyword(Keyword::Type) => StatementKind::TypeDeclaration,
        TokenKind::Keyword(Keyword::Class)
        | TokenKind::Keyword(Keyword::Const)
        | TokenKind::Keyword(Keyword::Enum)
        | TokenKind::Keyword(Keyword::Macro)
        | TokenKind::Keyword(Keyword::Switch) => StatementKind::XLuauDeclaration,
        TokenKind::Symbol(Symbol::At) => StatementKind::XLuauDeclaration,
        _ if significant.iter().any(has_extension_token) => StatementKind::XLuauExpression,
        _ => StatementKind::Luau,
    }
}

fn has_extension_token(token: &&Token) -> bool {
    matches!(
        token.kind,
        TokenKind::Symbol(Symbol::QuestionDot)
            | TokenKind::Symbol(Symbol::QuestionQuestion)
            | TokenKind::Symbol(Symbol::QuestionQuestionEqual)
            | TokenKind::Symbol(Symbol::PipeGreater)
            | TokenKind::Symbol(Symbol::FatArrow)
    )
}

fn symbol_text(symbol: Symbol) -> &'static str {
    match symbol {
        Symbol::LeftParen => "(",
        Symbol::RightParen => ")",
        Symbol::LeftBrace => "{",
        Symbol::RightBrace => "}",
        Symbol::LeftBracket => "[",
        Symbol::RightBracket => "]",
        Symbol::Comma => ",",
        Symbol::Dot => ".",
        Symbol::Colon => ":",
        Symbol::DoubleColon => "::",
        Symbol::Semicolon => ";",
        Symbol::Plus => "+",
        Symbol::Minus => "-",
        Symbol::Star => "*",
        Symbol::Slash => "/",
        Symbol::Percent => "%",
        Symbol::Caret => "^",
        Symbol::Hash => "#",
        Symbol::Assign => "=",
        Symbol::Equals => "==",
        Symbol::NotEquals => "~=",
        Symbol::Less => "<",
        Symbol::LessEqual => "<=",
        Symbol::Greater => ">",
        Symbol::GreaterEqual => ">=",
        Symbol::Question => "?",
        Symbol::QuestionDot => "?.",
        Symbol::QuestionQuestion => "??",
        Symbol::QuestionQuestionEqual => "??=",
        Symbol::Pipe => "|",
        Symbol::PipeGreater => "|>",
        Symbol::FatArrow => "=>",
        Symbol::DotDot => "..",
        Symbol::DotDotDot => "...",
        Symbol::At => "@",
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::Parser;
    use crate::ast::StatementKind;
    use crate::lexer::Lexer;
    use crate::source::{SourceFile, SourceKind};

    #[test]
    fn parser_classifies_import_and_extension_statements() {
        let source = SourceFile {
            path: PathBuf::from("test.xl"),
            kind: SourceKind::XLuau,
            text: "import { value } from \"./mod\"\nlocal next = item?.value ?? 0".to_owned(),
        };
        let tokens = Lexer::new(&source).lex(&mut Vec::new());
        let program = Parser::new(&source, &tokens).parse(&mut Vec::new());

        assert_eq!(program.statements[0].kind, StatementKind::ImportDeclaration);
        assert_eq!(program.statements[1].kind, StatementKind::XLuauExpression);
    }

    #[test]
    fn parser_keeps_function_block_as_single_statement() {
        let source = SourceFile {
            path: PathBuf::from("test.xl"),
            kind: SourceKind::XLuau,
            text: "function demo()\n    local value = 1\n    return value\nend\nprint(demo())"
                .to_owned(),
        };
        let tokens = Lexer::new(&source).lex(&mut Vec::new());
        let program = Parser::new(&source, &tokens).parse(&mut Vec::new());

        assert_eq!(program.statements.len(), 2);
        assert!(program.statements[0].raw_text.contains("return value"));
    }
}
