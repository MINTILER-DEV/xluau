use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub file: Option<PathBuf>,
    pub span: Option<Span>,
}

impl Diagnostic {
    pub fn error(file: Option<&Path>, span: Option<Span>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            file: file.map(Path::to_path_buf),
            span,
        }
    }

    pub fn warning(file: Option<&Path>, span: Option<Span>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
            file: file.map(Path::to_path_buf),
            span,
        }
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    pub fn render(&self, source: Option<&str>) -> String {
        let (line, column) = self
            .span
            .and_then(|span| source.map(|text| line_and_column(text, span.start)))
            .unwrap_or((0, 0));
        let location = self
            .file
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<unknown>".to_owned());

        if line == 0 {
            format!("{location}: {}: {}", self.severity, self.message)
        } else {
            format!(
                "{location}:{line}:{column}: {}: {}",
                self.severity, self.message
            )
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Warning => f.write_str("warning"),
            Severity::Error => f.write_str("error"),
        }
    }
}

fn line_and_column(source: &str, offset: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut column = 1usize;

    for (index, ch) in source.char_indices() {
        if index >= offset {
            break;
        }

        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }

    (line, column)
}
