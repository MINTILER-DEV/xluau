use crate::ast::Program;
use crate::diagnostic::Diagnostic;
use crate::source::SourceFile;

#[derive(Debug, Default)]
pub struct Lowerer;

impl Lowerer {
    pub fn new() -> Self {
        Self
    }

    pub fn lower_program(
        &self,
        _source: &SourceFile,
        program: &Program,
        _diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        program
            .statements
            .iter()
            .map(|statement| statement.raw_text.as_str())
            .collect::<String>()
    }
}
