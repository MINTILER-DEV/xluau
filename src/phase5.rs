use std::path::PathBuf;

use crate::ast::Program;
use crate::config::XLuauConfig;
use crate::diagnostic::Diagnostic;
use crate::emitter::Emitter;
use crate::source::SourceFile;

#[derive(Debug, Clone)]
pub struct PhaseFiveTransformer {
    #[allow(dead_code)]
    config: XLuauConfig,
}

impl PhaseFiveTransformer {
    pub fn new(config: XLuauConfig) -> Self {
        Self { config }
    }

    pub fn transform_program(
        &self,
        _source: &SourceFile,
        program: &Program,
        _diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        Emitter::new().emit(program).text
    }
}

#[allow(dead_code)]
fn _placeholder_path(_: &PathBuf) {}

