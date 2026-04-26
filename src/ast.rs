#![allow(dead_code)]

use crate::diagnostic::Span;
use crate::source::SourceKind;

#[derive(Debug, Clone)]
pub struct Program {
    pub source_kind: SourceKind,
    pub statements: Vec<Statement>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Statement {
    pub kind: StatementKind,
    pub node: StatementNode,
    pub trailing: String,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum StatementNode {
    Trivia(String),
    Text(String),
    Import(ImportStatement),
    Export(ExportStatement),
    Local(LocalStatement),
    Return(ReturnStatement),
    If(IfStatement),
    While(WhileStatement),
    Repeat(RepeatStatement),
    For(ForStatement),
    Function(FunctionStatement),
    Do(BlockStatement),
    Switch(SwitchStatement),
}

#[derive(Debug, Clone)]
pub struct LocalStatement {
    pub keyword: LocalKeyword,
    pub bindings: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ImportStatement {
    pub kind: ImportKind,
    pub source: String,
}

#[derive(Debug, Clone)]
pub enum ImportKind {
    SideEffect,
    TypeNamed {
        named: Vec<NamedImportSpecifier>,
    },
    Value {
        default: Option<String>,
        namespace: Option<String>,
        named: Vec<NamedImportSpecifier>,
    },
}

#[derive(Debug, Clone)]
pub struct NamedImportSpecifier {
    pub imported: String,
    pub local: String,
}

#[derive(Debug, Clone)]
pub struct ExportStatement {
    pub kind: ExportKind,
}

#[derive(Debug, Clone)]
pub enum ExportKind {
    Declaration(Box<StatementNode>),
    Named {
        specifiers: Vec<NamedExportSpecifier>,
        source: Option<String>,
        is_type_only: bool,
    },
    All {
        source: String,
        is_type_only: bool,
    },
    Default {
        expression: String,
    },
    TypeDeclaration(String),
}

#[derive(Debug, Clone)]
pub struct NamedExportSpecifier {
    pub local: String,
    pub exported: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalKeyword {
    Local,
    Let,
    Const,
}

#[derive(Debug, Clone)]
pub struct ReturnStatement {
    pub values: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IfStatement {
    pub clauses: Vec<ConditionalClause>,
    pub else_body: Option<Vec<Statement>>,
}

#[derive(Debug, Clone)]
pub struct ConditionalClause {
    pub keyword: ConditionalKeyword,
    pub condition: String,
    pub body: Vec<Statement>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionalKeyword {
    If,
    ElseIf,
}

#[derive(Debug, Clone)]
pub struct WhileStatement {
    pub condition: String,
    pub body: Vec<Statement>,
}

#[derive(Debug, Clone)]
pub struct RepeatStatement {
    pub body: Vec<Statement>,
    pub condition: String,
}

#[derive(Debug, Clone)]
pub struct ForStatement {
    pub head: String,
    pub body: Vec<Statement>,
}

#[derive(Debug, Clone)]
pub struct FunctionStatement {
    pub header_prefix: String,
    pub params: String,
    pub header_suffix: String,
    pub body: Vec<Statement>,
}

#[derive(Debug, Clone)]
pub struct BlockStatement {
    pub body: Vec<Statement>,
}

#[derive(Debug, Clone)]
pub struct SwitchStatement {
    pub expression: String,
    pub sections: Vec<SwitchSection>,
}

#[derive(Debug, Clone)]
pub struct SwitchSection {
    pub label: SwitchLabel,
    pub body: Vec<Statement>,
}

#[derive(Debug, Clone)]
pub enum SwitchLabel {
    Case(Vec<String>),
    Default,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatementKind {
    Luau,
    ImportDeclaration,
    ExportDeclaration,
    TypeDeclaration,
    XLuauDeclaration,
    XLuauExpression,
    Comment,
    Whitespace,
}
