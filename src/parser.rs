use crate::ast::{
    BlockStatement, ConditionalClause, ConditionalKeyword, ForStatement, FunctionStatement,
    IfStatement, ImportKind, ImportStatement, LocalKeyword, LocalStatement,
    NamedExportSpecifier, NamedImportSpecifier, Program, RepeatStatement, ReturnStatement,
    Statement, StatementKind, StatementNode, SwitchLabel, SwitchSection, SwitchStatement,
    WhileStatement, ExportKind, ExportStatement,
};
use crate::diagnostic::{Diagnostic, Span};
use crate::lexer::{Keyword, Lexer, Symbol, Token, TokenKind};
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

        let statements = self.collect_statements(0, self.tokens.len());

        Program {
            source_kind: self.source.kind,
            span: Span::new(0, self.source.text.len()),
            statements,
        }
    }

    fn collect_statements(&self, start: usize, end: usize) -> Vec<Statement> {
        let mut statements = Vec::new();
        let mut statement_start = start;
        let mut paren_depth = 0usize;
        let mut brace_depth = 0usize;
        let mut bracket_depth = 0usize;
        let mut block_depth = 0usize;

        for (index, token) in self.tokens.iter().enumerate().skip(start).take(end - start) {
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
                TokenKind::Keyword(keyword)
                    if brace_depth == 0 && paren_depth == 0 && bracket_depth == 0 =>
                {
                    match keyword {
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
                    }
                }
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
                let statement_end = if matches!(token.kind, TokenKind::Eof) {
                    index
                } else {
                    index + 1
                };

                if let Some(statement) = self.make_statement(statement_start, statement_end) {
                    statements.push(statement);
                }
                statement_start = index + 1;
            }
        }

        statements
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

        let full_text = slice
            .iter()
            .map(|token| token.lexeme.as_str())
            .collect::<String>();
        let (body_text, trailing) = split_statement_suffix(slice, &full_text);
        let span = Span::new(slice.first()?.span.start, slice.last()?.span.end);
        let kind = classify_statement(slice);
        let node = self.parse_statement_node(kind, body_text.as_str());

        Some(Statement {
            kind,
            node,
            trailing,
            span,
        })
    }

    fn parse_statement_node(&self, kind: StatementKind, text: &str) -> StatementNode {
        if matches!(kind, StatementKind::Comment | StatementKind::Whitespace) {
            return StatementNode::Trivia(text.to_owned());
        }

        let fragment = Fragment::new(self.source, text);
        let Some(first) = fragment.first_significant_index() else {
            return StatementNode::Trivia(text.to_owned());
        };

        match fragment.tokens[first].kind {
            TokenKind::Keyword(Keyword::Import) => self.parse_import_statement(text),
            TokenKind::Keyword(Keyword::Export) => self.parse_export_statement(text),
            TokenKind::Keyword(Keyword::Const) => self.parse_local_statement(text, LocalKeyword::Const),
            TokenKind::Keyword(Keyword::Let) => self.parse_local_statement(text, LocalKeyword::Let),
            TokenKind::Keyword(Keyword::Local) => {
                if let Some(next) = fragment.next_significant_after(first) {
                    if fragment.tokens[next].kind == TokenKind::Keyword(Keyword::Function) {
                        return self.parse_function_statement(text);
                    }
                }
                self.parse_local_statement(text, LocalKeyword::Local)
            }
            TokenKind::Keyword(Keyword::Return) => self.parse_return_statement(text),
            TokenKind::Keyword(Keyword::If) => self.parse_if_statement(text),
            TokenKind::Keyword(Keyword::While) => self.parse_while_statement(text),
            TokenKind::Keyword(Keyword::Repeat) => self.parse_repeat_statement(text),
            TokenKind::Keyword(Keyword::For) => self.parse_for_statement(text),
            TokenKind::Keyword(Keyword::Function) => self.parse_function_statement(text),
            TokenKind::Keyword(Keyword::Do) => self.parse_do_statement(text),
            TokenKind::Keyword(Keyword::Switch) => self.parse_switch_statement(text),
            _ => StatementNode::Text(text.to_owned()),
        }
    }

    fn parse_import_statement(&self, text: &str) -> StatementNode {
        let fragment = Fragment::new(self.source, text);
        let tokens = fragment.significant_tokens();
        let mut cursor = 1usize;

        let is_type_only = matches!(
            tokens.get(cursor).map(|token| token.kind.clone()),
            Some(TokenKind::Keyword(Keyword::Type))
        );
        if is_type_only {
            cursor += 1;
        }

        if tokens
            .get(cursor)
            .map(|token| matches!(token.kind, TokenKind::String))
            .unwrap_or(false)
        {
            return StatementNode::Import(ImportStatement {
                kind: ImportKind::SideEffect,
                source: unquote_string(tokens[cursor].lexeme.as_str()),
            });
        }

        let mut default = None;
        let mut namespace = None;
        let mut named = Vec::new();

        match tokens.get(cursor).map(|token| &token.kind) {
            Some(TokenKind::Symbol(Symbol::Star)) => {
                cursor += 1;
                if tokens
                    .get(cursor)
                    .map(|token| token.lexeme.as_str() == "as")
                    .unwrap_or(false)
                {
                    cursor += 1;
                }
                namespace = tokens.get(cursor).map(token_text_owned);
                cursor += 1;
            }
            Some(TokenKind::Symbol(Symbol::LeftBrace)) => {
                let end = fragment.matching_delimiter(
                    &tokens,
                    cursor,
                    Symbol::LeftBrace,
                    Symbol::RightBrace,
                );
                named = self.parse_named_import_specifiers(&fragment, &tokens, cursor, end);
                cursor = end + 1;
            }
            Some(_) => {
                default = tokens.get(cursor).map(token_text_owned);
                cursor += 1;
                if matches!(
                    tokens.get(cursor).map(|token| token.kind.clone()),
                    Some(TokenKind::Symbol(Symbol::Comma))
                ) {
                    cursor += 1;
                    match tokens.get(cursor).map(|token| token.kind.clone()) {
                        Some(TokenKind::Symbol(Symbol::Star)) => {
                            cursor += 1;
                            if tokens
                                .get(cursor)
                                .map(|token| token.lexeme.as_str() == "as")
                                .unwrap_or(false)
                            {
                                cursor += 1;
                            }
                            namespace = tokens.get(cursor).map(token_text_owned);
                            cursor += 1;
                        }
                        Some(TokenKind::Symbol(Symbol::LeftBrace)) => {
                            let end = fragment.matching_delimiter(
                                &tokens,
                                cursor,
                                Symbol::LeftBrace,
                                Symbol::RightBrace,
                            );
                            named =
                                self.parse_named_import_specifiers(&fragment, &tokens, cursor, end);
                            cursor = end + 1;
                        }
                        _ => {}
                    }
                }
            }
            None => {}
        }

        let source = tokens
            .iter()
            .skip(cursor)
            .find(|token| matches!(token.kind, TokenKind::String))
            .map(|token| unquote_string(token.lexeme.as_str()))
            .unwrap_or_default();

        let kind = if is_type_only {
            ImportKind::TypeNamed { named }
        } else {
            ImportKind::Value {
                default,
                namespace,
                named,
            }
        };

        StatementNode::Import(ImportStatement { kind, source })
    }

    fn parse_export_statement(&self, text: &str) -> StatementNode {
        let fragment = Fragment::new(self.source, text);
        let tokens = fragment.significant_tokens();
        if tokens.len() < 2 {
            return StatementNode::Export(ExportStatement {
                kind: ExportKind::Declaration(Box::new(StatementNode::Text(text.to_owned()))),
            });
        }
        let mut cursor = 1usize;

        let is_type_only = matches!(
            tokens.get(cursor).map(|token| token.kind.clone()),
            Some(TokenKind::Keyword(Keyword::Type))
        );
        if is_type_only {
            cursor += 1;
        }

        let kind = if is_type_only {
            match tokens.get(cursor).map(|token| token.kind.clone()) {
                Some(TokenKind::Symbol(Symbol::LeftBrace)) => {
                    let end = fragment.matching_delimiter(
                        &tokens,
                        cursor,
                        Symbol::LeftBrace,
                        Symbol::RightBrace,
                    );
                    let specifiers =
                        self.parse_named_export_specifiers(&fragment, &tokens, cursor, end);
                    let source = tokens
                        .iter()
                        .skip(end + 1)
                        .find(|token| matches!(token.kind, TokenKind::String))
                        .map(|token| unquote_string(token.lexeme.as_str()));
                    ExportKind::Named {
                        specifiers,
                        source,
                        is_type_only: true,
                    }
                }
                Some(TokenKind::Symbol(Symbol::Star)) => {
                    let source = tokens
                        .iter()
                        .skip(cursor + 1)
                        .find(|token| matches!(token.kind, TokenKind::String))
                        .map(|token| unquote_string(token.lexeme.as_str()))
                        .unwrap_or_default();
                    ExportKind::All {
                        source,
                        is_type_only: true,
                    }
                }
                _ => ExportKind::TypeDeclaration(
                    fragment
                        .text_between(tokens[1].span.start, text.len())
                        .trim()
                        .to_owned(),
                ),
            }
        } else {
            match tokens.get(cursor).map(|token| token.kind.clone()) {
            Some(TokenKind::Keyword(Keyword::Function)) => ExportKind::Declaration(Box::new(
                self.parse_statement_node(
                    StatementKind::Luau,
                    fragment.text_between(tokens[cursor].span.start, text.len()).as_str(),
                ),
            )),
            Some(TokenKind::Keyword(Keyword::Local)) => ExportKind::Declaration(Box::new(
                self.parse_statement_node(
                    StatementKind::Luau,
                    fragment.text_between(tokens[cursor].span.start, text.len()).as_str(),
                ),
            )),
            Some(TokenKind::Keyword(Keyword::Let)) => ExportKind::Declaration(Box::new(
                self.parse_statement_node(
                    StatementKind::XLuauDeclaration,
                    fragment.text_between(tokens[cursor].span.start, text.len()).as_str(),
                ),
            )),
            Some(TokenKind::Keyword(Keyword::Const)) => ExportKind::Declaration(Box::new(
                self.parse_statement_node(
                    StatementKind::XLuauDeclaration,
                    fragment.text_between(tokens[cursor].span.start, text.len()).as_str(),
                ),
            )),
            Some(TokenKind::Keyword(Keyword::Default)) if !is_type_only => {
                let expression = fragment
                    .text_between(tokens[cursor].span.end, text.len())
                    .trim()
                    .to_owned();
                ExportKind::Default { expression }
            }
            Some(TokenKind::Symbol(Symbol::LeftBrace)) => {
                let end = fragment.matching_delimiter(
                    &tokens,
                    cursor,
                    Symbol::LeftBrace,
                    Symbol::RightBrace,
                );
                let specifiers = self.parse_named_export_specifiers(&fragment, &tokens, cursor, end);
                let source = tokens
                    .iter()
                    .skip(end + 1)
                    .find(|token| matches!(token.kind, TokenKind::String))
                    .map(|token| unquote_string(token.lexeme.as_str()));
                ExportKind::Named {
                    specifiers,
                    source,
                    is_type_only,
                }
            }
            Some(TokenKind::Symbol(Symbol::Star)) => {
                let source = tokens
                    .iter()
                    .skip(cursor + 1)
                    .find(|token| matches!(token.kind, TokenKind::String))
                    .map(|token| unquote_string(token.lexeme.as_str()))
                    .unwrap_or_default();
                ExportKind::All {
                    source,
                    is_type_only,
                }
            }
            _ => ExportKind::Declaration(Box::new(StatementNode::Text(
                fragment
                    .text_between(tokens[cursor.min(tokens.len() - 1)].span.start, text.len())
                    .trim()
                    .to_owned(),
            ))),
        }};

        StatementNode::Export(ExportStatement { kind })
    }

    fn parse_named_import_specifiers(
        &self,
        fragment: &Fragment,
        tokens: &[Token],
        start: usize,
        end: usize,
    ) -> Vec<NamedImportSpecifier> {
        split_top_level(
            fragment
                .text_between(tokens[start].span.end, tokens[end].span.start)
                .as_str(),
            ',',
        )
        .into_iter()
        .filter_map(|entry| {
            let trimmed = entry.trim();
            if trimmed.is_empty() {
                return None;
            }
            let (imported, local) = if let Some((left, right)) = split_keyword_alias(trimmed, "as") {
                (left.trim().to_owned(), right.trim().to_owned())
            } else {
                (trimmed.to_owned(), trimmed.to_owned())
            };
            Some(NamedImportSpecifier { imported, local })
        })
        .collect()
    }

    fn parse_named_export_specifiers(
        &self,
        fragment: &Fragment,
        tokens: &[Token],
        start: usize,
        end: usize,
    ) -> Vec<NamedExportSpecifier> {
        split_top_level(
            fragment
                .text_between(tokens[start].span.end, tokens[end].span.start)
                .as_str(),
            ',',
        )
        .into_iter()
        .filter_map(|entry| {
            let trimmed = entry.trim();
            if trimmed.is_empty() {
                return None;
            }
            let (local, exported) = if let Some((left, right)) = split_keyword_alias(trimmed, "as") {
                (left.trim().to_owned(), right.trim().to_owned())
            } else {
                (trimmed.to_owned(), trimmed.to_owned())
            };
            Some(NamedExportSpecifier { local, exported })
        })
        .collect()
    }

    fn parse_local_statement(&self, text: &str, keyword: LocalKeyword) -> StatementNode {
        let fragment = Fragment::new(self.source, text);
        let first = fragment.first_significant_index().unwrap_or(0);
        let next = fragment.next_significant_after(first).unwrap_or(fragment.tokens.len());

        let (bindings, value) = if let Some(assign_index) = fragment.find_top_level_symbol(Symbol::Assign)
        {
            (
                fragment.slice_between_tokens(next, assign_index).trim().to_owned(),
                Some(
                    fragment
                        .slice_between_tokens(assign_index + 1, fragment.tokens.len() - 1)
                        .trim()
                        .to_owned(),
                ),
            )
        } else {
            (
                fragment
                    .slice_between_tokens(next, fragment.tokens.len() - 1)
                    .trim()
                    .to_owned(),
                None,
            )
        };

        StatementNode::Local(LocalStatement {
            keyword,
            bindings,
            value,
        })
    }

    fn parse_return_statement(&self, text: &str) -> StatementNode {
        let fragment = Fragment::new(self.source, text);
        let first = fragment.first_significant_index().unwrap_or(0);
        let values = fragment
            .slice_between_tokens(first + 1, fragment.tokens.len() - 1)
            .trim()
            .to_owned();

        StatementNode::Return(ReturnStatement {
            values: if values.is_empty() { None } else { Some(values) },
        })
    }

    fn parse_if_statement(&self, text: &str) -> StatementNode {
        let fragment = Fragment::new(self.source, text);
        let tokens = fragment.significant_tokens();
        let mut clauses = Vec::new();
        let mut else_body = None;
        let mut cursor = 0usize;

        while cursor < tokens.len() {
            match tokens[cursor].kind {
                TokenKind::Keyword(Keyword::If) | TokenKind::Keyword(Keyword::ElseIf) => {
                    let keyword = if matches!(tokens[cursor].kind, TokenKind::Keyword(Keyword::If)) {
                        ConditionalKeyword::If
                    } else {
                        ConditionalKeyword::ElseIf
                    };
                    let then_index = fragment
                        .find_top_level_keyword_between(&tokens, cursor + 1, Keyword::Then)
                        .expect("if clause should contain then");
                    let cond = fragment
                        .text_between(tokens[cursor].span.end, tokens[then_index].span.start)
                        .trim()
                        .to_owned();
                    let body_end = fragment
                        .find_next_clause_boundary(&tokens, then_index + 1)
                        .unwrap_or(tokens.len() - 1);
                    let body_text = fragment
                        .text_between(tokens[then_index].span.end, tokens[body_end].span.start);
                    clauses.push(ConditionalClause {
                        keyword,
                        condition: cond,
                        body: self.parse_nested_statements(body_text),
                    });
                    cursor = body_end;
                }
                TokenKind::Keyword(Keyword::Else) => {
                    let body_end = tokens.len() - 1;
                    let body_text =
                        fragment.text_between(tokens[cursor].span.end, tokens[body_end].span.start);
                    else_body = Some(self.parse_nested_statements(body_text));
                    cursor = body_end;
                }
                TokenKind::Keyword(Keyword::End) => break,
                _ => cursor += 1,
            }
        }

        StatementNode::If(IfStatement { clauses, else_body })
    }

    fn parse_while_statement(&self, text: &str) -> StatementNode {
        let fragment = Fragment::new(self.source, text);
        let tokens = fragment.significant_tokens();
        let do_index = fragment
            .find_top_level_keyword_between(&tokens, 1, Keyword::Do)
            .expect("while should contain do");
        let condition = fragment
            .text_between(tokens[0].span.end, tokens[do_index].span.start)
            .trim()
            .to_owned();
        let body_text = fragment.text_between(
            tokens[do_index].span.end,
            tokens[tokens.len() - 1].span.start,
        );

        StatementNode::While(WhileStatement {
            condition,
            body: self.parse_nested_statements(body_text),
        })
    }

    fn parse_repeat_statement(&self, text: &str) -> StatementNode {
        let fragment = Fragment::new(self.source, text);
        let tokens = fragment.significant_tokens();
        let until_index = tokens
            .iter()
            .position(|token| token.kind == TokenKind::Keyword(Keyword::Until))
            .expect("repeat should contain until");
        let body_text = fragment.text_between(tokens[0].span.end, tokens[until_index].span.start);
        let condition = fragment
            .text_between(tokens[until_index].span.end, text.len())
            .trim()
            .to_owned();

        StatementNode::Repeat(RepeatStatement {
            body: self.parse_nested_statements(body_text),
            condition,
        })
    }

    fn parse_for_statement(&self, text: &str) -> StatementNode {
        let fragment = Fragment::new(self.source, text);
        let tokens = fragment.significant_tokens();
        let do_index = fragment
            .find_top_level_keyword_between(&tokens, 1, Keyword::Do)
            .expect("for should contain do");
        let head = fragment
            .text_between(tokens[0].span.end, tokens[do_index].span.start)
            .trim()
            .to_owned();
        let body_text = fragment.text_between(
            tokens[do_index].span.end,
            tokens[tokens.len() - 1].span.start,
        );

        StatementNode::For(ForStatement {
            head,
            body: self.parse_nested_statements(body_text),
        })
    }

    fn parse_function_statement(&self, text: &str) -> StatementNode {
        let fragment = Fragment::new(self.source, text);
        let tokens = fragment.significant_tokens();
        let open_paren_index = tokens
            .iter()
            .position(|token| token.kind == TokenKind::Symbol(Symbol::LeftParen))
            .expect("function should contain opening paren");
        let close_paren_index = fragment.matching_paren(&tokens, open_paren_index);
        let end_index = tokens.len() - 1;
        let params = fragment
            .text_between(
                tokens[open_paren_index].span.end,
                tokens[close_paren_index].span.start,
            )
            .trim()
            .to_owned();
        let mut body_start = tokens[close_paren_index].span.end;
        if let Some(newline) = fragment.tokens.iter().find(|token| {
            matches!(token.kind, TokenKind::Newline)
                && token.span.start >= tokens[close_paren_index].span.end
        }) {
            body_start = newline.span.end;
        }

        let body_text = fragment.text_between(body_start, tokens[end_index].span.start);
        let header_prefix = fragment.text_between(0, tokens[open_paren_index].span.end);
        let header_suffix =
            fragment.text_between(tokens[close_paren_index].span.start, body_start);

        StatementNode::Function(FunctionStatement {
            header_prefix,
            params,
            header_suffix,
            body: self.parse_nested_statements(body_text),
        })
    }

    fn parse_do_statement(&self, text: &str) -> StatementNode {
        let fragment = Fragment::new(self.source, text);
        let tokens = fragment.significant_tokens();
        let body_text =
            fragment.text_between(tokens[0].span.end, tokens[tokens.len() - 1].span.start);

        StatementNode::Do(BlockStatement {
            body: self.parse_nested_statements(body_text),
        })
    }

    fn parse_switch_statement(&self, text: &str) -> StatementNode {
        let fragment = Fragment::new(self.source, text);
        let tokens = fragment.significant_tokens();
        let expression = fragment
            .text_between(
                tokens[0].span.end,
                fragment
                    .first_body_newline(&tokens)
                    .unwrap_or(tokens[1].span.start),
            )
            .trim()
            .to_owned();
        let mut sections = Vec::new();

        for section in fragment.switch_sections(&tokens) {
            let label = match section.label {
                FragmentSwitchLabel::Case(values) => SwitchLabel::Case(values),
                FragmentSwitchLabel::Default => SwitchLabel::Default,
            };
            sections.push(SwitchSection {
                label,
                body: self.parse_nested_statements(section.body),
            });
        }

        StatementNode::Switch(SwitchStatement { expression, sections })
    }

    fn parse_nested_statements(&self, text: String) -> Vec<Statement> {
        if text.trim().is_empty() {
            return Vec::new();
        }

        let nested_source =
            SourceFile::virtual_file(self.source.path.clone(), self.source.kind, text);
        let nested_tokens = Lexer::new(&nested_source).lex(&mut Vec::new());
        Parser::new(&nested_source, &nested_tokens)
            .parse(&mut Vec::new())
            .statements
    }
}

fn split_statement_suffix(tokens: &[Token], text: &str) -> (String, String) {
    if let Some(last) = tokens.last() {
        match last.kind {
            TokenKind::Newline | TokenKind::Symbol(Symbol::Semicolon) => {
                let split_at = text.len().saturating_sub(last.lexeme.len());
                return (text[..split_at].to_owned(), text[split_at..].to_owned());
            }
            _ => {}
        }
    }

    (text.to_owned(), String::new())
}

fn token_text(token: &Token) -> &str {
    token.lexeme.as_str()
}

fn token_text_owned(token: &Token) -> String {
    token_text(token).to_owned()
}

fn unquote_string(text: &str) -> String {
    if text.len() >= 2 {
        let bytes = text.as_bytes();
        let first = bytes[0] as char;
        let last = bytes[text.len() - 1] as char;
        if matches!(first, '"' | '\'' | '`') && first == last {
            return text[1..text.len() - 1].to_owned();
        }
    }

    text.to_owned()
}

fn split_keyword_alias(text: &str, keyword: &str) -> Option<(String, String)> {
    let needle = format!(" {keyword} ");
    text.find(&needle).map(|index| {
        (
            text[..index].to_owned(),
            text[index + needle.len()..].to_owned(),
        )
    })
}

fn split_top_level(text: &str, separator: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut paren = 0usize;
    let mut brace = 0usize;
    let mut bracket = 0usize;

    for ch in text.chars() {
        match ch {
            '(' => paren += 1,
            ')' => paren = paren.saturating_sub(1),
            '{' => brace += 1,
            '}' => brace = brace.saturating_sub(1),
            '[' => bracket += 1,
            ']' => bracket = bracket.saturating_sub(1),
            _ => {}
        }

        if ch == separator && paren == 0 && brace == 0 && bracket == 0 {
            parts.push(current.trim().to_owned());
            current.clear();
        } else {
            current.push(ch);
        }
    }

    if !current.trim().is_empty() {
        parts.push(current.trim().to_owned());
    }

    parts
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
        TokenKind::Keyword(Keyword::Abstract) if significant.get(1).map(|token| token.kind.clone()) == Some(TokenKind::Keyword(Keyword::Class)) => StatementKind::XLuauDeclaration,
        TokenKind::Keyword(Keyword::Class)
        | TokenKind::Keyword(Keyword::Interface)
        | TokenKind::Keyword(Keyword::Const)
        | TokenKind::Keyword(Keyword::Enum)
        | TokenKind::Keyword(Keyword::Let)
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
        Symbol::Ampersand => "&",
        Symbol::PipeGreater => "|>",
        Symbol::FatArrow => "=>",
        Symbol::DotDot => "..",
        Symbol::DotDotDot => "...",
        Symbol::At => "@",
    }
}

#[derive(Debug)]
struct Fragment {
    text: String,
    tokens: Vec<Token>,
}

impl Fragment {
    fn new(source: &SourceFile, text: &str) -> Self {
        let fragment_source = SourceFile::virtual_file(source.path.clone(), source.kind, text.to_owned());
        let tokens = Lexer::new(&fragment_source).lex(&mut Vec::new());
        Self {
            text: text.to_owned(),
            tokens,
        }
    }

    fn significant_tokens(&self) -> Vec<Token> {
        self.tokens
            .iter()
            .filter(|token| !token.is_trivia() && token.kind != TokenKind::Eof)
            .cloned()
            .collect()
    }

    fn first_significant_index(&self) -> Option<usize> {
        self.tokens
            .iter()
            .position(|token| !token.is_trivia() && token.kind != TokenKind::Eof)
    }

    fn next_significant_after(&self, index: usize) -> Option<usize> {
        self.tokens
            .iter()
            .enumerate()
            .skip(index + 1)
            .find(|(_, token)| !token.is_trivia() && token.kind != TokenKind::Eof)
            .map(|(index, _)| index)
    }

    fn slice_between_tokens(&self, start: usize, end_exclusive: usize) -> String {
        let Some(first) = self.tokens.get(start) else {
            return String::new();
        };
        if end_exclusive <= start {
            return String::new();
        }
        let last = &self.tokens[end_exclusive - 1];
        self.text[first.span.start..last.span.end].to_owned()
    }

    fn text_between(&self, start: usize, end: usize) -> String {
        if start >= end || end > self.text.len() {
            return String::new();
        }
        self.text[start..end].to_owned()
    }

    fn find_top_level_symbol(&self, symbol: Symbol) -> Option<usize> {
        let mut paren = 0usize;
        let mut brace = 0usize;
        let mut bracket = 0usize;
        for (index, token) in self.tokens.iter().enumerate() {
            match token.kind {
                TokenKind::Symbol(Symbol::LeftParen) => paren += 1,
                TokenKind::Symbol(Symbol::RightParen) => paren = paren.saturating_sub(1),
                TokenKind::Symbol(Symbol::LeftBrace) => brace += 1,
                TokenKind::Symbol(Symbol::RightBrace) => brace = brace.saturating_sub(1),
                TokenKind::Symbol(Symbol::LeftBracket) => bracket += 1,
                TokenKind::Symbol(Symbol::RightBracket) => bracket = bracket.saturating_sub(1),
                TokenKind::Symbol(found)
                    if found == symbol && paren == 0 && brace == 0 && bracket == 0 =>
                {
                    return Some(index);
                }
                _ => {}
            }
        }
        None
    }

    fn find_top_level_keyword_between(
        &self,
        tokens: &[Token],
        start: usize,
        keyword: Keyword,
    ) -> Option<usize> {
        let mut paren = 0usize;
        let mut brace = 0usize;
        let mut bracket = 0usize;
        let mut blocks = 0usize;

        for (index, token) in tokens.iter().enumerate().skip(start) {
            match token.kind {
                TokenKind::Symbol(Symbol::LeftParen) => paren += 1,
                TokenKind::Symbol(Symbol::RightParen) => paren = paren.saturating_sub(1),
                TokenKind::Symbol(Symbol::LeftBrace) => brace += 1,
                TokenKind::Symbol(Symbol::RightBrace) => brace = brace.saturating_sub(1),
                TokenKind::Symbol(Symbol::LeftBracket) => bracket += 1,
                TokenKind::Symbol(Symbol::RightBracket) => bracket = bracket.saturating_sub(1),
                TokenKind::Keyword(Keyword::Function)
                | TokenKind::Keyword(Keyword::If)
                | TokenKind::Keyword(Keyword::For)
                | TokenKind::Keyword(Keyword::While)
                | TokenKind::Keyword(Keyword::Repeat)
                | TokenKind::Keyword(Keyword::Switch) => blocks += 1,
                TokenKind::Keyword(Keyword::End) | TokenKind::Keyword(Keyword::Until) => {
                    blocks = blocks.saturating_sub(1)
                }
                TokenKind::Keyword(found)
                    if found == keyword
                        && paren == 0
                        && brace == 0
                        && bracket == 0
                        && blocks == 0 =>
                {
                    return Some(index);
                }
                _ => {}
            }
        }

        None
    }

    fn find_next_clause_boundary(&self, tokens: &[Token], start: usize) -> Option<usize> {
        let mut depth = 0usize;
        for (index, token) in tokens.iter().enumerate().skip(start) {
            match token.kind {
                TokenKind::Keyword(Keyword::If)
                | TokenKind::Keyword(Keyword::Function)
                | TokenKind::Keyword(Keyword::For)
                | TokenKind::Keyword(Keyword::While)
                | TokenKind::Keyword(Keyword::Repeat)
                | TokenKind::Keyword(Keyword::Switch) => depth += 1,
                TokenKind::Keyword(Keyword::End) | TokenKind::Keyword(Keyword::Until) => {
                    if depth == 0 {
                        return Some(index);
                    }
                    depth -= 1;
                }
                TokenKind::Keyword(Keyword::ElseIf) | TokenKind::Keyword(Keyword::Else)
                    if depth == 0 =>
                {
                    return Some(index);
                }
                _ => {}
            }
        }
        None
    }

    fn matching_paren(&self, tokens: &[Token], start: usize) -> usize {
        self.matching_delimiter(tokens, start, Symbol::LeftParen, Symbol::RightParen)
    }

    fn matching_delimiter(
        &self,
        tokens: &[Token],
        start: usize,
        open: Symbol,
        close: Symbol,
    ) -> usize {
        let mut depth = 0usize;
        for (index, token) in tokens.iter().enumerate().skip(start) {
            match token.kind {
                TokenKind::Symbol(symbol) if symbol == open => depth += 1,
                TokenKind::Symbol(symbol) if symbol == close => {
                    depth -= 1;
                    if depth == 0 {
                        return index;
                    }
                }
                _ => {}
            }
        }
        start
    }

    fn first_body_newline(&self, tokens: &[Token]) -> Option<usize> {
        self.tokens
            .iter()
            .find(|token| {
                matches!(token.kind, TokenKind::Newline) && token.span.start >= tokens[0].span.end
            })
            .map(|token| token.span.start)
    }

    fn switch_sections(&self, tokens: &[Token]) -> Vec<FragmentSwitchSection> {
        let mut sections = Vec::new();
        let body_start = self
            .first_body_newline(tokens)
            .unwrap_or(tokens[1].span.start);
        let body_tokens = tokens
            .iter()
            .filter(|token| token.span.start >= body_start)
            .cloned()
            .collect::<Vec<_>>();
        let mut cursor = 0usize;

        while cursor < body_tokens.len() {
            match body_tokens[cursor].kind {
                TokenKind::Keyword(Keyword::Case) => {
                    let label_end = self
                        .find_case_colon(&body_tokens, cursor + 1)
                        .unwrap_or(cursor + 1);
                    let body_end = self
                        .find_next_switch_boundary(&body_tokens, label_end + 1)
                        .unwrap_or(body_tokens.len() - 1);
                    let labels = self
                        .text_between(
                            body_tokens[cursor].span.end,
                            body_tokens[label_end].span.start,
                        )
                        .split(',')
                        .map(|expr| expr.trim().to_owned())
                        .filter(|expr| !expr.is_empty())
                        .collect::<Vec<_>>();
                    let body = self.text_between(
                        body_tokens[label_end].span.end,
                        body_tokens[body_end].span.start,
                    );
                    sections.push(FragmentSwitchSection {
                        label: FragmentSwitchLabel::Case(labels),
                        body,
                    });
                    cursor = body_end;
                }
                TokenKind::Keyword(Keyword::Default) => {
                    let label_end = self
                        .find_case_colon(&body_tokens, cursor + 1)
                        .unwrap_or(cursor);
                    let body_end = self
                        .find_next_switch_boundary(&body_tokens, label_end + 1)
                        .unwrap_or(body_tokens.len() - 1);
                    let body = self.text_between(
                        body_tokens[label_end].span.end,
                        body_tokens[body_end].span.start,
                    );
                    sections.push(FragmentSwitchSection {
                        label: FragmentSwitchLabel::Default,
                        body,
                    });
                    cursor = body_end;
                }
                TokenKind::Keyword(Keyword::End) => break,
                _ => cursor += 1,
            }
        }

        sections
    }

    fn find_case_colon(&self, tokens: &[Token], start: usize) -> Option<usize> {
        tokens
            .iter()
            .enumerate()
            .skip(start)
            .find(|(_, token)| token.kind == TokenKind::Symbol(Symbol::Colon))
            .map(|(index, _)| index)
    }

    fn find_next_switch_boundary(&self, tokens: &[Token], start: usize) -> Option<usize> {
        let mut depth = 0usize;
        for (index, token) in tokens.iter().enumerate().skip(start) {
            match token.kind {
                TokenKind::Keyword(Keyword::If)
                | TokenKind::Keyword(Keyword::Function)
                | TokenKind::Keyword(Keyword::For)
                | TokenKind::Keyword(Keyword::While)
                | TokenKind::Keyword(Keyword::Repeat)
                | TokenKind::Keyword(Keyword::Switch) => depth += 1,
                TokenKind::Keyword(Keyword::End) | TokenKind::Keyword(Keyword::Until) => {
                    if depth == 0 {
                        return Some(index);
                    }
                    depth -= 1;
                }
                TokenKind::Keyword(Keyword::Case) | TokenKind::Keyword(Keyword::Default)
                    if depth == 0 =>
                {
                    return Some(index);
                }
                _ => {}
            }
        }
        None
    }
}

#[derive(Debug)]
struct FragmentSwitchSection {
    label: FragmentSwitchLabel,
    body: String,
}

#[derive(Debug)]
enum FragmentSwitchLabel {
    Case(Vec<String>),
    Default,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::Parser;
    use crate::ast::{ExportKind, ImportKind, StatementKind, StatementNode, SwitchLabel};
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
    fn parser_builds_recursive_function_body() {
        let source = SourceFile {
            path: PathBuf::from("test.xl"),
            kind: SourceKind::XLuau,
            text: "function demo()\n    local value = 1\n    return value\nend\nprint(demo())"
                .to_owned(),
        };
        let tokens = Lexer::new(&source).lex(&mut Vec::new());
        let program = Parser::new(&source, &tokens).parse(&mut Vec::new());

        assert_eq!(program.statements.len(), 2);
        match &program.statements[0].node {
            StatementNode::Function(function) => {
                assert_eq!(function.body.len(), 2);
                assert!(matches!(function.body[0].node, StatementNode::Local(_)));
                assert!(matches!(function.body[1].node, StatementNode::Return(_)));
            }
            other => panic!("expected function node, found {other:?}"),
        }
    }

    #[test]
    fn parser_keeps_statement_after_generic_function() {
        let source = SourceFile {
            path: PathBuf::from("test.xl"),
            kind: SourceKind::XLuau,
            text: "function wrap<T extends string, U = {T}>(value: T): U\n    return value\nend\nlocal boxed = wrap<number?>(nil)\n"
                .to_owned(),
        };
        let tokens = Lexer::new(&source).lex(&mut Vec::new());
        let program = Parser::new(&source, &tokens).parse(&mut Vec::new());

        assert_eq!(program.statements.len(), 2);
        assert!(matches!(program.statements[0].node, StatementNode::Function(_)));
        assert!(matches!(program.statements[1].node, StatementNode::Local(_)));
    }

    #[test]
    fn parser_keeps_statement_after_transformed_typed_call() {
        let source = SourceFile {
            path: PathBuf::from("test.luau"),
            kind: SourceKind::Luau,
            text: "function wrap(value: (T & string)): U\n    return value :: any\nend\nlocal boxed = wrap((nil :: number?))\n"
                .to_owned(),
        };
        let tokens = Lexer::new(&source).lex(&mut Vec::new());
        let program = Parser::new(&source, &tokens).parse(&mut Vec::new());

        assert_eq!(program.statements.len(), 2);
        assert!(matches!(program.statements[0].node, StatementNode::Function(_)));
        assert!(matches!(program.statements[1].node, StatementNode::Local(_)));
    }

    #[test]
    fn parser_builds_switch_sections() {
        let source = SourceFile {
            path: PathBuf::from("test.xl"),
            kind: SourceKind::XLuau,
            text: "switch value\ncase 1, 2:\n    print(\"hit\")\ndefault:\n    print(\"miss\")\nend\n"
                .to_owned(),
        };
        let tokens = Lexer::new(&source).lex(&mut Vec::new());
        let program = Parser::new(&source, &tokens).parse(&mut Vec::new());

        match &program.statements[0].node {
            StatementNode::Switch(switch) => {
                assert_eq!(switch.sections.len(), 2);
                assert!(matches!(&switch.sections[0].label, SwitchLabel::Case(values) if values == &vec!["1".to_owned(), "2".to_owned()]));
                assert!(matches!(switch.sections[1].label, SwitchLabel::Default));
            }
            other => panic!("expected switch node, found {other:?}"),
        }
    }

    #[test]
    fn parser_builds_import_and_export_nodes() {
        let source = SourceFile {
            path: PathBuf::from("test.xl"),
            kind: SourceKind::XLuau,
            text: "import React, { useState as state } from \"./react\"\nexport { React as default, state }\n".to_owned(),
        };
        let tokens = Lexer::new(&source).lex(&mut Vec::new());
        let program = Parser::new(&source, &tokens).parse(&mut Vec::new());

        match &program.statements[0].node {
            StatementNode::Import(import) => match &import.kind {
                ImportKind::Value {
                    default,
                    namespace,
                    named,
                } => {
                    assert_eq!(default.as_deref(), Some("React"));
                    assert!(namespace.is_none());
                    assert_eq!(named[0].imported, "useState");
                    assert_eq!(named[0].local, "state");
                }
                other => panic!("expected value import, found {other:?}"),
            },
            other => panic!("expected import node, found {other:?}"),
        }

        match &program.statements[1].node {
            StatementNode::Export(export) => match &export.kind {
                ExportKind::Named {
                    specifiers,
                    source,
                    is_type_only,
                } => {
                    assert!(!is_type_only);
                    assert!(source.is_none());
                    assert_eq!(specifiers[0].local, "React");
                    assert_eq!(specifiers[0].exported, "default");
                    assert_eq!(specifiers[1].local, "state");
                    assert_eq!(specifiers[1].exported, "state");
                }
                other => panic!("expected named export, found {other:?}"),
            },
            other => panic!("expected export node, found {other:?}"),
        }
    }

    #[test]
    fn parser_separates_interface_and_abstract_class_blocks() {
        let source = SourceFile {
            path: PathBuf::from("test.xl"),
            kind: SourceKind::XLuau,
            text: "interface Serializable {\n    serialize: (self: Serializable) -> string\n}\nabstract class Base {\n    abstract function serialize(): string\n}\nclass Broken extends Base {\n    function mutate()\n        return nil\n    end\n}\n"
                .to_owned(),
        };
        let tokens = Lexer::new(&source).lex(&mut Vec::new());
        let program = Parser::new(&source, &tokens).parse(&mut Vec::new());

        assert_eq!(program.statements.len(), 3);
        assert!(matches!(program.statements[0].node, StatementNode::Text(_)));
        assert!(matches!(program.statements[1].node, StatementNode::Text(_)));
        assert!(matches!(program.statements[2].node, StatementNode::Text(_)));
    }
}
