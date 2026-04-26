use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use crate::ast::{
    ExportKind, ExportStatement, ImportKind, ImportStatement, Program, StatementNode,
};
use crate::config::{TargetKind, XLuauConfig};
use crate::emitter::Emitter;
use crate::error::{Result, XLuauError};
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::source::{SourceFile, SourceKind};

#[derive(Debug)]
pub struct Resolver {
    project_root: PathBuf,
    config: XLuauConfig,
    next_temp_id: usize,
}

#[derive(Debug, Clone)]
pub struct ResolvedModule {
    pub chunks: Vec<String>,
    pub has_runtime_exports: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitState {
    Visiting,
    Visited,
}

#[derive(Debug, Clone)]
struct ModuleResolution {
    resolved_path: PathBuf,
    require_target: RequireTarget,
}

#[derive(Debug, Clone)]
enum RequireTarget {
    String(String),
    Expression(String),
}

impl RequireTarget {
    fn render(&self) -> String {
        match self {
            Self::String(path) => format!("\"{path}\""),
            Self::Expression(expr) => expr.clone(),
        }
    }
}

impl Resolver {
    pub fn new(project_root: PathBuf, config: XLuauConfig) -> Self {
        Self {
            project_root: normalize_absolute_path(project_root.as_path()),
            config,
            next_temp_id: 0,
        }
    }

    pub fn validate_entrypoints(&self, entries: &[PathBuf]) -> Result<()> {
        let mut states = HashMap::new();
        let mut stack = Vec::new();

        for entry in entries {
            self.visit_module(entry, &mut states, &mut stack)?;
        }

        Ok(())
    }

    pub fn resolve_program(&mut self, source: &SourceFile, program: &Program) -> Result<ResolvedModule> {
        let mut chunks = Vec::new();
        let mut has_runtime_exports = false;
        let emitter = Emitter::new();

        for statement in &program.statements {
            match &statement.node {
                StatementNode::Import(import) => {
                    if let Some(chunk) = self.resolve_import(source, import, statement.trailing.as_str())? {
                        chunks.push(chunk);
                    }
                }
                StatementNode::Export(export) => {
                    let (resolved, exported) =
                        self.resolve_export(source, export, statement.trailing.as_str(), &emitter)?;
                    if let Some(chunk) = resolved {
                        chunks.push(chunk);
                    }
                    has_runtime_exports |= exported;
                }
                _ => chunks.push(emitter.emit_single_statement(statement)),
            }
        }

        Ok(ResolvedModule {
            chunks,
            has_runtime_exports,
        })
    }

    fn resolve_import(
        &mut self,
        source: &SourceFile,
        import: &ImportStatement,
        trailing: &str,
    ) -> Result<Option<String>> {
        match &import.kind {
            ImportKind::SideEffect => {
                let target = self.resolve_module_specifier(source.path.as_path(), import.source.as_str())?;
                Ok(Some(format!("require({}){}", target.require_target.render(), trailing)))
            }
            ImportKind::TypeNamed { .. } => Ok(None),
            ImportKind::Value {
                default,
                namespace,
                named,
            } => {
                let target = self.resolve_module_specifier(source.path.as_path(), import.source.as_str())?;
                if default.is_some() && namespace.is_none() && named.is_empty() {
                    return Ok(Some(format!(
                        "local {} = require({}).__default{}",
                        default.as_deref().unwrap_or("_"),
                        target.require_target.render(),
                        trailing
                    )));
                }

                if default.is_none() && namespace.is_some() && named.is_empty() {
                    return Ok(Some(format!(
                        "local {} = require({}){}",
                        namespace.as_deref().unwrap_or("_"),
                        target.require_target.render(),
                        trailing
                    )));
                }

                if default.is_none() && namespace.is_none() && named.len() == 1 {
                    let specifier = &named[0];
                    return Ok(Some(format!(
                        "local {} = require({}).{}{}",
                        specifier.local,
                        target.require_target.render(),
                        specifier.imported,
                        trailing
                    )));
                }

                let temp = self.next_temp("import");
                let mut lines = vec![format!(
                    "local {} = require({})",
                    temp,
                    target.require_target.render()
                )];
                if let Some(default) = default {
                    lines.push(format!("local {} = {}.__default", default, temp));
                }
                if let Some(namespace) = namespace {
                    lines.push(format!("local {} = {}", namespace, temp));
                }
                for specifier in named {
                    lines.push(format!(
                        "local {} = {}.{}",
                        specifier.local, temp, specifier.imported
                    ));
                }
                Ok(Some(finalize_lines(lines, trailing)))
            }
        }
    }

    fn resolve_export(
        &mut self,
        source: &SourceFile,
        export: &ExportStatement,
        trailing: &str,
        emitter: &Emitter,
    ) -> Result<(Option<String>, bool)> {
        match &export.kind {
            ExportKind::Declaration(node) => {
                let declaration = emitter.emit_node(node);
                let export_names = exported_names_from_declaration(node);
                if export_names.is_empty() {
                    return Ok((Some(format!("{declaration}{trailing}")), false));
                }

                let mut lines = vec![declaration];
                for name in export_names {
                    lines.push(format!("_exports.{0} = {0}", name));
                }
                Ok((Some(finalize_lines(lines, trailing)), true))
            }
            ExportKind::Named {
                specifiers,
                source: reexport_source,
                is_type_only,
            } => {
                if *is_type_only {
                    return Ok((None, false));
                }

                if let Some(specifier_source) = reexport_source {
                    let resolved = self.resolve_module_specifier(
                        source.path.as_path(),
                        specifier_source.as_str(),
                    )?;
                    let temp = self.next_temp("reexport");
                    let mut lines = vec![format!(
                        "local {} = require({})",
                        temp,
                        resolved.require_target.render()
                    )];
                    for specifier in specifiers {
                        let rhs = if specifier.local == "default" {
                            format!("{temp}.__default")
                        } else {
                            format!("{temp}.{}", specifier.local)
                        };
                        lines.push(format!(
                            "_exports.{} = {}",
                            export_key(specifier.exported.as_str()),
                            rhs
                        ));
                    }
                    return Ok((Some(finalize_lines(lines, trailing)), true));
                }

                let lines = specifiers
                    .iter()
                    .map(|specifier| {
                        format!(
                            "_exports.{} = {}",
                            export_key(specifier.exported.as_str()),
                            specifier.local
                        )
                    })
                    .collect::<Vec<_>>();
                Ok((Some(finalize_lines(lines, trailing)), true))
            }
            ExportKind::All {
                source: reexport_source,
                is_type_only,
            } => {
                if *is_type_only {
                    return Ok((None, false));
                }

                let resolved = self.resolve_module_specifier(
                    source.path.as_path(),
                    reexport_source.as_str(),
                )?;
                let temp = self.next_temp("reexport");
                let key = self.next_temp("export_key");
                let value = self.next_temp("export_value");
                let lines = vec![
                    format!("local {} = require({})", temp, resolved.require_target.render()),
                    format!("for {}, {} in pairs({}) do", key, value, temp),
                    format!("    if {} ~= \"__default\" then", key),
                    format!("        _exports[{}] = {}", key, value),
                    "    end".to_owned(),
                    "end".to_owned(),
                ];
                Ok((Some(finalize_lines(lines, trailing)), true))
            }
            ExportKind::Default { expression } => Ok((
                Some(format!("_exports.__default = {}{}", expression.trim(), trailing)),
                true,
            )),
            ExportKind::TypeDeclaration(text) => Ok((Some(format!("export {}{}", text, trailing)), false)),
        }
    }

    fn visit_module(
        &self,
        path: &PathBuf,
        states: &mut HashMap<PathBuf, VisitState>,
        stack: &mut Vec<PathBuf>,
    ) -> Result<()> {
        let canonical = normalize_absolute_path(path);

        match states.get(&canonical) {
            Some(VisitState::Visited) => return Ok(()),
            Some(VisitState::Visiting) => {
                let cycle_start = stack.iter().position(|entry| *entry == canonical).unwrap_or(0);
                let mut cycle = stack[cycle_start..]
                    .iter()
                    .chain(std::iter::once(&canonical))
                    .map(|entry| self.relative_display(entry.as_path()))
                    .collect::<Vec<_>>();
                if cycle.is_empty() {
                    cycle.push(self.relative_display(canonical.as_path()));
                }
                return Err(XLuauError::Validation(format!(
                    "circular dependency detected: {}",
                    cycle.join(" -> ")
                )));
            }
            None => {}
        }

        states.insert(canonical.clone(), VisitState::Visiting);
        stack.push(canonical.clone());

        let source = SourceFile::load(canonical.clone())?;
        let mut diagnostics = Vec::new();
        let tokens = Lexer::new(&source).lex(&mut diagnostics);
        let program = Parser::new(&source, &tokens).parse(&mut diagnostics);

        if diagnostics.iter().any(crate::diagnostic::Diagnostic::is_error) {
            return Err(XLuauError::Validation(format!(
                "failed to parse {} while validating imports",
                self.relative_display(canonical.as_path())
            )));
        }

        for dependency in self.runtime_dependencies(&source, &program)? {
            self.visit_module(&dependency, states, stack)?;
        }

        stack.pop();
        states.insert(canonical, VisitState::Visited);
        Ok(())
    }

    fn runtime_dependencies(&self, source: &SourceFile, program: &Program) -> Result<Vec<PathBuf>> {
        let mut dependencies = Vec::new();

        for statement in &program.statements {
            match &statement.node {
                StatementNode::Import(import) => {
                    if !matches!(import.kind, ImportKind::TypeNamed { .. }) {
                        dependencies.push(
                            self.resolve_module_specifier(source.path.as_path(), import.source.as_str())?
                                .resolved_path,
                        );
                    }
                }
                StatementNode::Export(export) => match &export.kind {
                    ExportKind::Named {
                        source: Some(specifier),
                        is_type_only: false,
                        ..
                    }
                    | ExportKind::All {
                        source: specifier,
                        is_type_only: false,
                    } => dependencies.push(
                        self.resolve_module_specifier(source.path.as_path(), specifier.as_str())?
                            .resolved_path,
                    ),
                    _ => {}
                },
                _ => {}
            }
        }

        dependencies.sort();
        dependencies.dedup();
        Ok(dependencies)
    }

    fn resolve_module_specifier(&self, importer: &Path, specifier: &str) -> Result<ModuleResolution> {
        let resolved_path = self.resolve_source_path(importer, specifier)?;
        let require_target = match self.config.target {
            TargetKind::Filesystem | TargetKind::Custom => {
                RequireTarget::String(self.filesystem_require_path(importer, resolved_path.as_path())?)
            }
            TargetKind::Roblox => {
                RequireTarget::Expression(self.roblox_require_path(importer, resolved_path.as_path())?)
            }
        };

        Ok(ModuleResolution {
            resolved_path,
            require_target,
        })
    }

    fn resolve_source_path(&self, importer: &Path, specifier: &str) -> Result<PathBuf> {
        let base = if let Some(alias_path) = self.apply_path_alias(specifier) {
            self.project_root.join(alias_path)
        } else if specifier.starts_with("./") || specifier.starts_with("../") {
            normalize_absolute_path(importer)
                .parent()
                .unwrap_or(self.project_root.as_path())
                .join(specifier)
        } else if Path::new(specifier).is_absolute() {
            PathBuf::from(specifier)
        } else {
            self.project_root.join(&self.config.base_dir).join(specifier)
        };
        let base = normalize_absolute_path(base.as_path());

        if base.is_file() && self.is_supported_extension(base.as_path()) {
            return Ok(base);
        }

        for extension in &self.config.extensions {
            let candidate = with_extension(base.as_path(), extension);
            if candidate.is_file() {
                return Ok(normalize_absolute_path(candidate.as_path()));
            }
        }

        for index_name in &self.config.index_files {
            for extension in &self.config.extensions {
                let candidate = base.join(index_name).with_extension(extension.trim_start_matches('.'));
                if candidate.is_file() {
                    return Ok(normalize_absolute_path(candidate.as_path()));
                }
            }
        }

        Err(XLuauError::Validation(format!(
            "unable to resolve module specifier `{}` from {}",
            specifier,
            self.relative_display(importer)
        )))
    }

    fn apply_path_alias(&self, specifier: &str) -> Option<PathBuf> {
        let mut best_match = None::<(usize, PathBuf)>;

        for (alias, target) in &self.config.paths {
            if let Some((prefix, suffix)) = alias.split_once('*') {
                if specifier.starts_with(prefix) && specifier.ends_with(suffix) {
                    let captured = &specifier[prefix.len()..specifier.len() - suffix.len()];
                    let replaced = target.replace('*', captured);
                    let score = prefix.len() + suffix.len();
                    if best_match
                        .as_ref()
                        .map(|(best_score, _)| score > *best_score)
                        .unwrap_or(true)
                    {
                        best_match = Some((score, PathBuf::from(replaced)));
                    }
                }
            } else if specifier == alias {
                let score = alias.len();
                if best_match
                    .as_ref()
                    .map(|(best_score, _)| score > *best_score)
                    .unwrap_or(true)
                {
                    best_match = Some((score, PathBuf::from(target)));
                }
            }
        }

        best_match.map(|(_, path)| path)
    }

    fn filesystem_require_path(&self, importer: &Path, resolved: &Path) -> Result<String> {
        let importer_output = self.output_path(importer)?;
        let target_output = self.output_path(resolved)?;
        let importer_dir = importer_output
            .parent()
            .ok_or_else(|| XLuauError::Validation("importer output path has no parent".to_owned()))?;
        let target_without_extension = without_extension(target_output.as_path());
        let relative = relative_path(importer_dir, target_without_extension.as_path());
        let normalized = normalize_relative_path(relative.as_path());
        Ok(if normalized.starts_with('.') {
            normalized
        } else {
            format!("./{normalized}")
        })
    }

    fn roblox_require_path(&self, importer: &Path, resolved: &Path) -> Result<String> {
        let importer_module = self.module_relative_path(importer)?;
        let target_module = self.module_relative_path(resolved)?;
        let importer_dir = importer_module.parent().unwrap_or(Path::new(""));
        let relative = relative_path(importer_dir, target_module.as_path());

        let mut expression = String::from("script.Parent");
        for component in relative.components() {
            match component {
                Component::ParentDir => expression.push_str(".Parent"),
                Component::Normal(segment) => {
                    let segment = segment.to_string_lossy();
                    if is_luau_identifier(segment.as_ref()) {
                        expression.push('.');
                        expression.push_str(segment.as_ref());
                    } else {
                        expression.push_str(format!("[\"{}\"]", segment).as_str());
                    }
                }
                Component::CurDir => {}
                _ => {}
            }
        }

        Ok(expression)
    }

    fn module_relative_path(&self, source_path: &Path) -> Result<PathBuf> {
        let normalized = normalize_absolute_path(source_path);
        let base_dir = self.project_root.join(&self.config.base_dir);
        let relative = normalized
            .strip_prefix(&base_dir)
            .or_else(|_| normalized.strip_prefix(&self.project_root))
            .map_err(|_| {
                XLuauError::Validation(format!(
                    "unable to compute module-relative path for {}",
                    source_path.display()
                ))
            })?;
        Ok(without_extension(relative))
    }

    fn output_path(&self, source_path: &Path) -> Result<PathBuf> {
        let normalized = normalize_absolute_path(source_path);
        let base_dir = self.project_root.join(&self.config.base_dir);
        let relative = normalized
            .strip_prefix(&base_dir)
            .or_else(|_| normalized.strip_prefix(&self.project_root))
            .map_err(|_| {
                XLuauError::Validation(format!(
                    "unable to determine output path for {}",
                    source_path.display()
                ))
            })?;

        let mut output = self.project_root.join(&self.config.out_dir).join(relative);
        if matches!(SourceKind::from_path(source_path), Ok(SourceKind::XLuau)) {
            output.set_extension("luau");
        }

        Ok(output)
    }

    fn is_supported_extension(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| {
                self.config
                    .extensions
                    .iter()
                    .any(|configured| configured.trim_start_matches('.') == ext)
            })
            .unwrap_or(false)
    }

    fn next_temp(&mut self, prefix: &str) -> String {
        self.next_temp_id += 1;
        format!("_xluau_{}_{}", prefix, self.next_temp_id)
    }

    fn relative_display(&self, path: &Path) -> String {
        let normalized = normalize_absolute_path(path);
        normalized
            .strip_prefix(&self.project_root)
            .unwrap_or(normalized.as_path())
            .display()
            .to_string()
            .replace('\\', "/")
    }
}

fn finalize_lines(lines: Vec<String>, trailing: &str) -> String {
    format!("{}{}", lines.join("\n"), trailing)
}

fn exported_names_from_declaration(node: &StatementNode) -> Vec<String> {
    match node {
        StatementNode::Function(function) => function_name(function.header_prefix.as_str())
            .into_iter()
            .collect(),
        StatementNode::Local(local) => declared_binding_names(local.bindings.as_str()),
        _ => Vec::new(),
    }
}

fn function_name(prefix: &str) -> Option<String> {
    let prefix = prefix.trim_start();
    let prefix = prefix
        .strip_prefix("local function")
        .or_else(|| prefix.strip_prefix("function"))?
        .trim_start();
    let name = prefix
        .chars()
        .take_while(|ch| *ch == '_' || ch.is_ascii_alphanumeric())
        .collect::<String>();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn export_key(name: &str) -> &str {
    if name == "default" {
        "__default"
    } else {
        name
    }
}

fn declared_binding_names(text: &str) -> Vec<String> {
    split_top_level(text, ',')
        .into_iter()
        .filter_map(|binding| {
            let binding = binding.trim();
            if binding.is_empty() || binding.starts_with('{') || binding.starts_with('[') {
                return None;
            }
            let name = binding
                .split(':')
                .next()
                .unwrap_or(binding)
                .trim()
                .to_owned();
            if name.is_empty() {
                None
            } else {
                Some(name)
            }
        })
        .collect()
}

fn split_top_level(text: &str, separator: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut paren = 0usize;
    let mut brace = 0usize;
    let mut bracket = 0usize;

    for ch in text.chars() {
        match ch {
            '(' => paren += 1,
            ')' => paren = paren.saturating_sub(1),
            '{' => brace += 1,
            '}' => brace = brace.saturating_sub(1),
            '[' => bracket += 1,
            ']' => bracket = bracket.saturating_sub(1),
            _ => {}
        }

        if ch == separator && paren == 0 && brace == 0 && bracket == 0 {
            parts.push(current.trim().to_owned());
            current.clear();
        } else {
            current.push(ch);
        }
    }

    if !current.trim().is_empty() {
        parts.push(current.trim().to_owned());
    }

    parts
}

fn with_extension(path: &Path, extension: &str) -> PathBuf {
    path.with_extension(extension.trim_start_matches('.'))
}

fn without_extension(path: &Path) -> PathBuf {
    let mut output = path.to_path_buf();
    output.set_extension("");
    if output.extension().is_some_and(|ext| ext.is_empty()) {
        output.set_extension("");
    }
    if path.extension().is_some() {
        let stem = path.file_stem().unwrap_or_default();
        output.set_file_name(stem);
    }
    output
}

fn normalize_relative_path(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    if normalized.is_empty() {
        ".".to_owned()
    } else {
        normalized
    }
}

fn normalize_absolute_path(path: &Path) -> PathBuf {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let text = resolved.to_string_lossy();
    if let Some(stripped) = text.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        resolved
    }
}

fn relative_path(from: &Path, to: &Path) -> PathBuf {
    let from_components = from
        .components()
        .filter_map(normal_component)
        .collect::<Vec<_>>();
    let to_components = to
        .components()
        .filter_map(normal_component)
        .collect::<Vec<_>>();

    let mut shared = 0usize;
    while shared < from_components.len()
        && shared < to_components.len()
        && from_components[shared] == to_components[shared]
    {
        shared += 1;
    }

    let mut relative = PathBuf::new();
    for _ in shared..from_components.len() {
        relative.push("..");
    }
    for component in &to_components[shared..] {
        relative.push(component);
    }

    if relative.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        relative
    }
}

fn normal_component(component: Component<'_>) -> Option<String> {
    match component {
        Component::Normal(part) => Some(part.to_string_lossy().to_string()),
        _ => None,
    }
}

fn is_luau_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    use super::Resolver;
    use crate::config::{TargetKind, XLuauConfig};
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::source::{SourceFile, SourceKind};

    #[test]
    fn resolver_emits_filesystem_imports_and_exports() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("src")).expect("src dir");
        fs::write(temp.path().join("src/mod.xl"), "export const value = 1\n").expect("mod");
        let source = SourceFile::virtual_file(
            temp.path().join("src/main.xl"),
            SourceKind::XLuau,
            "import { value as answer } from \"./mod\"\nexport { answer }\n".to_owned(),
        );
        let tokens = Lexer::new(&source).lex(&mut Vec::new());
        let program = Parser::new(&source, &tokens).parse(&mut Vec::new());

        let mut resolver = Resolver::new(temp.path().to_path_buf(), XLuauConfig::default());
        let resolved = resolver.resolve_program(&source, &program).expect("resolved");

        assert!(resolved.has_runtime_exports);
        assert!(resolved.chunks[0].contains("require(\"./mod\")"));
        assert!(resolved.chunks[1].contains("_exports.answer = answer"));
    }

    #[test]
    fn resolver_supports_aliases_and_barrel_files() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("src/shared/utils")).expect("utils dir");
        fs::write(temp.path().join("src/shared/utils/init.xl"), "return {}\n").expect("init");

        let mut config = XLuauConfig::default();
        config.paths.insert("@utils".to_owned(), "./src/shared/utils".to_owned());

        let resolver = Resolver::new(temp.path().to_path_buf(), config);
        let resolution = resolver
            .resolve_module_specifier(&temp.path().join("src/main.xl"), "@utils")
            .expect("resolution");

        assert!(resolution.resolved_path.ends_with(Path::new("src/shared/utils/init.xl")));
    }

    #[test]
    fn resolver_emits_roblox_paths() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("src/server/players")).expect("players dir");
        fs::write(temp.path().join("src/server/players/PlayerManager.xl"), "return {}\n")
            .expect("player manager");

        let mut config = XLuauConfig::default();
        config.target = TargetKind::Roblox;
        let resolver = Resolver::new(temp.path().to_path_buf(), config);

        let resolution = resolver
            .resolve_module_specifier(
                &temp.path().join("src/server/Game.xl"),
                "./players/PlayerManager",
            )
            .expect("resolution");

        assert_eq!(
            resolution.require_target.render(),
            "script.Parent.players.PlayerManager"
        );
    }

    #[test]
    fn resolver_detects_circular_dependencies() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("src")).expect("src dir");
        fs::write(temp.path().join("src/a.xl"), "import \"./b\"\n").expect("a");
        fs::write(temp.path().join("src/b.xl"), "import \"./a\"\n").expect("b");

        let resolver = Resolver::new(temp.path().to_path_buf(), XLuauConfig::default());
        let error = resolver
            .validate_entrypoints(&[temp.path().join("src/a.xl")])
            .expect_err("cycle error");

        assert!(error.to_string().contains("circular dependency detected"));
    }
}
