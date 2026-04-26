use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, XLuauError};

const CONFIG_FILE_NAME: &str = "xluau.config.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TargetKind {
    Filesystem,
    Roblox,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LuauTarget {
    NewSolver,
    Legacy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AsyncAdapter {
    Coroutine,
    Promise,
    Roblox,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct XLuauConfig {
    pub version: u32,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    #[serde(rename = "outDir")]
    pub out_dir: PathBuf,
    pub target: TargetKind,
    #[serde(rename = "luauTarget")]
    pub luau_target: LuauTarget,
    #[serde(rename = "baseDir")]
    pub base_dir: PathBuf,
    pub paths: BTreeMap<String, String>,
    pub extensions: Vec<String>,
    #[serde(rename = "indexFiles")]
    pub index_files: Vec<String>,
    #[serde(rename = "sourceMaps")]
    pub source_maps: bool,
    #[serde(rename = "linePragmas")]
    pub line_pragmas: bool,
    pub strict: bool,
    #[serde(rename = "noImplicitAny")]
    pub no_implicit_any: bool,
    #[serde(rename = "noUncheckedOptionalChain")]
    pub no_unchecked_optional_chain: bool,
    #[serde(rename = "asyncAdapter")]
    pub async_adapter: AsyncAdapter,
    #[serde(rename = "emitReadonly")]
    pub emit_readonly: bool,
    #[serde(rename = "decoratorLibrary")]
    pub decorator_library: Option<PathBuf>,
    #[serde(rename = "typeCheckOnly")]
    pub type_check_only: bool,
    pub plugins: Vec<String>,
}

impl Default for XLuauConfig {
    fn default() -> Self {
        Self {
            version: 1,
            include: vec!["src/**/*.xl".to_owned()],
            exclude: Vec::new(),
            out_dir: PathBuf::from("out"),
            target: TargetKind::Filesystem,
            luau_target: LuauTarget::NewSolver,
            base_dir: PathBuf::from("src"),
            paths: BTreeMap::new(),
            extensions: vec![".xl".to_owned(), ".luau".to_owned(), ".lua".to_owned()],
            index_files: vec!["init".to_owned()],
            source_maps: true,
            line_pragmas: false,
            strict: true,
            no_implicit_any: true,
            no_unchecked_optional_chain: true,
            async_adapter: AsyncAdapter::Coroutine,
            emit_readonly: true,
            decorator_library: None,
            type_check_only: false,
            plugins: Vec::new(),
        }
    }
}

impl XLuauConfig {
    pub fn load_or_default(project_root: &Path, explicit_path: Option<&Path>) -> Result<Self> {
        let path = explicit_path
            .map(Path::to_path_buf)
            .unwrap_or_else(|| project_root.join(CONFIG_FILE_NAME));

        if !path.exists() {
            let default = Self::default();
            default.validate()?;
            return Ok(default);
        }

        let contents = fs::read_to_string(&path)?;
        let config: Self = serde_json::from_str(&contents).map_err(|error| {
            XLuauError::Validation(format!("failed to parse {}: {error}", path.display()))
        })?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err(XLuauError::Validation(format!(
                "unsupported config version {}; expected 1",
                self.version
            )));
        }

        if self.include.is_empty() {
            return Err(XLuauError::Validation(
                "`include` must contain at least one glob pattern".to_owned(),
            ));
        }

        if self.out_dir.as_os_str().is_empty() {
            return Err(XLuauError::Validation(
                "`outDir` cannot be empty".to_owned(),
            ));
        }

        if self.base_dir.as_os_str().is_empty() {
            return Err(XLuauError::Validation(
                "`baseDir` cannot be empty".to_owned(),
            ));
        }

        if self.extensions.is_empty() {
            return Err(XLuauError::Validation(
                "`extensions` must list at least one file extension".to_owned(),
            ));
        }

        for extension in &self.extensions {
            if !extension.starts_with('.') || extension.len() < 2 {
                return Err(XLuauError::Validation(format!(
                    "extensions must start with `.`, found `{extension}`"
                )));
            }
        }

        for (alias, target) in &self.paths {
            if alias.trim().is_empty() || target.trim().is_empty() {
                return Err(XLuauError::Validation(
                    "path aliases cannot contain empty keys or values".to_owned(),
                ));
            }
        }

        if self.line_pragmas && !self.source_maps {
            return Err(XLuauError::Validation(
                "`linePragmas` requires `sourceMaps` to stay enabled in Phase 1".to_owned(),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::XLuauConfig;

    #[test]
    fn default_config_is_valid() {
        XLuauConfig::default().validate().expect("valid config");
    }
}
