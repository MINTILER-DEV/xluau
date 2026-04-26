use std::fs;
use std::path::PathBuf;

use crate::error::{Result, XLuauError};

#[derive(Debug, Clone)]
pub struct SourceFile {
    pub path: PathBuf,
    pub kind: SourceKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    XLuau,
    Luau,
    Lua,
}

impl SourceFile {
    pub fn load(path: PathBuf) -> Result<Self> {
        let kind = SourceKind::from_path(&path)?;
        let text = fs::read_to_string(&path)?;
        Ok(Self { path, kind, text })
    }

    pub fn virtual_file(path: PathBuf, kind: SourceKind, text: String) -> Self {
        Self { path, kind, text }
    }
}

impl SourceKind {
    pub fn from_path(path: &std::path::Path) -> Result<Self> {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("xl") => Ok(Self::XLuau),
            Some("luau") => Ok(Self::Luau),
            Some("lua") => Ok(Self::Lua),
            _ => Err(XLuauError::Validation(format!(
                "unsupported file extension for {}",
                path.display()
            ))),
        }
    }
}
