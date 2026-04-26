use std::fs;
use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use walkdir::WalkDir;

use crate::ast::Program;
use crate::config::XLuauConfig;
use crate::diagnostic::Diagnostic;
use crate::emitter::Emitter;
use crate::error::{Result, XLuauError};
use crate::formatter::Formatter;
use crate::lexer::Lexer;
use crate::lowering::Lowerer;
use crate::parser::Parser;
use crate::resolver::Resolver;
use crate::source::{SourceFile, SourceKind};

#[derive(Debug)]
pub struct Compiler {
    project_root: PathBuf,
    config: XLuauConfig,
}

#[derive(Debug, Clone, Copy)]
enum Mode {
    Build,
    Check,
}

#[derive(Debug)]
pub struct BuildSummary {
    pub checked_files: usize,
    pub written_files: usize,
}

#[derive(Debug)]
pub struct CheckSummary {
    pub checked_files: usize,
}

#[derive(Debug)]
struct CompiledFile {
    source: SourceFile,
    _program: Program,
    output: String,
}

impl Compiler {
    pub fn new(project_root: PathBuf, config: XLuauConfig) -> Result<Self> {
        Ok(Self {
            project_root,
            config,
        })
    }

    pub fn build(&self, paths: &[PathBuf]) -> Result<BuildSummary> {
        let compiled = self.compile_many(paths, Mode::Build)?;
        let mut written_files = 0;

        for file in compiled {
            let output_path = self.output_path(&file.source)?;
            if let Some(parent) = output_path.parent() {
                fs::create_dir_all(parent)?;
            }

            fs::write(output_path, file.output)?;
            written_files += 1;
        }

        Ok(BuildSummary {
            checked_files: written_files,
            written_files,
        })
    }

    pub fn check(&self, paths: &[PathBuf]) -> Result<CheckSummary> {
        let compiled = self.compile_many(paths, Mode::Check)?;
        Ok(CheckSummary {
            checked_files: compiled.len(),
        })
    }

    fn compile_many(&self, paths: &[PathBuf], mode: Mode) -> Result<Vec<CompiledFile>> {
        let inputs = self.discover_inputs(paths)?;
        if inputs.is_empty() {
            return Err(XLuauError::Validation(
                "no matching input files were found".to_owned(),
            ));
        }

        let mut resolver = Resolver::new(self.project_root.clone(), self.config.clone());
        resolver.validate_entrypoints(&inputs)?;

        let mut compiled = Vec::with_capacity(inputs.len());
        let mut all_diagnostics = Vec::new();
        let mut warning_diagnostics: Vec<(Diagnostic, String)> = Vec::new();

        for path in inputs {
            let source = SourceFile::load(path)?;
            let mut diagnostics = Vec::new();

            let tokens = Lexer::new(&source).lex(&mut diagnostics);
            let program = Parser::new(&source, &tokens).parse(&mut diagnostics);

            if diagnostics.iter().any(Diagnostic::is_error) {
                all_diagnostics.extend(diagnostics);
                continue;
            }
            warning_diagnostics.extend(
                diagnostics
                    .drain(..)
                    .map(|diagnostic| (diagnostic, source.text.clone())),
            );

            let lowered_text = Lowerer::new().lower_program(&source, &program, &mut diagnostics);

            if diagnostics.iter().any(Diagnostic::is_error) {
                all_diagnostics.extend(diagnostics);
                continue;
            }
            warning_diagnostics.extend(
                diagnostics
                    .drain(..)
                    .map(|diagnostic| (diagnostic, source.text.clone())),
            );

            let lowered_source =
                SourceFile::virtual_file(source.path.clone(), SourceKind::Luau, lowered_text);
            let lowered_tokens = Lexer::new(&lowered_source).lex(&mut diagnostics);
            let lowered_program =
                Parser::new(&lowered_source, &lowered_tokens).parse(&mut diagnostics);

            if diagnostics.iter().any(Diagnostic::is_error) {
                all_diagnostics.extend(diagnostics);
                continue;
            }
            warning_diagnostics.extend(
                diagnostics
                    .drain(..)
                    .map(|diagnostic| (diagnostic, lowered_source.text.clone())),
            );

            let resolved_program = match resolver.resolve_program(&lowered_source, &lowered_program) {
                Ok(resolved) => resolved,
                Err(error) => {
                    all_diagnostics.push(Diagnostic::error(
                        Some(&source.path),
                        None,
                        error.to_string(),
                    ));
                    continue;
                }
            };

            let emitted = Emitter::new().emit_resolved(&resolved_program);
            let output = Formatter::default().format(&emitted.text);

            compiled.push(CompiledFile {
                source,
                _program: lowered_program,
                output,
            });

            if matches!(mode, Mode::Check) {
                continue;
            }
        }

        if all_diagnostics.is_empty() {
            for (diagnostic, source_text) in warning_diagnostics {
                if !diagnostic.is_error() {
                    eprintln!("{}", diagnostic.render(Some(&source_text)));
                }
            }
            Ok(compiled)
        } else {
            Err(XLuauError::diagnostics(all_diagnostics))
        }
    }

    fn discover_inputs(&self, requested_paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
        if requested_paths.is_empty() {
            return self.discover_from_config();
        }

        let mut files = Vec::new();
        for requested in requested_paths {
            let path = self.resolve_requested_path(requested);
            if path.is_file() {
                self.push_explicit_file(&path, &mut files)?;
                continue;
            }

            if path.is_dir() {
                for entry in WalkDir::new(&path) {
                    let entry = entry?;
                    if entry.file_type().is_file() {
                        self.push_if_supported(entry.path(), &mut files);
                    }
                }
                continue;
            }

            return Err(XLuauError::Validation(format!(
                "input path does not exist: {}",
                requested.display()
            )));
        }

        files.sort();
        files.dedup();
        Ok(files)
    }

    fn discover_from_config(&self) -> Result<Vec<PathBuf>> {
        let include_set = self.compile_glob_set(&self.config.include)?;
        let exclude_set = self.compile_glob_set(&self.config.exclude)?;
        let mut files = Vec::new();

        for entry in WalkDir::new(&self.project_root) {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.path();
            if !self.is_supported_extension(path) {
                continue;
            }

            let relative = self.relative_to_project(path)?;
            let normalized = normalize_path(relative);
            if include_set.is_match(&normalized) && !exclude_set.is_match(&normalized) {
                files.push(path.to_path_buf());
            }
        }

        files.sort();
        Ok(files)
    }

    fn resolve_requested_path(&self, requested: &Path) -> PathBuf {
        if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            self.project_root.join(requested)
        }
    }

    fn push_explicit_file(&self, path: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
        if self.is_supported_extension(path) {
            files.push(path.to_path_buf());
            Ok(())
        } else {
            Err(XLuauError::Validation(format!(
                "unsupported input file extension: {}",
                path.display()
            )))
        }
    }

    fn push_if_supported(&self, path: &Path, files: &mut Vec<PathBuf>) {
        if self.is_supported_extension(path) {
            files.push(path.to_path_buf());
        }
    }

    fn compile_glob_set(&self, patterns: &[String]) -> Result<GlobSet> {
        let mut builder = GlobSetBuilder::new();
        for pattern in patterns {
            builder.add(Glob::new(pattern).map_err(|error| {
                XLuauError::Validation(format!(
                    "invalid glob pattern `{pattern}` in config: {error}"
                ))
            })?);
        }

        builder.build().map_err(|error| {
            XLuauError::Validation(format!("failed to build glob matcher: {error}"))
        })
    }

    fn is_supported_extension(&self, path: &Path) -> bool {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some(ext) => self
                .config
                .extensions
                .iter()
                .any(|configured| configured.trim_start_matches('.') == ext),
            None => false,
        }
    }

    fn relative_to_project<'a>(&self, path: &'a Path) -> Result<&'a Path> {
        path.strip_prefix(&self.project_root).map_err(|_| {
            XLuauError::Validation(format!("path escapes the project root: {}", path.display()))
        })
    }

    fn output_path(&self, source: &SourceFile) -> Result<PathBuf> {
        let base_dir = self.project_root.join(&self.config.base_dir);
        let relative = source
            .path
            .strip_prefix(&base_dir)
            .or_else(|_| source.path.strip_prefix(&self.project_root))
            .map_err(|_| {
                XLuauError::Validation(format!(
                    "unable to determine output path for {}",
                    source.path.display()
                ))
            })?;

        let mut output = self.project_root.join(&self.config.out_dir).join(relative);
        if source.kind == SourceKind::XLuau {
            output.set_extension("luau");
        }

        Ok(output)
    }
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::Compiler;
    use crate::config::{TargetKind, XLuauConfig};

    #[test]
    fn build_transpiles_xl_file_to_luau_output() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("src")).expect("src dir");
        fs::write(
            temp.path().join("src/init.xl"),
            "local answer = 42\r\nprint(answer)",
        )
        .expect("source file");

        let compiler =
            Compiler::new(temp.path().to_path_buf(), XLuauConfig::default()).expect("compiler");
        let summary = compiler.build(&[]).expect("build");

        assert_eq!(summary.checked_files, 1);
        let emitted = fs::read_to_string(temp.path().join("out/init.luau")).expect("output");
        assert_eq!(emitted, "local answer = 42\nprint(answer)\n");
    }

    #[test]
    fn explicit_directory_inputs_ignore_non_xluau_files() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("workspace")).expect("workspace dir");
        fs::write(temp.path().join("workspace/main.xl"), "print('ok')").expect("xl file");
        fs::write(temp.path().join("workspace/readme.txt"), "ignored").expect("text file");

        let compiler =
            Compiler::new(temp.path().to_path_buf(), XLuauConfig::default()).expect("compiler");
        let summary = compiler
            .check(&[PathBuf::from("workspace")])
            .expect("directory check");

        assert_eq!(summary.checked_files, 1);
    }

    #[test]
    fn build_emits_phase_three_filesystem_modules() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("src")).expect("src dir");
        fs::write(
            temp.path().join("src/math.xl"),
            "export const answer = 42\nexport default answer\n",
        )
        .expect("math file");
        fs::write(
            temp.path().join("src/main.xl"),
            "import value, { answer as named } from \"./math\"\nexport { value as default, named }\n",
        )
        .expect("main file");

        let compiler =
            Compiler::new(temp.path().to_path_buf(), XLuauConfig::default()).expect("compiler");
        compiler.build(&[]).expect("build");

        let math_output = fs::read_to_string(temp.path().join("out/math.luau")).expect("math output");
        assert!(math_output.contains("local _exports = {}"));
        assert!(math_output.contains("_exports.answer = answer"));
        assert!(math_output.contains("_exports.__default = answer"));

        let main_output = fs::read_to_string(temp.path().join("out/main.luau")).expect("main output");
        assert!(main_output.contains("require(\"./math\")"));
        assert!(main_output.contains("_exports.__default = value"));
        assert!(main_output.contains("_exports.named = named"));
    }

    #[test]
    fn build_emits_roblox_require_paths() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("src/server/players")).expect("players dir");
        fs::write(
            temp.path().join("src/server/players/PlayerManager.xl"),
            "export function spawnPlayer()\n    return true\nend\n",
        )
        .expect("player manager");
        fs::write(
            temp.path().join("src/server/Game.xl"),
            "import { spawnPlayer } from \"./players/PlayerManager\"\nspawnPlayer()\n",
        )
        .expect("game file");

        let mut config = XLuauConfig::default();
        config.target = TargetKind::Roblox;

        let compiler = Compiler::new(temp.path().to_path_buf(), config).expect("compiler");
        compiler.build(&[]).expect("build");

        let output = fs::read_to_string(temp.path().join("out/server/Game.luau")).expect("output");
        assert!(output.contains("require(script.Parent.players.PlayerManager)"));
    }
}
