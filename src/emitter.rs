use crate::ast::{
    ConditionalKeyword, LocalKeyword, Program, Statement, StatementNode, SwitchLabel,
};

#[derive(Debug, Clone)]
pub struct EmittedModule {
    pub text: String,
}

#[derive(Debug, Default)]
pub struct Emitter;

impl Emitter {
    pub fn new() -> Self {
        Self
    }

    pub fn emit(&self, program: &Program) -> EmittedModule {
        EmittedModule {
            text: self.emit_statements(&program.statements),
        }
    }

    fn emit_statements(&self, statements: &[Statement]) -> String {
        statements
            .iter()
            .map(|statement| self.emit_statement(statement))
            .collect()
    }

    fn emit_statement(&self, statement: &Statement) -> String {
        let mut text = match &statement.node {
            StatementNode::Trivia(text) | StatementNode::Text(text) => text.clone(),
            StatementNode::Local(local) => {
                let keyword = match local.keyword {
                    LocalKeyword::Local => "local",
                    LocalKeyword::Let => "let",
                    LocalKeyword::Const => "const",
                };
                match &local.value {
                    Some(value) => format!("{keyword} {} = {}", local.bindings, value),
                    None => format!("{keyword} {}", local.bindings),
                }
            }
            StatementNode::Return(ret) => match &ret.values {
                Some(values) => format!("return {values}"),
                None => "return".to_owned(),
            },
            StatementNode::If(if_stmt) => {
                let mut output = String::new();
                for clause in &if_stmt.clauses {
                    let keyword = match clause.keyword {
                        ConditionalKeyword::If => "if",
                        ConditionalKeyword::ElseIf => "elseif",
                    };
                    output.push_str(format!("{keyword} {} then\n", clause.condition).as_str());
                    output.push_str(self.emit_statements(&clause.body).as_str());
                }
                if let Some(body) = &if_stmt.else_body {
                    output.push_str("else\n");
                    output.push_str(self.emit_statements(body).as_str());
                }
                output.push_str("end");
                output
            }
            StatementNode::While(while_stmt) => format!(
                "while {} do\n{}end",
                while_stmt.condition,
                self.emit_statements(&while_stmt.body)
            ),
            StatementNode::Repeat(repeat_stmt) => format!(
                "repeat\n{}until {}",
                self.emit_statements(&repeat_stmt.body),
                repeat_stmt.condition
            ),
            StatementNode::For(for_stmt) => format!(
                "for {} do\n{}end",
                for_stmt.head,
                self.emit_statements(&for_stmt.body)
            ),
            StatementNode::Function(function) => {
                let mut output = String::new();
                output.push_str(function.header_prefix.as_str());
                output.push_str(function.params.as_str());
                output.push_str(function.header_suffix.as_str());
                output.push_str(self.emit_statements(&function.body).as_str());
                output.push_str("end");
                output
            }
            StatementNode::Do(block) => format!("do\n{}end", self.emit_statements(&block.body)),
            StatementNode::Switch(switch) => {
                let mut output = format!("switch {}\n", switch.expression);
                for section in &switch.sections {
                    match &section.label {
                        SwitchLabel::Case(values) => {
                            output.push_str(format!("case {}:\n", values.join(", ")).as_str());
                        }
                        SwitchLabel::Default => output.push_str("default:\n"),
                    }
                    output.push_str(self.emit_statements(&section.body).as_str());
                }
                output.push_str("end");
                output
            }
        };
        text.push_str(statement.trailing.as_str());
        text
    }
}

#[cfg(test)]
mod tests {
    use crate::ast::{
        ConditionalClause, ConditionalKeyword, IfStatement, Program, ReturnStatement, Statement,
        StatementKind, StatementNode,
    };
    use crate::diagnostic::Span;
    use crate::emitter::Emitter;
    use crate::source::SourceKind;

    #[test]
    fn emitter_walks_structured_statements() {
        let program = Program {
            source_kind: SourceKind::XLuau,
            span: Span::new(0, 0),
            statements: vec![Statement {
                kind: StatementKind::Luau,
                trailing: "\n".to_owned(),
                span: Span::new(0, 0),
                node: StatementNode::If(IfStatement {
                    clauses: vec![ConditionalClause {
                        keyword: ConditionalKeyword::If,
                        condition: "ready".to_owned(),
                        body: vec![Statement {
                            kind: StatementKind::Luau,
                            trailing: "\n".to_owned(),
                            span: Span::new(0, 0),
                            node: StatementNode::Return(ReturnStatement {
                                values: Some("value".to_owned()),
                            }),
                        }],
                    }],
                    else_body: None,
                }),
            }],
        };

        let emitted = Emitter::new().emit(&program);
        assert_eq!(emitted.text, "if ready then\nreturn value\nend\n");
    }
}
