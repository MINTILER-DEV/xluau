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
    pub raw_text: String,
    pub span: Span,
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
