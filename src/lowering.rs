use std::collections::HashSet;
use std::fmt::Write;
use std::path::PathBuf;

use crate::ast::Program;
use crate::diagnostic::{Diagnostic, Span};
use crate::lexer::{Keyword, Lexer, Symbol, Token, TokenKind};
use crate::source::{SourceFile, SourceKind};

#[derive(Debug, Default)]
pub struct Lowerer {
    next_temp_id: usize,
    scopes: Vec<ScopeFrame>,
}

#[derive(Debug, Default)]
struct ScopeFrame {
    locals: HashSet<String>,
    consts: HashSet<String>,
}

impl Lowerer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn lower_program(
        &mut self,
        source: &SourceFile,
        program: &Program,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        self.push_scope();
        let mut output = String::new();
        for statement in &program.statements {
            output.push_str(&self.lower_statement(
                source.path.clone(),
                statement.raw_text.as_str(),
                diagnostics,
            ));
        }
        self.pop_scope();
        output
    }

    fn lower_statement(
        &mut self,
        path: PathBuf,
        text: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let fragment = Fragment::new(path.clone(), text);
        let Some(first) = fragment.first_significant_index() else {
            return text.to_owned();
        };

        match fragment.tokens[first].kind {
            TokenKind::Keyword(Keyword::Const) => {
                self.lower_const_statement(&fragment, first, diagnostics)
            }
            TokenKind::Keyword(Keyword::Local) | TokenKind::Keyword(Keyword::Let) => {
                self.lower_local_statement(&fragment, first, diagnostics)
            }
            TokenKind::Keyword(Keyword::Return) => self.lower_return_statement(&fragment, first),
            TokenKind::Keyword(Keyword::If) => self.lower_if_statement(&path, text, diagnostics),
            TokenKind::Keyword(Keyword::While) => {
                self.lower_while_statement(&path, text, diagnostics)
            }
            TokenKind::Keyword(Keyword::Repeat) => {
                self.lower_repeat_statement(&path, text, diagnostics)
            }
            TokenKind::Keyword(Keyword::For) => self.lower_for_statement(&path, text, diagnostics),
            TokenKind::Keyword(Keyword::Function) => {
                self.lower_function_statement(&path, text, diagnostics)
            }
            TokenKind::Keyword(Keyword::Do) => self.lower_do_statement(&path, text, diagnostics),
            TokenKind::Keyword(Keyword::Switch) => {
                self.lower_switch_statement(&path, text, diagnostics)
            }
            _ => self.lower_assignment_or_expression(&fragment, diagnostics),
        }
    }

    fn lower_nested_block(
        &mut self,
        path: PathBuf,
        text: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let source = SourceFile::virtual_file(path, SourceKind::XLuau, text.to_owned());
        let tokens = Lexer::new(&source).lex(diagnostics);
        let program = crate::parser::Parser::new(&source, &tokens).parse(diagnostics);
        self.lower_program(&source, &program, diagnostics)
    }

    fn lower_const_statement(
        &mut self,
        fragment: &Fragment,
        const_index: usize,
        _diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let Some(name_start) = fragment.next_significant_after(const_index) else {
            return fragment.text.clone();
        };
        let Some(assign_index) = fragment.find_top_level_symbol(Symbol::Assign) else {
            return fragment.text.replacen("const", "local", 1);
        };

        let names = fragment
            .top_level_identifier_list(name_start, assign_index)
            .unwrap_or_default();
        for name in &names {
            self.declare_const(name);
        }

        let rhs = fragment.slice_between_tokens(assign_index + 1, fragment.tokens.len() - 1);
        let lowered_rhs = self.lower_expression_list(rhs.trim());
        format!(
            "local {} = {}{}",
            names.join(", "),
            lowered_rhs,
            fragment.trailing_newline()
        )
    }

    fn lower_local_statement(
        &mut self,
        fragment: &Fragment,
        local_index: usize,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let Some(next_index) = fragment.next_significant_after(local_index) else {
            return fragment.text.clone();
        };

        match fragment.tokens[next_index].kind {
            TokenKind::Keyword(Keyword::Function) => {
                if let Some(function_name_index) = fragment.next_significant_after(next_index) {
                    if let TokenKind::Identifier = fragment.tokens[function_name_index].kind {
                        self.declare_local(fragment.tokens[function_name_index].lexeme.as_str());
                    }
                }
                self.lower_function_statement(&fragment.path, fragment.text.as_str(), diagnostics)
            }
            TokenKind::Symbol(Symbol::LeftBrace) | TokenKind::Symbol(Symbol::LeftBracket) => {
                self.lower_destructuring_local(fragment, next_index, diagnostics)
            }
            _ => {
                let names =
                    if let Some(assign_index) = fragment.find_top_level_symbol(Symbol::Assign) {
                        fragment
                            .top_level_identifier_list(local_index + 1, assign_index)
                            .unwrap_or_default()
                    } else {
                        fragment
                            .top_level_identifier_list(local_index + 1, fragment.tokens.len() - 1)
                            .unwrap_or_default()
                    };
                for name in &names {
                    self.declare_local(name);
                }

                if let Some(assign_index) = fragment.find_top_level_symbol(Symbol::Assign) {
                    let lhs = fragment.slice_between_tokens(local_index + 1, assign_index);
                    let rhs =
                        fragment.slice_between_tokens(assign_index + 1, fragment.tokens.len() - 1);
                    format!(
                        "local {} = {}{}",
                        lhs.trim(),
                        self.lower_expression_list(rhs.trim()),
                        fragment.trailing_newline()
                    )
                } else {
                    format!("local {}{}", names.join(", "), fragment.trailing_newline())
                }
            }
        }
    }

    fn lower_return_statement(&mut self, fragment: &Fragment, return_index: usize) -> String {
        let exprs = fragment.slice_between_tokens(return_index + 1, fragment.tokens.len() - 1);
        if exprs.trim().is_empty() {
            return fragment.text.clone();
        }

        format!(
            "return {}{}",
            self.lower_expression_list(exprs.trim()),
            fragment.trailing_newline()
        )
    }

    fn lower_assignment_or_expression(
        &mut self,
        fragment: &Fragment,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        if let Some(index) = fragment.find_top_level_symbol(Symbol::QuestionQuestionEqual) {
            let lhs = fragment.slice_between_tokens(0, index);
            let rhs = fragment.slice_between_tokens(index + 1, fragment.tokens.len() - 1);
            return self.lower_nullish_assignment(
                lhs.trim(),
                rhs.trim(),
                fragment.trailing_newline(),
            );
        }

        if let Some(index) = fragment.find_top_level_symbol(Symbol::Assign) {
            let lhs = fragment.slice_between_tokens(0, index).trim().to_owned();
            self.check_const_reassignment(fragment, lhs.as_str(), diagnostics);
            let rhs = fragment.slice_between_tokens(index + 1, fragment.tokens.len() - 1);
            return format!(
                "{} = {}{}",
                lhs,
                self.lower_expression_list(rhs.trim()),
                fragment.trailing_newline()
            );
        }

        format!(
            "{}{}",
            self.lower_expression(fragment.text.trim_end()),
            fragment.trailing_newline()
        )
    }

    fn lower_if_statement(
        &mut self,
        path: &PathBuf,
        text: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let fragment = Fragment::new(path.clone(), text);
        let tokens = fragment.significant_tokens();
        let mut cursor = 0usize;
        let mut output = String::new();

        while cursor < tokens.len() {
            match tokens[cursor].kind {
                TokenKind::Keyword(Keyword::If) | TokenKind::Keyword(Keyword::ElseIf) => {
                    let keyword = fragment.token_text(&tokens[cursor]);
                    let then_index = fragment
                        .find_top_level_keyword_between(&tokens, cursor + 1, Keyword::Then)
                        .expect("if clause should contain then");
                    let cond = fragment
                        .text_between(tokens[cursor].span.end, tokens[then_index].span.start);
                    let body_end = fragment
                        .find_next_clause_boundary(&tokens, then_index + 1)
                        .unwrap_or(tokens.len() - 1);
                    let body = fragment
                        .text_between(tokens[then_index].span.end, tokens[body_end].span.start);
                    let lowered_body = self.lower_nested_block(path.clone(), &body, diagnostics);

                    writeln!(
                        output,
                        "{} {} then",
                        keyword.trim(),
                        self.lower_expression(cond.trim())
                    )
                    .ok();
                    output.push_str(lowered_body.as_str());
                    cursor = body_end;
                }
                TokenKind::Keyword(Keyword::Else) => {
                    let body_end = tokens.len() - 1;
                    let body =
                        fragment.text_between(tokens[cursor].span.end, tokens[body_end].span.start);
                    let lowered_body = self.lower_nested_block(path.clone(), &body, diagnostics);
                    output.push_str("else\n");
                    output.push_str(lowered_body.as_str());
                    cursor = body_end;
                }
                TokenKind::Keyword(Keyword::End) => {
                    output.push_str("end");
                    output.push_str(fragment.trailing_newline().as_str());
                    break;
                }
                _ => cursor += 1,
            }
        }

        output
    }

    fn lower_while_statement(
        &mut self,
        path: &PathBuf,
        text: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let fragment = Fragment::new(path.clone(), text);
        let tokens = fragment.significant_tokens();
        let do_index = fragment
            .find_top_level_keyword_between(&tokens, 1, Keyword::Do)
            .expect("while should contain do");
        let cond = fragment.text_between(tokens[0].span.end, tokens[do_index].span.start);
        let end_index = tokens.len() - 1;
        let body = fragment.text_between(tokens[do_index].span.end, tokens[end_index].span.start);
        let lowered_body = self.lower_nested_block(path.clone(), &body, diagnostics);

        format!(
            "while {} do\n{}end{}",
            self.lower_expression(cond.trim()),
            lowered_body,
            fragment.trailing_newline()
        )
    }

    fn lower_repeat_statement(
        &mut self,
        path: &PathBuf,
        text: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let fragment = Fragment::new(path.clone(), text);
        let tokens = fragment.significant_tokens();
        let until_index = tokens
            .iter()
            .position(|token| token.kind == TokenKind::Keyword(Keyword::Until))
            .expect("repeat should contain until");
        let body = fragment.text_between(tokens[0].span.end, tokens[until_index].span.start);
        let cond = fragment.text_between(tokens[until_index].span.end, fragment.text.len());
        let lowered_body = self.lower_nested_block(path.clone(), &body, diagnostics);

        format!(
            "repeat\n{}until {}{}",
            lowered_body,
            self.lower_expression(cond.trim()),
            fragment.trailing_newline()
        )
    }

    fn lower_do_statement(
        &mut self,
        path: &PathBuf,
        text: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let fragment = Fragment::new(path.clone(), text);
        let tokens = fragment.significant_tokens();
        let end_index = tokens.len() - 1;
        let body = fragment.text_between(tokens[0].span.end, tokens[end_index].span.start);
        let lowered_body = self.lower_nested_block(path.clone(), &body, diagnostics);
        format!("do\n{}end{}", lowered_body, fragment.trailing_newline())
    }

    fn lower_for_statement(
        &mut self,
        path: &PathBuf,
        text: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let fragment = Fragment::new(path.clone(), text);
        let tokens = fragment.significant_tokens();
        let do_index = fragment
            .find_top_level_keyword_between(&tokens, 1, Keyword::Do)
            .expect("for should contain do");
        let head = fragment
            .text_between(tokens[0].span.end, tokens[do_index].span.start)
            .trim()
            .to_owned();
        let body = fragment.text_between(
            tokens[do_index].span.end,
            tokens[tokens.len() - 1].span.start,
        );
        let lowered_body = self.lower_nested_block(path.clone(), &body, diagnostics);

        if head.contains(" in ") {
            self.lower_generic_for(&head, lowered_body, fragment.trailing_newline())
        } else {
            let numeric = self.lower_numeric_for_head(&head);
            format!(
                "for {} do\n{}end{}",
                numeric,
                lowered_body,
                fragment.trailing_newline()
            )
        }
    }

    fn lower_function_statement(
        &mut self,
        path: &PathBuf,
        text: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let fragment = Fragment::new(path.clone(), text);
        let tokens = fragment.significant_tokens();
        let open_paren_index = tokens
            .iter()
            .position(|token| token.kind == TokenKind::Symbol(Symbol::LeftParen))
            .expect("function should contain opening paren");
        let close_paren_index = fragment.matching_paren(&tokens, open_paren_index);
        let end_index = tokens.len() - 1;

        let header_start = 0usize;
        let params = fragment.text_between(
            tokens[open_paren_index].span.end,
            tokens[close_paren_index].span.start,
        );
        let mut body_start = tokens[close_paren_index].span.end;
        if let Some(newline) = fragment.tokens.iter().find(|token| {
            matches!(token.kind, TokenKind::Newline)
                && token.span.start >= tokens[close_paren_index].span.end
        }) {
            body_start = newline.span.end;
        }

        let body = fragment.text_between(body_start, tokens[end_index].span.start);
        let (lowered_params, prologue) = self.lower_function_parameters(params.trim());
        let lowered_body = self.lower_nested_block(path.clone(), &body, diagnostics);
        let header_prefix = fragment.text_between(header_start, tokens[open_paren_index].span.end);
        let header_suffix = fragment.text_between(tokens[close_paren_index].span.start, body_start);

        let mut output = String::new();
        output.push_str(header_prefix.as_str());
        output.push_str(lowered_params.as_str());
        output.push_str(header_suffix.as_str());
        if !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(prologue.as_str());
        output.push_str(lowered_body.as_str());
        output.push_str("end");
        output.push_str(fragment.trailing_newline().as_str());
        output
    }

    fn lower_switch_statement(
        &mut self,
        path: &PathBuf,
        text: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let fragment = Fragment::new(path.clone(), text);
        let tokens = fragment.significant_tokens();
        let switch_expr = fragment
            .text_between(
                tokens[0].span.end,
                fragment
                    .first_body_newline(&tokens)
                    .unwrap_or(tokens[1].span.start),
            )
            .trim()
            .to_owned();
        let switch_value = self.next_temp("switch");
        let sections = fragment.switch_sections(&tokens);

        let mut output = String::new();
        writeln!(output, "do").ok();
        writeln!(
            output,
            "    local {} = {}",
            switch_value,
            self.lower_expression(switch_expr.as_str())
        )
        .ok();

        let mut first_branch = true;
        let mut default_body = None;
        let mut pending_labels = Vec::new();

        for section in sections {
            match section.label {
                SwitchLabel::Case(exprs) => {
                    if section.body.trim().is_empty() {
                        pending_labels.extend(exprs);
                        continue;
                    }

                    let mut all_exprs = pending_labels.clone();
                    all_exprs.extend(exprs);
                    pending_labels.clear();

                    let lowered_conditions = all_exprs
                        .iter()
                        .map(|expr| {
                            format!("{} == {}", switch_value, self.lower_expression(expr.trim()))
                        })
                        .collect::<Vec<_>>()
                        .join(" or ");
                    let lowered_body = indent_block(
                        self.lower_nested_block(path.clone(), section.body.as_str(), diagnostics)
                            .as_str(),
                        "    ",
                    );
                    if first_branch {
                        writeln!(output, "    if {} then", lowered_conditions).ok();
                        first_branch = false;
                    } else {
                        writeln!(output, "    elseif {} then", lowered_conditions).ok();
                    }
                    output.push_str(lowered_body.as_str());
                }
                SwitchLabel::Default => {
                    default_body = Some(indent_block(
                        self.lower_nested_block(path.clone(), section.body.as_str(), diagnostics)
                            .as_str(),
                        "    ",
                    ));
                }
            }
        }

        if first_branch {
            diagnostics.push(Diagnostic::warning(
                Some(path),
                Some(Span::new(0, text.len())),
                "switch statement has no case clauses",
            ));
            output.push_str("    -- empty switch\n");
        } else if let Some(default_body) = default_body {
            output.push_str("    else\n");
            output.push_str(default_body.as_str());
            output.push_str("    end\n");
        } else {
            output.push_str("    end\n");
        }

        output.push_str("end");
        output.push_str(fragment.trailing_newline().as_str());
        output
    }

    fn lower_expression_list(&mut self, text: &str) -> String {
        split_top_level(text, ',')
            .into_iter()
            .map(|part| self.lower_expression(part.trim()))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn lower_expression(&mut self, text: &str) -> String {
        let fragment = Fragment::new(PathBuf::from("<expr>"), text);
        let tokens = fragment
            .tokens
            .iter()
            .filter(|token| !token.is_trivia() && token.kind != TokenKind::Eof)
            .cloned()
            .collect::<Vec<_>>();

        if tokens.is_empty() {
            return text.to_owned();
        }

        let mut parser = ExprParser::new(tokens);
        let expr = parser.parse_expression(0);
        self.render_expression(&expr)
    }

    fn render_expression(&mut self, expr: &Expr) -> String {
        match expr {
            Expr::Raw(text) | Expr::Literal(text) | Expr::Name(text) => text.clone(),
            Expr::Group(inner) => format!("({})", self.render_expression(inner)),
            Expr::Unary { op, expr } => format!("({}{})", op, self.render_expression(expr)),
            Expr::Binary { left, op, right } => format!(
                "({} {} {})",
                self.render_expression(left),
                op,
                self.render_expression(right)
            ),
            Expr::Ternary {
                cond,
                then_expr,
                else_expr,
            } => format!(
                "(if {} then {} else {})",
                self.render_expression(cond),
                self.render_expression(then_expr),
                self.render_expression(else_expr)
            ),
            Expr::Nullish { left, right } => {
                let temp = self.next_temp("nullish");
                format!(
                    "(function()\n    local {temp} = {left}\n    if {temp} ~= nil then\n        return {temp}\n    end\n    return {right}\nend)()",
                    left = self.render_expression(left),
                    right = self.render_expression(right)
                )
            }
            Expr::Pipe { left, right } => self.render_pipe(left, right),
            Expr::Chain { base, segments } => self.render_chain(base, segments),
        }
    }

    fn render_pipe(&mut self, left: &Expr, right: &Expr) -> String {
        let input = self.render_expression(left);
        match right {
            Expr::Chain { base, segments } if !segments.is_empty() => {
                let mut updated_segments = segments.clone();
                if let Some(ChainSegment::Call { args, optional }) = updated_segments.last_mut() {
                    let mut next_args = vec![Expr::Raw(input)];
                    next_args.extend(args.clone());
                    *args = next_args;
                    *optional = false;
                    return self.render_chain(base, &updated_segments);
                }
            }
            _ => {}
        }

        format!("{}({})", self.render_expression(right), input)
    }

    fn render_chain(&mut self, base: &Expr, segments: &[ChainSegment]) -> String {
        let has_optional = segments.iter().any(ChainSegment::is_optional);
        if !has_optional {
            let mut current = self.render_expression(base);
            for segment in segments {
                current = self.apply_chain_segment(current, segment);
            }
            return current;
        }

        let mut prelude = String::new();
        let mut current = self.render_expression(base);
        for segment in segments {
            if segment.is_optional() {
                let temp = self.next_temp("chain");
                writeln!(prelude, "    local {} = {}", temp, current).ok();
                writeln!(prelude, "    if {} == nil then", temp).ok();
                writeln!(prelude, "        return nil").ok();
                writeln!(prelude, "    end").ok();
                current = self.apply_chain_segment(temp, &segment.non_optional());
            } else {
                current = self.apply_chain_segment(current, segment);
            }
        }

        format!("(function()\n{}    return {}\nend)()", prelude, current)
    }

    fn apply_chain_segment(&mut self, base: String, segment: &ChainSegment) -> String {
        match segment {
            ChainSegment::Member { name, .. } => format!("{}.{}", base, name),
            ChainSegment::Index { expr, .. } => {
                format!("{}[{}]", base, self.render_expression(expr))
            }
            ChainSegment::Call { args, .. } => format!(
                "{}({})",
                base,
                args.iter()
                    .map(|arg| self.render_expression(arg))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            ChainSegment::MethodCall { name, args } => format!(
                "{}:{}({})",
                base,
                name,
                args.iter()
                    .map(|arg| self.render_expression(arg))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    fn lower_nullish_assignment(
        &mut self,
        lhs: &str,
        rhs: &str,
        trailing_newline: String,
    ) -> String {
        let temp = self.next_temp("nullish_assign");
        format!(
            "do\n    local {temp} = {lhs}\n    if {temp} == nil then\n        {lhs} = {rhs}\n    end\nend{trailing}",
            temp = temp,
            lhs = lhs,
            rhs = self.lower_expression(rhs),
            trailing = trailing_newline
        )
    }

    fn lower_destructuring_local(
        &mut self,
        fragment: &Fragment,
        pattern_start: usize,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let Some(assign_index) = fragment.find_top_level_symbol(Symbol::Assign) else {
            return fragment.text.clone();
        };

        let pattern_end = match fragment.tokens[pattern_start].kind {
            TokenKind::Symbol(Symbol::LeftBrace) => {
                fragment.matching_delimiter(pattern_start, Symbol::LeftBrace, Symbol::RightBrace)
            }
            TokenKind::Symbol(Symbol::LeftBracket) => fragment.matching_delimiter(
                pattern_start,
                Symbol::LeftBracket,
                Symbol::RightBracket,
            ),
            _ => pattern_start,
        };
        let pattern = fragment.slice_between_tokens(pattern_start, pattern_end + 1);
        let expr = fragment.slice_between_tokens(assign_index + 1, fragment.tokens.len() - 1);
        let temp = self.next_temp("destructure");
        let lowered_expr = self.lower_expression(expr.trim());

        match parse_binding_pattern(pattern.trim()) {
            Some(BindingPattern::Object(bindings)) => {
                let mut lines = vec![format!("local {} = {}", temp, lowered_expr)];
                for binding in bindings {
                    lines.push(format!(
                        "local {} = {}.{}",
                        binding.alias, temp, binding.key
                    ));
                }
                format!("{}{}", lines.join("\n"), fragment.trailing_newline())
            }
            Some(BindingPattern::Array(bindings)) => {
                let mut lines = vec![format!("local {} = {}", temp, lowered_expr)];
                for (index, binding) in bindings.iter().enumerate() {
                    lines.push(format!("local {} = {}[{}]", binding.alias, temp, index + 1));
                }
                format!("{}{}", lines.join("\n"), fragment.trailing_newline())
            }
            None => {
                diagnostics.push(Diagnostic::warning(
                    Some(&fragment.path),
                    Some(Span::new(0, fragment.text.len())),
                    "unsupported destructuring pattern; leaving statement unchanged",
                ));
                fragment.text.clone()
            }
        }
    }

    fn lower_generic_for(
        &mut self,
        head: &str,
        lowered_body: String,
        trailing_newline: String,
    ) -> String {
        let Some((targets, exprs)) = head.split_once(" in ") else {
            return format!("for {} do\n{}end{}", head, lowered_body, trailing_newline);
        };
        let targets = targets.trim();
        let exprs = self.lower_expression_list(exprs.trim());

        match parse_binding_pattern(targets) {
            Some(pattern) => {
                let temp = self.next_temp("iter");
                let prologue = emit_binding_pattern(&pattern, temp.as_str());
                format!(
                    "for {} in {} do\n{}{}end{}",
                    temp,
                    exprs,
                    indent_block(prologue.as_str(), "    "),
                    lowered_body,
                    trailing_newline
                )
            }
            None => format!(
                "for {} in {} do\n{}end{}",
                targets, exprs, lowered_body, trailing_newline
            ),
        }
    }

    fn lower_numeric_for_head(&mut self, head: &str) -> String {
        let Some((binding, rest)) = head.split_once('=') else {
            return head.to_owned();
        };

        format!(
            "{} = {}",
            binding.trim(),
            self.lower_expression_list(rest.trim())
        )
    }

    fn lower_function_parameters(&mut self, params: &str) -> (String, String) {
        let mut lowered_params = Vec::new();
        let mut prologue = Vec::new();

        for param in split_top_level(params, ',') {
            let trimmed = param.trim();
            if let Some(pattern) = parse_binding_pattern(trimmed) {
                let temp = self.next_temp("param");
                lowered_params.push(temp.clone());
                prologue.push(emit_binding_pattern(&pattern, temp.as_str()));
            } else {
                lowered_params.push(trimmed.to_owned());
            }
        }

        let prelude = if prologue.is_empty() {
            String::new()
        } else {
            format!("{}\n", indent_block(prologue.join("\n").as_str(), "    "))
        };

        (lowered_params.join(", "), prelude)
    }

    fn next_temp(&mut self, prefix: &str) -> String {
        self.next_temp_id += 1;
        format!("_xluau_{}_{}", prefix, self.next_temp_id)
    }

    fn check_const_reassignment(
        &self,
        fragment: &Fragment,
        lhs: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        for name in split_top_level(lhs, ',') {
            let name = name.trim();
            if self.resolve_assignment_to_const(name) {
                diagnostics.push(Diagnostic::error(
                    Some(&fragment.path),
                    Some(Span::new(0, fragment.text.len())),
                    format!("cannot reassign const binding `{name}`"),
                ));
            }
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(ScopeFrame::default());
    }

    fn pop_scope(&mut self) {
        let _ = self.scopes.pop();
    }

    fn declare_local(&mut self, name: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.locals.insert(name.to_owned());
        }
    }

    fn declare_const(&mut self, name: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.locals.insert(name.to_owned());
            scope.consts.insert(name.to_owned());
        }
    }

    fn resolve_assignment_to_const(&self, name: &str) -> bool {
        for scope in self.scopes.iter().rev() {
            if scope.locals.contains(name) {
                return scope.consts.contains(name);
            }
        }

        false
    }
}

#[derive(Debug)]
struct Fragment {
    path: PathBuf,
    text: String,
    tokens: Vec<Token>,
}

impl Fragment {
    fn new(path: PathBuf, text: &str) -> Self {
        let source = SourceFile::virtual_file(path.clone(), SourceKind::XLuau, text.to_owned());
        let tokens = Lexer::new(&source).lex(&mut Vec::new());
        Self {
            path,
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

    fn token_text<'a>(&'a self, token: &Token) -> &'a str {
        &self.text[token.span.start..token.span.end]
    }

    fn text_between(&self, start: usize, end: usize) -> String {
        if start >= end || end > self.text.len() {
            return String::new();
        }
        self.text[start..end].to_owned()
    }

    fn trailing_newline(&self) -> String {
        if self.text.ends_with("\r\n") {
            "\n".to_owned()
        } else if self.text.ends_with('\n') {
            "\n".to_owned()
        } else {
            String::new()
        }
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

    fn matching_delimiter(&self, start: usize, open: Symbol, close: Symbol) -> usize {
        let mut depth = 0usize;
        for (index, token) in self.tokens.iter().enumerate().skip(start) {
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

    fn top_level_identifier_list(&self, start: usize, end: usize) -> Option<Vec<String>> {
        let mut names = Vec::new();
        let mut paren = 0usize;
        let mut brace = 0usize;
        let mut bracket = 0usize;

        for token in self
            .tokens
            .iter()
            .skip(start)
            .take(end.saturating_sub(start))
        {
            match token.kind {
                TokenKind::Symbol(Symbol::LeftParen) => paren += 1,
                TokenKind::Symbol(Symbol::RightParen) => paren = paren.saturating_sub(1),
                TokenKind::Symbol(Symbol::LeftBrace) => brace += 1,
                TokenKind::Symbol(Symbol::RightBrace) => brace = brace.saturating_sub(1),
                TokenKind::Symbol(Symbol::LeftBracket) => bracket += 1,
                TokenKind::Symbol(Symbol::RightBracket) => bracket = bracket.saturating_sub(1),
                TokenKind::Identifier if paren == 0 && brace == 0 && bracket == 0 => {
                    names.push(token.lexeme.clone());
                }
                _ => {}
            }
        }

        if names.is_empty() { None } else { Some(names) }
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
        let mut depth = 0usize;
        for (index, token) in tokens.iter().enumerate().skip(start) {
            match token.kind {
                TokenKind::Symbol(Symbol::LeftParen) => depth += 1,
                TokenKind::Symbol(Symbol::RightParen) => {
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

    fn switch_sections(&self, tokens: &[Token]) -> Vec<SwitchSection> {
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
                    sections.push(SwitchSection {
                        label: SwitchLabel::Case(labels),
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
                    sections.push(SwitchSection {
                        label: SwitchLabel::Default,
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

#[derive(Debug, Clone)]
enum Expr {
    Raw(String),
    Literal(String),
    Name(String),
    Group(Box<Expr>),
    Unary {
        op: String,
        expr: Box<Expr>,
    },
    Binary {
        left: Box<Expr>,
        op: String,
        right: Box<Expr>,
    },
    Ternary {
        cond: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
    },
    Nullish {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Pipe {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Chain {
        base: Box<Expr>,
        segments: Vec<ChainSegment>,
    },
}

#[derive(Debug, Clone)]
enum ChainSegment {
    Member { name: String, optional: bool },
    Index { expr: Box<Expr>, optional: bool },
    Call { args: Vec<Expr>, optional: bool },
    MethodCall { name: String, args: Vec<Expr> },
}

impl ChainSegment {
    fn is_optional(&self) -> bool {
        match self {
            ChainSegment::Member { optional, .. }
            | ChainSegment::Index { optional, .. }
            | ChainSegment::Call { optional, .. } => *optional,
            ChainSegment::MethodCall { .. } => false,
        }
    }

    fn non_optional(&self) -> Self {
        match self {
            ChainSegment::Member { name, .. } => Self::Member {
                name: name.clone(),
                optional: false,
            },
            ChainSegment::Index { expr, .. } => Self::Index {
                expr: expr.clone(),
                optional: false,
            },
            ChainSegment::Call { args, .. } => Self::Call {
                args: args.clone(),
                optional: false,
            },
            ChainSegment::MethodCall { name, args } => Self::MethodCall {
                name: name.clone(),
                args: args.clone(),
            },
        }
    }
}

struct ExprParser {
    tokens: Vec<Token>,
    cursor: usize,
}

impl ExprParser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, cursor: 0 }
    }

    fn parse_expression(&mut self, min_bp: u8) -> Expr {
        let mut lhs = self.parse_prefix();

        loop {
            let Some(token) = self.peek() else {
                break;
            };

            if matches!(token.kind, TokenKind::Symbol(Symbol::Question)) && min_bp <= 1 {
                self.advance();
                let then_expr = self.parse_expression(0);
                self.consume_symbol(Symbol::Colon);
                let else_expr = self.parse_expression(1);
                lhs = Expr::Ternary {
                    cond: Box::new(lhs),
                    then_expr: Box::new(then_expr),
                    else_expr: Box::new(else_expr),
                };
                continue;
            }

            let Some((left_bp, right_bp, op_kind)) = infix_binding_power(token) else {
                break;
            };
            if left_bp < min_bp {
                break;
            }

            self.advance();
            let rhs = self.parse_expression(right_bp);
            lhs = match op_kind {
                InfixKind::Nullish => Expr::Nullish {
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                },
                InfixKind::Pipe => Expr::Pipe {
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                },
                InfixKind::Binary(op) => Expr::Binary {
                    left: Box::new(lhs),
                    op: op.to_owned(),
                    right: Box::new(rhs),
                },
            };
        }

        lhs
    }

    fn parse_prefix(&mut self) -> Expr {
        let Some(token) = self.advance() else {
            return Expr::Raw(String::new());
        };

        match token.kind {
            TokenKind::Identifier => self.parse_postfix(Expr::Name(token.lexeme)),
            TokenKind::Number
            | TokenKind::String
            | TokenKind::BacktickString
            | TokenKind::TripleString
            | TokenKind::RawTripleString => self.parse_postfix(Expr::Literal(token.lexeme)),
            TokenKind::Keyword(Keyword::Nil)
            | TokenKind::Keyword(Keyword::True)
            | TokenKind::Keyword(Keyword::False) => self.parse_postfix(Expr::Literal(token.lexeme)),
            TokenKind::Symbol(Symbol::LeftParen) => {
                let inner = self.parse_expression(0);
                self.consume_symbol(Symbol::RightParen);
                self.parse_postfix(Expr::Group(Box::new(inner)))
            }
            TokenKind::Symbol(Symbol::Minus) => Expr::Unary {
                op: "-".to_owned(),
                expr: Box::new(self.parse_expression(11)),
            },
            TokenKind::Keyword(Keyword::Not) => Expr::Unary {
                op: "not ".to_owned(),
                expr: Box::new(self.parse_expression(11)),
            },
            TokenKind::Symbol(Symbol::Hash) => Expr::Unary {
                op: "#".to_owned(),
                expr: Box::new(self.parse_expression(11)),
            },
            _ => Expr::Raw(token.lexeme),
        }
    }

    fn parse_postfix(&mut self, base: Expr) -> Expr {
        let expr = base;
        let mut segments = Vec::new();

        loop {
            let Some(token) = self.peek() else {
                break;
            };
            match token.kind {
                TokenKind::Symbol(Symbol::Dot) => {
                    self.advance();
                    if let Some(name) = self.advance() {
                        segments.push(ChainSegment::Member {
                            name: name.lexeme,
                            optional: false,
                        });
                    }
                }
                TokenKind::Symbol(Symbol::QuestionDot) => {
                    self.advance();
                    if self.peek_kind(TokenKind::Symbol(Symbol::LeftBracket)) {
                        self.advance();
                        let expr_index = self.parse_expression(0);
                        self.consume_symbol(Symbol::RightBracket);
                        segments.push(ChainSegment::Index {
                            expr: Box::new(expr_index),
                            optional: true,
                        });
                    } else if self.peek_kind(TokenKind::Symbol(Symbol::LeftParen)) {
                        self.advance();
                        let args = self.parse_argument_list();
                        self.consume_symbol(Symbol::RightParen);
                        segments.push(ChainSegment::Call {
                            args,
                            optional: true,
                        });
                    } else if let Some(name) = self.advance() {
                        segments.push(ChainSegment::Member {
                            name: name.lexeme,
                            optional: true,
                        });
                    }
                }
                TokenKind::Symbol(Symbol::LeftBracket) => {
                    self.advance();
                    let index = self.parse_expression(0);
                    self.consume_symbol(Symbol::RightBracket);
                    segments.push(ChainSegment::Index {
                        expr: Box::new(index),
                        optional: false,
                    });
                }
                TokenKind::Symbol(Symbol::LeftParen) => {
                    self.advance();
                    let args = self.parse_argument_list();
                    self.consume_symbol(Symbol::RightParen);
                    segments.push(ChainSegment::Call {
                        args,
                        optional: false,
                    });
                }
                TokenKind::Symbol(Symbol::Colon) => {
                    self.advance();
                    let Some(name) = self.advance() else {
                        break;
                    };
                    self.consume_symbol(Symbol::LeftParen);
                    let args = self.parse_argument_list();
                    self.consume_symbol(Symbol::RightParen);
                    segments.push(ChainSegment::MethodCall {
                        name: name.lexeme,
                        args,
                    });
                }
                _ => break,
            }
        }

        if segments.is_empty() {
            expr
        } else {
            Expr::Chain {
                base: Box::new(expr),
                segments,
            }
        }
    }

    fn parse_argument_list(&mut self) -> Vec<Expr> {
        let mut args = Vec::new();
        while let Some(token) = self.peek() {
            if token.kind == TokenKind::Symbol(Symbol::RightParen) {
                break;
            }
            args.push(self.parse_expression(0));
            if self.peek_kind(TokenKind::Symbol(Symbol::Comma)) {
                self.advance();
            } else {
                break;
            }
        }
        args
    }

    fn consume_symbol(&mut self, symbol: Symbol) {
        if self.peek_kind(TokenKind::Symbol(symbol)) {
            self.advance();
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.cursor)
    }

    fn peek_kind(&self, kind: TokenKind) -> bool {
        self.peek().map(|token| token.kind == kind).unwrap_or(false)
    }

    fn advance(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.cursor).cloned()?;
        self.cursor += 1;
        Some(token)
    }
}

#[derive(Debug, Clone, Copy)]
enum InfixKind<'a> {
    Binary(&'a str),
    Nullish,
    Pipe,
}

fn infix_binding_power(token: &Token) -> Option<(u8, u8, InfixKind<'static>)> {
    Some(match token.kind {
        TokenKind::Keyword(Keyword::Or) => (2, 3, InfixKind::Binary("or")),
        TokenKind::Keyword(Keyword::And) => (4, 5, InfixKind::Binary("and")),
        TokenKind::Symbol(Symbol::QuestionQuestion) => (6, 7, InfixKind::Nullish),
        TokenKind::Symbol(Symbol::PipeGreater) => (8, 9, InfixKind::Pipe),
        TokenKind::Symbol(Symbol::Equals) => (10, 11, InfixKind::Binary("==")),
        TokenKind::Symbol(Symbol::NotEquals) => (10, 11, InfixKind::Binary("~=")),
        TokenKind::Symbol(Symbol::Less) => (10, 11, InfixKind::Binary("<")),
        TokenKind::Symbol(Symbol::LessEqual) => (10, 11, InfixKind::Binary("<=")),
        TokenKind::Symbol(Symbol::Greater) => (10, 11, InfixKind::Binary(">")),
        TokenKind::Symbol(Symbol::GreaterEqual) => (10, 11, InfixKind::Binary(">=")),
        TokenKind::Symbol(Symbol::DotDot) => (12, 12, InfixKind::Binary("..")),
        TokenKind::Symbol(Symbol::Plus) => (14, 15, InfixKind::Binary("+")),
        TokenKind::Symbol(Symbol::Minus) => (14, 15, InfixKind::Binary("-")),
        TokenKind::Symbol(Symbol::Star) => (16, 17, InfixKind::Binary("*")),
        TokenKind::Symbol(Symbol::Slash) => (16, 17, InfixKind::Binary("/")),
        TokenKind::Symbol(Symbol::Percent) => (16, 17, InfixKind::Binary("%")),
        TokenKind::Symbol(Symbol::Caret) => (18, 18, InfixKind::Binary("^")),
        _ => return None,
    })
}

#[derive(Debug, Clone)]
enum BindingPattern {
    Object(Vec<ObjectBinding>),
    Array(Vec<ObjectBinding>),
}

#[derive(Debug, Clone)]
struct ObjectBinding {
    key: String,
    alias: String,
}

#[derive(Debug)]
enum SwitchLabel {
    Case(Vec<String>),
    Default,
}

#[derive(Debug)]
struct SwitchSection {
    label: SwitchLabel,
    body: String,
}

fn parse_binding_pattern(text: &str) -> Option<BindingPattern> {
    let trimmed = text.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        let inner = &trimmed[1..trimmed.len() - 1];
        let bindings = split_top_level(inner, ',')
            .into_iter()
            .map(|entry| {
                let entry = entry.trim();
                if let Some((key, alias)) = entry.split_once(':') {
                    ObjectBinding {
                        key: key.trim().to_owned(),
                        alias: alias.trim().to_owned(),
                    }
                } else {
                    ObjectBinding {
                        key: entry.to_owned(),
                        alias: entry.to_owned(),
                    }
                }
            })
            .filter(|binding| !binding.alias.is_empty())
            .collect::<Vec<_>>();
        return Some(BindingPattern::Object(bindings));
    }

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        let inner = &trimmed[1..trimmed.len() - 1];
        let bindings = split_top_level(inner, ',')
            .into_iter()
            .map(|entry| {
                let alias = entry.trim().to_owned();
                ObjectBinding {
                    key: alias.clone(),
                    alias,
                }
            })
            .filter(|binding| !binding.alias.is_empty())
            .collect::<Vec<_>>();
        return Some(BindingPattern::Array(bindings));
    }

    None
}

fn emit_binding_pattern(pattern: &BindingPattern, source_name: &str) -> String {
    match pattern {
        BindingPattern::Object(bindings) => bindings
            .iter()
            .map(|binding| format!("local {} = {}.{}", binding.alias, source_name, binding.key))
            .collect::<Vec<_>>()
            .join("\n"),
        BindingPattern::Array(bindings) => bindings
            .iter()
            .enumerate()
            .map(|(index, binding)| {
                format!("local {} = {}[{}]", binding.alias, source_name, index + 1)
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
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

fn indent_block(text: &str, prefix: &str) -> String {
    if text.trim().is_empty() {
        return String::new();
    }

    text.lines()
        .map(|line| format!("{prefix}{line}\n"))
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::Lowerer;
    use crate::diagnostic::Diagnostic;
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::source::{SourceFile, SourceKind};

    fn lower(text: &str) -> (String, Vec<Diagnostic>) {
        let source =
            SourceFile::virtual_file(PathBuf::from("test.xl"), SourceKind::XLuau, text.to_owned());
        let mut diagnostics = Vec::new();
        let tokens = Lexer::new(&source).lex(&mut diagnostics);
        let program = Parser::new(&source, &tokens).parse(&mut diagnostics);
        let mut lowerer = Lowerer::new();
        let lowered = lowerer.lower_program(&source, &program, &mut diagnostics);
        (lowered, diagnostics)
    }

    #[test]
    fn lowers_ternary_nullish_and_pipe() {
        let (lowered, diagnostics) =
            lower("local value = user?.name ?? fallback ? fallback : default |> format\n");

        assert!(diagnostics.is_empty());
        assert!(lowered.contains("function()"));
        assert!(lowered.contains("if"));
        assert!(lowered.contains("format"));
    }

    #[test]
    fn lowers_const_and_reports_reassignment() {
        let (_, diagnostics) = lower("const answer = 42\nanswer = 0\n");
        assert!(diagnostics.iter().any(Diagnostic::is_error));
    }

    #[test]
    fn let_is_lowered_like_local_and_shadowing_stops_const_errors() {
        let (lowered, diagnostics) =
            lower("const value = 1\nif true then\n    let value = 2\n    value = 3\nend\n");

        assert!(diagnostics.iter().all(|diagnostic| !diagnostic.is_error()));
        assert!(lowered.contains("local value = 2"));
    }

    #[test]
    fn lowers_destructuring_and_switch() {
        let (lowered, diagnostics) = lower(
            "local {x, y: z} = point\nswitch value\ncase 1:\n    print(x)\ndefault:\n    print(z)\nend\n",
        );

        assert!(diagnostics.iter().all(|diagnostic| !diagnostic.is_error()));
        assert!(lowered.contains("local x ="));
        assert!(lowered.contains("local _xluau_switch_"));
        assert!(lowered.contains("if _xluau_switch_"));
    }

    #[test]
    fn lowers_function_param_and_for_destructuring() {
        let (lowered, diagnostics) = lower(
            "function demo({x, y}, [a, b])\n    for [left, right] in pairs(items) do\n        print(x, y, a, b, left, right)\n    end\nend\n",
        );

        assert!(diagnostics.iter().all(|diagnostic| !diagnostic.is_error()));
        assert!(lowered.contains("function demo(_xluau_param_"));
        assert!(lowered.contains("local x = _xluau_param_"));
        assert!(lowered.contains("for _xluau_iter_"));
    }

    #[test]
    fn lowers_switch_fallthrough_labels() {
        let (lowered, diagnostics) =
            lower("switch value\ncase 1:\ncase 2:\n    print(\"hit\")\nend\n");

        assert!(diagnostics.iter().all(|diagnostic| !diagnostic.is_error()));
        assert!(lowered.contains("== 1 or"));
        assert!(lowered.contains("== 2"));
    }
}
