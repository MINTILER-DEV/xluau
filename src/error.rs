use std::fmt;

use thiserror::Error;

use crate::diagnostic::Diagnostic;

pub type Result<T> = std::result::Result<T, XLuauError>;

#[derive(Debug, Error)]
pub enum XLuauError {
    #[error("{0}")]
    Validation(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    WalkDir(#[from] walkdir::Error),
    #[error("{0}")]
    DiagnosticsBundle(FormattedDiagnostics),
    #[error(transparent)]
    Cli(#[from] clap::Error),
}

#[derive(Debug, Clone)]
pub struct FormattedDiagnostics(String);

impl XLuauError {
    pub fn diagnostics(diagnostics: Vec<Diagnostic>) -> Self {
        let rendered = diagnostics
            .into_iter()
            .map(|diagnostic| diagnostic.render(None))
            .collect::<Vec<_>>()
            .join("\n");

        Self::DiagnosticsBundle(FormattedDiagnostics(rendered))
    }
}

impl fmt::Display for FormattedDiagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
