use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write;
use std::path::PathBuf;

use crate::ast::{
    ExportKind, FunctionStatement, LocalKeyword, LocalStatement, Program, Statement, StatementKind,
    StatementNode,
};
use crate::config::{LuauTarget, XLuauConfig};
use crate::diagnostic::Diagnostic;
use crate::emitter::Emitter;
use crate::lexer::Lexer;
use crate::source::{SourceFile, SourceKind};

#[derive(Debug, Clone)]
pub struct PhaseFourTransformer {
    config: XLuauConfig,
    functions: HashMap<String, FunctionSignature>,
    simple_function_names: HashMap<String, String>,
    types: HashMap<String, TypeDefinition>,
}

#[derive(Debug, Clone)]
struct FunctionSignature {
    name: String,
    generics: Vec<GenericParam>,
    params: Vec<FunctionParam>,
    return_type: Option<String>,
}

#[derive(Debug, Clone)]
struct GenericParam {
    name: String,
    constraint: Option<String>,
    default: Option<String>,
}

#[derive(Debug, Clone)]
struct FunctionParam {
    name: String,
    ty: Option<String>,
}

#[derive(Debug, Clone)]
struct TypeDefinition {
    name: String,
    object: Option<ObjectType>,
}

#[derive(Debug, Clone)]
struct ObjectType {
    fields: Vec<ObjectField>,
}

#[derive(Debug, Clone)]
struct ObjectField {
    name: String,
    ty: String,
    readonly: bool,
    optional: bool,
}

#[derive(Debug, Clone)]
struct EnumMember {
    name: String,
    value: String,
    explicit: bool,
}

#[derive(Debug, Clone)]
struct EnumMethod {
    name: String,
    text: String,
}

#[derive(Debug, Clone)]
struct EnumDefinition {
    name: String,
    backing: Option<String>,
    members: Vec<EnumMember>,
    methods: Vec<EnumMethod>,
}

impl PhaseFourTransformer {
    pub fn new(config: XLuauConfig) -> Self {
        Self {
            config,
            functions: HashMap::new(),
            simple_function_names: HashMap::new(),
            types: HashMap::new(),
        }
    }

    pub fn transform_program(
        mut self,
        source: &SourceFile,
        program: &Program,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        self.collect_definitions(program);
        self.transform_statement_list(source.path.clone(), &program.statements, diagnostics)
    }

    fn collect_definitions(&mut self, program: &Program) {
        for statement in &program.statements {
            self.collect_from_statement(statement);
        }
    }

    fn collect_from_statement(&mut self, statement: &Statement) {
        match &statement.node {
            StatementNode::Function(function) => {
                if let Some(signature) = parse_function_signature(function) {
                    self.register_function(signature);
                }
                for nested in &function.body {
                    self.collect_from_statement(nested);
                }
            }
            StatementNode::If(if_stmt) => {
                for clause in &if_stmt.clauses {
                    for nested in &clause.body {
                        self.collect_from_statement(nested);
                    }
                }
                if let Some(body) = &if_stmt.else_body {
                    for nested in body {
                        self.collect_from_statement(nested);
                    }
                }
            }
            StatementNode::While(while_stmt) => {
                for nested in &while_stmt.body {
                    self.collect_from_statement(nested);
                }
            }
            StatementNode::Repeat(repeat_stmt) => {
                for nested in &repeat_stmt.body {
                    self.collect_from_statement(nested);
                }
            }
            StatementNode::For(for_stmt) => {
                for nested in &for_stmt.body {
                    self.collect_from_statement(nested);
                }
            }
            StatementNode::Do(block) => {
                for nested in &block.body {
                    self.collect_from_statement(nested);
                }
            }
            StatementNode::Switch(switch) => {
                for section in &switch.sections {
                    for nested in &section.body {
                        self.collect_from_statement(nested);
                    }
                }
            }
            StatementNode::Export(export) => match &export.kind {
                ExportKind::Declaration(node) => {
                    self.collect_from_exported_node(node);
                }
                ExportKind::TypeDeclaration(text) => {
                    if let Some(definition) = parse_type_definition(text) {
                        self.types.insert(definition.name.clone(), definition);
                    }
                }
                _ => {}
            },
            StatementNode::Text(text) if statement.kind == StatementKind::TypeDeclaration => {
                if let Some(definition) = parse_type_definition(text) {
                    self.types.insert(definition.name.clone(), definition);
                }
            }
            _ => {}
        }
    }

    fn collect_from_exported_node(&mut self, node: &StatementNode) {
        match node {
            StatementNode::Function(function) => {
                if let Some(signature) = parse_function_signature(function) {
                    self.register_function(signature);
                }
            }
            StatementNode::Text(text) => {
                if let Some(definition) = parse_type_definition(text) {
                    self.types.insert(definition.name.clone(), definition);
                }
            }
            _ => {}
        }
    }

    fn register_function(&mut self, signature: FunctionSignature) {
        let simple_name = tail_callable_name(signature.name.as_str()).to_owned();
        self.simple_function_names
            .entry(simple_name)
            .or_insert_with(|| signature.name.clone());
        self.functions.insert(signature.name.clone(), signature);
    }

    fn transform_statement_list(
        &self,
        path: PathBuf,
        statements: &[Statement],
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let mut output = String::new();
        for statement in statements {
            output.push_str(
                self.transform_statement(path.clone(), statement, diagnostics)
                    .as_str(),
            );
        }
        output
    }

    fn transform_statement(
        &self,
        path: PathBuf,
        statement: &Statement,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let mut text = match &statement.node {
            StatementNode::Trivia(text) => text.clone(),
            StatementNode::Import(_) => Emitter::new().emit_single_statement(statement),
            StatementNode::Export(export) => self.transform_export(path, export, diagnostics),
            StatementNode::Local(local) => self.transform_local_statement(local, diagnostics),
            StatementNode::Function(function) => {
                self.transform_function_statement(path, function, diagnostics)
            }
            StatementNode::Return(ret) => match &ret.values {
                Some(values) => format!(
                    "return {}",
                    self.transform_expression_text(values.as_str(), diagnostics)
                ),
                None => "return".to_owned(),
            },
            StatementNode::If(if_stmt) => {
                let mut output = String::new();
                for clause in &if_stmt.clauses {
                    let keyword = match clause.keyword {
                        crate::ast::ConditionalKeyword::If => "if",
                        crate::ast::ConditionalKeyword::ElseIf => "elseif",
                    };
                    writeln!(
                        output,
                        "{} {} then",
                        keyword,
                        self.transform_expression_text(clause.condition.as_str(), diagnostics)
                    )
                    .ok();
                    output.push_str(
                        self.transform_statement_list(path.clone(), &clause.body, diagnostics)
                            .as_str(),
                    );
                }
                if let Some(body) = &if_stmt.else_body {
                    output.push_str("else\n");
                    output.push_str(
                        self.transform_statement_list(path.clone(), body, diagnostics)
                            .as_str(),
                    );
                }
                output.push_str("end");
                output
            }
            StatementNode::While(while_stmt) => format!(
                "while {} do\n{}end",
                self.transform_expression_text(while_stmt.condition.as_str(), diagnostics),
                self.transform_statement_list(path.clone(), &while_stmt.body, diagnostics)
            ),
            StatementNode::Repeat(repeat_stmt) => format!(
                "repeat\n{}until {}",
                self.transform_statement_list(path.clone(), &repeat_stmt.body, diagnostics),
                self.transform_expression_text(repeat_stmt.condition.as_str(), diagnostics)
            ),
            StatementNode::For(for_stmt) => format!(
                "for {} do\n{}end",
                self.transform_expression_text(for_stmt.head.as_str(), diagnostics),
                self.transform_statement_list(path.clone(), &for_stmt.body, diagnostics)
            ),
            StatementNode::Do(block) => format!(
                "do\n{}end",
                self.transform_statement_list(path.clone(), &block.body, diagnostics)
            ),
            StatementNode::Switch(switch) => {
                let mut output = format!(
                    "switch {}\n",
                    self.transform_expression_text(switch.expression.as_str(), diagnostics)
                );
                for section in &switch.sections {
                    match &section.label {
                        crate::ast::SwitchLabel::Case(values) => {
                            output.push_str(
                                format!(
                                    "case {}:\n",
                                    values
                                        .iter()
                                        .map(|value| {
                                            self.transform_expression_text(value.as_str(), diagnostics)
                                        })
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                )
                                .as_str(),
                            );
                        }
                        crate::ast::SwitchLabel::Default => output.push_str("default:\n"),
                    }
                    output.push_str(
                        self.transform_statement_list(path.clone(), &section.body, diagnostics)
                            .as_str(),
                    );
                }
                output.push_str("end");
                output
            }
            StatementNode::Text(text) => self.transform_text_statement(statement.kind, text, diagnostics),
        };
        text.push_str(statement.trailing.as_str());
        text
    }

    fn transform_export(
        &self,
        path: PathBuf,
        export: &crate::ast::ExportStatement,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        match &export.kind {
            ExportKind::Declaration(node) => match node.as_ref() {
                StatementNode::Function(function) => {
                    format!(
                        "export {}",
                        self.transform_function_statement(path, function, diagnostics)
                    )
                }
                StatementNode::Local(local) => {
                    format!("export {}", self.transform_local_statement(local, diagnostics))
                }
                StatementNode::Text(text) if text.trim_start().starts_with("enum ") => {
                    self.transform_enum_statement(text, true, diagnostics)
                }
                StatementNode::Text(text) if text.trim_start().starts_with("type ") => {
                    format!(
                        "export {}",
                        self.transform_type_declaration_text(text, diagnostics)
                    )
                }
                other => {
                    let statement = Statement {
                        kind: StatementKind::ExportDeclaration,
                        node: other.clone(),
                        trailing: String::new(),
                        span: statement_span_placeholder(),
                    };
                    format!("export {}", Emitter::new().emit_single_statement(&statement))
                }
            },
            ExportKind::Named {
                specifiers,
                source,
                is_type_only,
            } => {
                let mut output = format!(
                    "export{} {{ {} }}",
                    if *is_type_only { " type" } else { "" },
                    specifiers
                        .iter()
                        .map(|specifier| {
                            if specifier.local == specifier.exported {
                                specifier.local.clone()
                            } else {
                                format!("{} as {}", specifier.local, specifier.exported)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                if let Some(source) = source {
                    output.push_str(format!(" from \"{}\"", source).as_str());
                }
                output
            }
            ExportKind::All {
                source,
                is_type_only,
            } => format!(
                "export{} * from \"{}\"",
                if *is_type_only { " type" } else { "" },
                source
            ),
            ExportKind::Default { expression } => format!(
                "export default {}",
                self.transform_expression_text(expression.as_str(), diagnostics)
            ),
            ExportKind::TypeDeclaration(text) => format!(
                "export {}",
                self.transform_type_declaration_text(text, diagnostics)
            ),
        }
    }

    fn transform_local_statement(
        &self,
        local: &LocalStatement,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let keyword = match local.keyword {
            LocalKeyword::Local => "local",
            LocalKeyword::Let => "let",
            LocalKeyword::Const => "const",
        };
        let bindings = transform_binding_list(local.bindings.as_str(), self, diagnostics);
        match &local.value {
            Some(value) => format!(
                "{} {} = {}",
                keyword,
                bindings,
                self.transform_expression_text(value.as_str(), diagnostics)
            ),
            None => format!("{} {}", keyword, bindings),
        }
    }

    fn transform_function_statement(
        &self,
        path: PathBuf,
        function: &FunctionStatement,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let signature = parse_function_signature(function);
        let generic_map = signature
            .as_ref()
            .map(build_constraint_map)
            .unwrap_or_default();

        let base_prefix = strip_function_generics(function.header_prefix.as_str());
        let params = split_top_level_commas(function.params.as_str())
            .into_iter()
            .map(|param| transform_function_param(param.as_str(), self, &generic_map))
            .collect::<Vec<_>>()
            .join(", ");
        let return_type = parse_return_type(function.header_suffix.as_str())
            .map(|ret| self.transform_type_expression(ret.as_str(), diagnostics));
        let body = self.transform_statement_list(path.clone(), &function.body, diagnostics);

        if let Some(signature) = signature {
            for param in &signature.params {
                if let Some(generic) = signature
                    .generics
                    .iter()
                    .find(|generic| param.ty.as_deref() == Some(generic.name.as_str()) && generic.constraint.is_some())
                {
                    if function_body_mentions_constrained_member(&function.body, param.name.as_str()) {
                        diagnostics.push(Diagnostic::warning(
                            Some(&path),
                            None,
                            format!(
                                "constraint methods on `{}` may require an explicit cast to `{}` inside the function body",
                                param.name,
                                generic.constraint.as_deref().unwrap_or("constraint")
                            ),
                        ));
                    }
                }
            }
        }

        let mut output = format!("{}{})", base_prefix.trim_end(), params);
        if let Some(return_type) = return_type {
            output.push_str(format!(": {}", return_type.trim()).as_str());
        }
        output.push('\n');
        output.push_str(body.as_str());
        output.push_str("end");
        output
    }

    fn transform_text_statement(
        &self,
        kind: StatementKind,
        text: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let trimmed = text.trim_start();
        if trimmed.starts_with("enum ") {
            return self.transform_enum_statement(text, false, diagnostics);
        }
        if kind == StatementKind::TypeDeclaration || trimmed.starts_with("type ") {
            return self.transform_type_declaration_text(text, diagnostics);
        }
        self.transform_expression_text(text, diagnostics)
    }

    fn transform_type_declaration_text(
        &self,
        text: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let trimmed = text.trim();
        let Some((head, body)) = split_top_level_once(trimmed, '=') else {
            return trimmed.to_owned();
        };
        let transformed_body = self.transform_type_expression(body.trim(), diagnostics);
        format!("{} = {}", head.trim(), transformed_body)
    }

    fn transform_enum_statement(
        &self,
        text: &str,
        exported: bool,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let Some(definition) = parse_enum_definition(text) else {
            return text.to_owned();
        };
        let mut output = String::new();
        let type_prefix = if exported { "export " } else { "" };
        writeln!(
            output,
            "{}type {} = {}",
            type_prefix,
            definition.name,
            enum_union_type(&definition)
        )
        .ok();
        writeln!(output, "local {} = {{", definition.name).ok();
        for member in &definition.members {
            writeln!(
                output,
                "    {} = {} :: {},",
                member.name, member.value, definition.name
            )
            .ok();
        }
        for method in &definition.methods {
            writeln!(output, "    {} = {},", method.name, method.text.trim()).ok();
        }
        output.push_str("}\n");
        output.push_str(format!("table.freeze({})", definition.name).as_str());
        if exported {
            output.push('\n');
            output.push_str(format!("export {{ {} }}", definition.name).as_str());
        }
        let _ = diagnostics;
        output
    }

    fn transform_expression_text(
        &self,
        text: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let frozen = transform_freeze_literals(text);
        self.transform_explicit_type_calls(frozen.as_str(), diagnostics)
    }

    fn transform_explicit_type_calls(
        &self,
        text: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let mut output = String::new();
        let chars: Vec<char> = text.chars().collect();
        let mut cursor = 0usize;

        while cursor < chars.len() {
            if chars[cursor] != '<' {
                cursor += 1;
                continue;
            }

            let Some(target_start) = find_call_target_start(&chars, cursor) else {
                cursor += 1;
                continue;
            };
            let target_text = chars[target_start..cursor]
                .iter()
                .collect::<String>()
                .trim()
                .to_owned();
            if target_text.is_empty() {
                cursor += 1;
                continue;
            }

            let Some(type_end) = find_matching_angle(&chars, cursor) else {
                cursor += 1;
                continue;
            };
            let mut after_type = type_end + 1;
            while after_type < chars.len() && chars[after_type].is_whitespace() {
                after_type += 1;
            }
            if after_type >= chars.len() || chars[after_type] != '(' {
                cursor += 1;
                continue;
            }
            let Some(call_end) = find_matching_paren_chars(&chars, after_type) else {
                cursor += 1;
                continue;
            };

            output.push_str(chars[..target_start].iter().collect::<String>().as_str());
            let type_args = chars[cursor + 1..type_end].iter().collect::<String>();
            let args_text = chars[after_type + 1..call_end].iter().collect::<String>();
            output.push_str(
                self.rewrite_typed_call(
                    target_text.as_str(),
                    type_args.as_str(),
                    args_text.as_str(),
                    diagnostics,
                )
                .as_str(),
            );
            output.push_str(chars[call_end + 1..].iter().collect::<String>().as_str());
            return output;
        }

        text.to_owned()
    }

    fn rewrite_typed_call(
        &self,
        target: &str,
        type_args: &str,
        args_text: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let Some(signature) = self.lookup_function_signature(target) else {
            diagnostics.push(Diagnostic::warning(
                None,
                None,
                format!(
                    "could not resolve explicit type arguments for `{}`; erasing them in output",
                    target
                ),
            ));
            return format!("{}({})", strip_method_type_args_target(target), args_text);
        };

        let mut concrete_types = split_top_level_angles(type_args, ',')
            .into_iter()
            .map(|value| self.transform_type_expression(value.trim(), diagnostics))
            .collect::<Vec<_>>();
        let explicit_type_count = concrete_types.len();

        if concrete_types.len() > signature.generics.len() {
            concrete_types.truncate(signature.generics.len());
        }

        while concrete_types.len() < signature.generics.len() {
            let Some(default) = signature.generics[concrete_types.len()].default.clone() else {
                break;
            };
            let resolved_default = substitute_type(
                default.as_str(),
                &generic_assignment_map(&signature.generics, &concrete_types),
            );
            concrete_types.push(self.transform_type_expression(resolved_default.as_str(), diagnostics));
        }

        let substitution = generic_assignment_map(&signature.generics, &concrete_types);
        let param_types = signature
            .params
            .iter()
            .map(|param| {
                param.ty
                    .as_ref()
                    .map(|ty| self.transform_type_expression(substitute_type(ty.as_str(), &substitution).as_str(), diagnostics))
            })
            .collect::<Vec<_>>();
        let return_type = signature
            .return_type
            .as_ref()
            .map(|ty| self.transform_type_expression(substitute_type(ty.as_str(), &substitution).as_str(), diagnostics))
            .unwrap_or_else(|| "any".to_owned());
        let args = split_top_level_angles(args_text, ',');

        let covered_generics = signature
            .params
            .iter()
            .filter_map(|param| param.ty.as_ref())
            .flat_map(|ty| signature.generics.iter().filter(move |generic| contains_word(ty, generic.name.as_str())).map(|generic| generic.name.clone()))
            .collect::<HashSet<_>>();
        let all_generics_covered = signature
            .generics
            .iter()
            .take(explicit_type_count)
            .all(|generic| covered_generics.contains(generic.name.as_str()));

        if all_generics_covered && !args.is_empty() {
            let casted_args = args
                .iter()
                .enumerate()
                .map(|(index, arg)| {
                    if let Some(Some(param_ty)) = param_types.get(index) {
                        format!("({} :: {})", arg.trim(), param_ty)
                    } else {
                        arg.trim().to_owned()
                    }
                })
                .collect::<Vec<_>>();
            return format!("{}({})", strip_method_type_args_target(target), casted_args.join(", "));
        }

        if target.contains(':') {
            let (base, method) = split_method_target(target);
            let param_sig = param_types
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            let signature_text = if param_sig.is_empty() {
                format!("(typeof({})) -> {}", base, return_type)
            } else {
                format!("(typeof({}), {}) -> {}", base, param_sig.join(", "), return_type)
            };
            let args_joined = if args_text.trim().is_empty() {
                base.to_owned()
            } else {
                format!("{}, {}", base, args_text.trim())
            };
            return format!(
                "(({}.{} ) :: {})({})",
                base, method, signature_text, args_joined
            )
            .replace(". ", ".");
        }

        let param_sig = param_types.into_iter().flatten().collect::<Vec<_>>();
        let signature_text = format!("({}) -> {}", param_sig.join(", "), return_type);
        format!("(({}) :: {})({})", target, signature_text, args_text.trim())
    }

    fn transform_type_expression(
        &self,
        text: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return String::new();
        }

        if trimmed.starts_with('{') && trimmed.ends_with('}') {
            return self.transform_object_type_literal(trimmed, diagnostics);
        }

        if let Some(expanded) = self.expand_builtin_utility(trimmed, diagnostics) {
            return expanded;
        }

        transform_nested_type_utilities(trimmed, self, diagnostics)
    }

    fn transform_object_type_literal(
        &self,
        text: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let inner = &text[1..text.len() - 1];
        let fields = split_top_level_angles(inner, ',')
            .into_iter()
            .filter_map(|entry| parse_object_field(entry.as_str()))
            .map(|field| render_object_field(field, self, diagnostics))
            .collect::<Vec<_>>();
        format!("{{ {} }}", fields.join(", "))
    }

    fn expand_builtin_utility(
        &self,
        text: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Option<String> {
        let (name, args) = parse_type_application(text)?;
        let args = split_top_level_angles(args.as_str(), ',')
            .into_iter()
            .map(|arg| arg.trim().to_owned())
            .collect::<Vec<_>>();

        Some(match name.as_str() {
            "Partial" if args.len() == 1 => self.expand_object_utility(args[0].as_str(), UtilityMode::Partial, diagnostics)?,
            "Required" if args.len() == 1 => self.expand_object_utility(args[0].as_str(), UtilityMode::Required, diagnostics)?,
            "Readonly" if args.len() == 1 => self.expand_object_utility(args[0].as_str(), UtilityMode::Readonly, diagnostics)?,
            "Pick" if args.len() == 2 => self.expand_pick_omit(args[0].as_str(), args[1].as_str(), true, diagnostics)?,
            "Omit" if args.len() == 2 => self.expand_pick_omit(args[0].as_str(), args[1].as_str(), false, diagnostics)?,
            "Record" if args.len() == 2 => format!(
                "{{[{}]: {}}}",
                self.transform_type_expression(args[0].as_str(), diagnostics),
                self.transform_type_expression(args[1].as_str(), diagnostics)
            ),
            "Exclude" if args.len() == 2 => subtract_union(args[0].as_str(), args[1].as_str()),
            "Extract" if args.len() == 2 => intersect_union(args[0].as_str(), args[1].as_str()),
            "ReturnType" if args.len() == 1 => self.resolve_return_type(args[0].as_str(), diagnostics),
            "Parameters" if args.len() == 1 => self.resolve_parameters_type(args[0].as_str(), diagnostics),
            "Awaited" if args.len() == 1 => self.resolve_awaited_type(args[0].as_str(), diagnostics),
            _ => return None,
        })
    }

    fn expand_object_utility(
        &self,
        source: &str,
        mode: UtilityMode,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Option<String> {
        let object = self.lookup_object_type(source)?;
        Some(render_object_type_with_mode(
            &object,
            mode,
            self,
            diagnostics,
            &self.config,
        ))
    }

    fn expand_pick_omit(
        &self,
        source: &str,
        keys: &str,
        pick: bool,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Option<String> {
        let object = self.lookup_object_type(source)?;
        let selected = keys_to_set(keys);
        let fields = object
            .fields
            .iter()
            .filter(|field| selected.contains(field.name.as_str()) == pick)
            .cloned()
            .collect::<Vec<_>>();
        Some(render_object_type_with_mode(
            &ObjectType { fields },
            UtilityMode::Identity,
            self,
            diagnostics,
            &self.config,
        ))
    }

    fn resolve_return_type(&self, source: &str, diagnostics: &mut Vec<Diagnostic>) -> String {
        let Some(name) = parse_typeof_target(source) else {
            return "any".to_owned();
        };
        self.lookup_function_signature(name.as_str())
            .and_then(|signature| signature.return_type.clone())
            .map(|ret| self.transform_type_expression(ret.as_str(), diagnostics))
            .unwrap_or_else(|| "any".to_owned())
    }

    fn resolve_parameters_type(&self, source: &str, diagnostics: &mut Vec<Diagnostic>) -> String {
        let Some(name) = parse_typeof_target(source) else {
            return "{}".to_owned();
        };
        let params = self
            .lookup_function_signature(name.as_str())
            .map(|signature| {
                signature
                    .params
                    .iter()
                    .filter_map(|param| param.ty.as_ref())
                    .map(|ty| self.transform_type_expression(ty.as_str(), diagnostics))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        format!("{{{}}}", params.join(", "))
    }

    fn resolve_awaited_type(&self, source: &str, diagnostics: &mut Vec<Diagnostic>) -> String {
        let inner = self.transform_type_expression(source, diagnostics);
        if let Some((name, args)) = parse_type_application(inner.as_str()) {
            if matches!(name.as_str(), "_XLPromise" | "Promise" | "PromiseLike") {
                return split_top_level_angles(args.as_str(), ',')
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "any".to_owned());
            }
        }

        if inner.starts_with("typeof(") && inner.ends_with(')') {
            let target = &inner["typeof(".len()..inner.len() - 1];
            if target.contains('(') {
                let call_target = target.split('(').next().unwrap_or(target).trim();
                return self
                    .lookup_function_signature(call_target)
                    .and_then(|signature| signature.return_type.clone())
                    .map(|ty| self.resolve_awaited_type(ty.as_str(), diagnostics))
                    .unwrap_or(inner);
            }
        }

        inner
    }

    fn lookup_function_signature(&self, target: &str) -> Option<&FunctionSignature> {
        self.functions
            .get(target)
            .or_else(|| self.functions.get(target.replace(':', ".").as_str()))
            .or_else(|| self.simple_function_names.get(tail_callable_name(target)).and_then(|name| self.functions.get(name)))
    }

    fn lookup_object_type(&self, source: &str) -> Option<ObjectType> {
        if source.trim().starts_with('{') && source.trim().ends_with('}') {
            let fields = split_top_level_angles(&source.trim()[1..source.trim().len() - 1], ',')
                .into_iter()
                .filter_map(|entry| parse_object_field(entry.as_str()))
                .collect::<Vec<_>>();
            return Some(ObjectType { fields });
        }

        self.types.get(source.trim()).and_then(|definition| definition.object.clone())
    }
}

#[derive(Debug, Clone, Copy)]
enum UtilityMode {
    Identity,
    Partial,
    Required,
    Readonly,
}

fn render_object_type_with_mode(
    object: &ObjectType,
    mode: UtilityMode,
    transformer: &PhaseFourTransformer,
    diagnostics: &mut Vec<Diagnostic>,
    config: &XLuauConfig,
) -> String {
    let fields = object
        .fields
        .iter()
        .map(|field| {
            let mut field = field.clone();
            match mode {
                UtilityMode::Identity => {}
                UtilityMode::Partial => field.optional = true,
                UtilityMode::Required => field.optional = false,
                UtilityMode::Readonly => field.readonly = true,
            }
            render_object_field_with_config(&field, transformer, diagnostics, config)
        })
        .collect::<Vec<_>>();
    format!("{{ {} }}", fields.join(", "))
}

fn render_object_field(
    field: ObjectField,
    transformer: &PhaseFourTransformer,
    diagnostics: &mut Vec<Diagnostic>,
) -> String {
    render_object_field_with_config(&field, transformer, diagnostics, &transformer.config)
}

fn render_object_field_with_config(
    field: &ObjectField,
    transformer: &PhaseFourTransformer,
    diagnostics: &mut Vec<Diagnostic>,
    config: &XLuauConfig,
) -> String {
    let ty = transformer.transform_type_expression(field.ty.as_str(), diagnostics);
    let name_prefix = if field.readonly && config.emit_readonly && config.luau_target == LuauTarget::NewSolver {
        "read "
    } else {
        ""
    };
    let optional_suffix = if field.optional { "?" } else { "" };
    let mut rendered = format!("{}{}: {}{}", name_prefix, field.name, ty.trim(), optional_suffix);
    if field.readonly && !(config.emit_readonly && config.luau_target == LuauTarget::NewSolver) {
        rendered.push_str(" -- @readonly");
    }
    rendered
}

fn parse_type_definition(text: &str) -> Option<TypeDefinition> {
    let trimmed = text.trim();
    let trimmed = trimmed.strip_prefix("type ")?;
    let (head, body) = split_top_level_once(trimmed, '=')?;
    let name = head
        .trim()
        .split('<')
        .next()
        .unwrap_or(head.trim())
        .trim()
        .to_owned();
    let body = body.trim().to_owned();
    let object = if body.starts_with('{') && body.ends_with('}') {
        Some(ObjectType {
            fields: split_top_level_angles(&body[1..body.len() - 1], ',')
                .into_iter()
                .filter_map(|entry| parse_object_field(entry.as_str()))
                .collect(),
        })
    } else {
        None
    };
    Some(TypeDefinition { name, object })
}

fn parse_function_signature(function: &FunctionStatement) -> Option<FunctionSignature> {
    let name = parse_function_name(function.header_prefix.as_str())?;
    let generics = parse_function_generics(function.header_prefix.as_str());
    let params = split_top_level_commas(function.params.as_str())
        .into_iter()
        .map(|param| parse_function_param(param.as_str()))
        .collect::<Vec<_>>();
    let return_type = parse_return_type(function.header_suffix.as_str());
    Some(FunctionSignature {
        name,
        generics,
        params,
        return_type,
    })
}

fn parse_function_name(prefix: &str) -> Option<String> {
    let trimmed = prefix.trim_start();
    let trimmed = trimmed
        .strip_prefix("local function")
        .or_else(|| trimmed.strip_prefix("function"))?
        .trim_start();
    let end = trimmed
        .char_indices()
        .find(|(_, ch)| *ch == '<' || ch.is_whitespace())
        .map(|(index, _)| index)
        .unwrap_or(trimmed.len());
    let name = trimmed[..end].trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_owned())
    }
}

fn parse_function_generics(prefix: &str) -> Vec<GenericParam> {
    let trimmed = prefix.trim();
    let Some(start) = trimmed.find('<') else {
        return Vec::new();
    };
    let Some(end) = trimmed.rfind('>') else {
        return Vec::new();
    };
    split_top_level_angles(&trimmed[start + 1..end], ',')
        .into_iter()
        .filter_map(|param| parse_generic_param(param.as_str()))
        .collect()
}

fn parse_generic_param(text: &str) -> Option<GenericParam> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (before_default, default) = split_top_level_once(trimmed, '=')
        .map(|(left, right)| (left.trim().to_owned(), Some(right.trim().to_owned())))
        .unwrap_or((trimmed.to_owned(), None));
    let (name, constraint) = split_keyword_once(before_default.as_str(), "extends")
        .map(|(left, right)| (left.trim().to_owned(), Some(right.trim().to_owned())))
        .unwrap_or((before_default.trim().to_owned(), None));
    Some(GenericParam {
        name,
        constraint,
        default,
    })
}

fn parse_function_param(text: &str) -> FunctionParam {
    let trimmed = text.trim();
    let trimmed = trimmed.strip_prefix("const ").unwrap_or(trimmed);
    let (name, ty) = split_top_level_once(trimmed, ':')
        .map(|(left, right)| (left.trim().to_owned(), Some(right.trim().to_owned())))
        .unwrap_or((trimmed.to_owned(), None));
    FunctionParam { name, ty }
}

fn parse_return_type(suffix: &str) -> Option<String> {
    let suffix = suffix.trim();
    let suffix = suffix.strip_prefix(')')?.trim();
    let suffix = suffix.strip_prefix(':')?.trim();
    if suffix.is_empty() {
        None
    } else {
        Some(suffix.to_owned())
    }
}

fn build_constraint_map(signature: &FunctionSignature) -> BTreeMap<String, String> {
    signature
        .generics
        .iter()
        .filter_map(|generic| {
            generic
                .constraint
                .as_ref()
                .map(|constraint| (generic.name.clone(), constraint.clone()))
        })
        .collect()
}

fn transform_function_param(
    text: &str,
    transformer: &PhaseFourTransformer,
    constraints: &BTreeMap<String, String>,
) -> String {
    let trimmed = text.trim();
    let trimmed = trimmed.strip_prefix("const ").unwrap_or(trimmed);
    let Some((name, ty)) = split_top_level_once(trimmed, ':') else {
        return trimmed.to_owned();
    };
    let mut ty = transformer.transform_type_expression(ty.trim(), &mut Vec::new());
    for (generic, constraint) in constraints {
        ty = replace_word(ty.as_str(), generic.as_str(), format!("({} & {})", generic, constraint).as_str());
    }
    format!("{}: {}", name.trim(), ty)
}

fn strip_function_generics(prefix: &str) -> String {
    let trimmed = prefix.trim_end();
    let Some(start) = trimmed.find('<') else {
        return trimmed.to_owned();
    };
    let Some(end) = trimmed.rfind('>') else {
        return trimmed.to_owned();
    };
    format!("{}{}", &trimmed[..start], &trimmed[end + 1..]).trim_end().to_owned()
}

fn parse_enum_definition(text: &str) -> Option<EnumDefinition> {
    let trimmed = text.trim();
    let trimmed = trimmed.strip_prefix("enum ")?;
    let brace_index = trimmed.find('{')?;
    let head = trimmed[..brace_index].trim();
    let body = trimmed[brace_index + 1..trimmed.rfind('}')?].trim();
    let (name, backing) = split_top_level_once(head, ':')
        .map(|(left, right)| (left.trim().to_owned(), Some(right.trim().to_owned())))
        .unwrap_or((head.to_owned(), None));

    let source =
        SourceFile::virtual_file(PathBuf::from("enum.xl"), SourceKind::XLuau, body.to_owned());
    let tokens = Lexer::new(&source).lex(&mut Vec::new());
    let mut members = Vec::new();
    let mut methods = Vec::new();
    let mut cursor = 0usize;

    while cursor < tokens.len() {
        let token = &tokens[cursor];
        if token.is_trivia() || token.kind == crate::lexer::TokenKind::Eof {
            cursor += 1;
            continue;
        }

        if token.kind == crate::lexer::TokenKind::Keyword(crate::lexer::Keyword::Function) {
            let end = find_function_end(&tokens, cursor);
            let method_text = source.text[token.span.start..tokens[end].span.end].to_owned();
            let transformed = transform_enum_method_text(method_text.as_str());
            let method_name = parse_function_name_from_text(method_text.as_str())?;
            methods.push(EnumMethod {
                name: tail_callable_name(method_name.as_str()).to_owned(),
                text: transformed,
            });
            cursor = end + 1;
            continue;
        }

        let end = find_top_level_comma(&tokens, cursor).unwrap_or(tokens.len() - 1);
        let raw = source.text[token.span.start..tokens[end].span.start.min(source.text.len())]
            .trim()
            .trim_end_matches(',')
            .trim()
            .to_owned();
        if !raw.is_empty() {
            let (member_name, value, explicit) = split_top_level_once(raw.as_str(), '=')
                .map(|(left, right)| (left.trim().to_owned(), right.trim().to_owned(), true))
                .unwrap_or((raw.clone(), format!("\"{}\"", raw.trim()), false));
            members.push(EnumMember {
                name: member_name,
                value,
                explicit,
            });
        }
        cursor = end + 1;
    }

    Some(EnumDefinition {
        name,
        backing,
        members,
        methods,
    })
}

fn enum_union_type(definition: &EnumDefinition) -> String {
    if definition
        .backing
        .as_ref()
        .map(|backing| backing.trim() == "number")
        .unwrap_or(false)
    {
        "number".to_owned()
    } else {
        definition
            .members
            .iter()
            .map(|member| {
                if definition.backing.is_some() || member.explicit {
                    member.value.clone()
                } else {
                    format!("\"{}\"", member.name)
                }
            })
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

fn transform_enum_method_text(text: &str) -> String {
    if let Some(name) = parse_function_name_from_text(text) {
        let tail = tail_callable_name(name.as_str());
        return text.replacen(
            format!("function {}", name).as_str(),
            format!("function {}", tail).as_str(),
            1,
        );
    }
    text.to_owned()
}

fn parse_function_name_from_text(text: &str) -> Option<String> {
    let header = text.lines().next()?.trim();
    let header = header.strip_prefix("function ")?;
    let end = header.find('(').unwrap_or(header.len());
    Some(header[..end].trim().to_owned())
}

fn find_function_end(tokens: &[crate::lexer::Token], start: usize) -> usize {
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate().skip(start) {
        match token.kind {
            crate::lexer::TokenKind::Keyword(crate::lexer::Keyword::Function)
            | crate::lexer::TokenKind::Keyword(crate::lexer::Keyword::If)
            | crate::lexer::TokenKind::Keyword(crate::lexer::Keyword::For)
            | crate::lexer::TokenKind::Keyword(crate::lexer::Keyword::While)
            | crate::lexer::TokenKind::Keyword(crate::lexer::Keyword::Repeat)
            | crate::lexer::TokenKind::Keyword(crate::lexer::Keyword::Switch) => depth += 1,
            crate::lexer::TokenKind::Keyword(crate::lexer::Keyword::End)
            | crate::lexer::TokenKind::Keyword(crate::lexer::Keyword::Until) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return index;
                }
            }
            _ => {}
        }
    }
    start
}

fn find_top_level_comma(tokens: &[crate::lexer::Token], start: usize) -> Option<usize> {
    let mut paren = 0usize;
    let mut brace = 0usize;
    let mut bracket = 0usize;
    for (index, token) in tokens.iter().enumerate().skip(start) {
        match token.kind {
            crate::lexer::TokenKind::Symbol(crate::lexer::Symbol::LeftParen) => paren += 1,
            crate::lexer::TokenKind::Symbol(crate::lexer::Symbol::RightParen) => {
                paren = paren.saturating_sub(1)
            }
            crate::lexer::TokenKind::Symbol(crate::lexer::Symbol::LeftBrace) => brace += 1,
            crate::lexer::TokenKind::Symbol(crate::lexer::Symbol::RightBrace) => {
                brace = brace.saturating_sub(1)
            }
            crate::lexer::TokenKind::Symbol(crate::lexer::Symbol::LeftBracket) => bracket += 1,
            crate::lexer::TokenKind::Symbol(crate::lexer::Symbol::RightBracket) => {
                bracket = bracket.saturating_sub(1)
            }
            crate::lexer::TokenKind::Symbol(crate::lexer::Symbol::Comma)
                if paren == 0 && brace == 0 && bracket == 0 =>
            {
                return Some(index);
            }
            _ => {}
        }
    }
    None
}

fn transform_binding_list(
    bindings: &str,
    transformer: &PhaseFourTransformer,
    diagnostics: &mut Vec<Diagnostic>,
) -> String {
    split_top_level_commas(bindings)
        .into_iter()
        .map(|binding| {
            if let Some((name, ty)) = split_top_level_once(binding.as_str(), ':') {
                format!(
                    "{}: {}",
                    name.trim(),
                    transformer.transform_type_expression(ty.trim(), diagnostics)
                )
            } else {
                binding.trim().to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn parse_object_field(text: &str) -> Option<ObjectField> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.starts_with('[') {
        return None;
    }
    let (name, ty) = split_top_level_once(trimmed, ':')?;
    let mut name = name.trim().to_owned();
    let mut readonly = false;
    if let Some(stripped) = name.strip_prefix("readonly ") {
        readonly = true;
        name = stripped.trim().to_owned();
    } else if let Some(stripped) = name.strip_prefix("mutable ") {
        name = stripped.trim().to_owned();
    }

    let mut ty = ty.trim().to_owned();
    let optional = ty.ends_with('?');
    if optional {
        ty.pop();
    }

    Some(ObjectField {
        name,
        ty: ty.trim().to_owned(),
        readonly,
        optional,
    })
}

fn transform_nested_type_utilities(
    text: &str,
    transformer: &PhaseFourTransformer,
    diagnostics: &mut Vec<Diagnostic>,
) -> String {
    let mut output = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0usize;

    while index < chars.len() {
        if !(chars[index].is_ascii_alphabetic() || chars[index] == '_') {
            output.push(chars[index]);
            index += 1;
            continue;
        }

        let start = index;
        index += 1;
        while index < chars.len() && (chars[index].is_ascii_alphanumeric() || chars[index] == '_')
        {
            index += 1;
        }
        let name = chars[start..index].iter().collect::<String>();
        if index < chars.len() && chars[index] == '<' {
            if let Some(end) = find_matching_angle(&chars, index) {
                let candidate = chars[start..=end].iter().collect::<String>();
                if let Some(expanded) = transformer.expand_builtin_utility(candidate.as_str(), diagnostics)
                {
                    output.push_str(expanded.as_str());
                    index = end + 1;
                    continue;
                }
            }
        }

        output.push_str(name.as_str());
    }

    if output.is_empty() {
        text.to_owned()
    } else {
        output
    }
}

fn transform_freeze_literals(text: &str) -> String {
    let mut output = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0usize;

    while index < chars.len() {
        if !starts_with_word(&chars, index, "freeze") {
            output.push(chars[index]);
            index += 1;
            continue;
        }

        let mut cursor = index + "freeze".len();
        while cursor < chars.len() && chars[cursor].is_whitespace() {
            cursor += 1;
        }
        if cursor >= chars.len() || chars[cursor] != '{' {
            output.push(chars[index]);
            index += 1;
            continue;
        }
        let Some(end) = find_matching_brace_chars(&chars, cursor) else {
            output.push(chars[index]);
            index += 1;
            continue;
        };

        output.push_str("table.freeze(");
        output.push_str(chars[cursor..=end].iter().collect::<String>().as_str());
        output.push(')');
        index = end + 1;
    }

    if output.is_empty() {
        text.to_owned()
    } else {
        output
    }
}

fn parse_type_application(text: &str) -> Option<(String, String)> {
    let trimmed = text.trim();
    let start = trimmed.find('<')?;
    let end = trimmed.rfind('>')?;
    if end <= start {
        return None;
    }
    Some((
        trimmed[..start].trim().to_owned(),
        trimmed[start + 1..end].trim().to_owned(),
    ))
}

fn parse_typeof_target(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let inner = trimmed.strip_prefix("typeof(")?.strip_suffix(')')?.trim();
    Some(
        inner
            .split('(')
            .next()
            .unwrap_or(inner)
            .trim()
            .to_owned(),
    )
}

fn substitute_type(text: &str, replacements: &BTreeMap<String, String>) -> String {
    replacements.iter().fold(text.to_owned(), |current, (name, value)| {
        replace_word(current.as_str(), name.as_str(), value.as_str())
    })
}

fn replace_word(text: &str, target: &str, replacement: &str) -> String {
    let mut output = String::new();
    let chars = text.chars().collect::<Vec<_>>();
    let target_chars = target.chars().collect::<Vec<_>>();
    let mut index = 0usize;

    while index < chars.len() {
        let left_ok = index == 0 || word_boundary(&chars, index - 1);
        let right_ok = word_boundary(&chars, index + target_chars.len());
        if index + target_chars.len() <= chars.len()
            && chars[index..index + target_chars.len()] == target_chars[..]
            && left_ok
            && right_ok
        {
            output.push_str(replacement);
            index += target_chars.len();
        } else {
            output.push(chars[index]);
            index += 1;
        }
    }

    output
}

fn word_boundary(chars: &[char], index: usize) -> bool {
    if index >= chars.len() {
        return true;
    }
    let ch = chars[index];
    !(ch == '_' || ch.is_ascii_alphanumeric())
}

fn contains_word(text: &str, needle: &str) -> bool {
    replace_word(text, needle, "__xluau_marker__").contains("__xluau_marker__")
}

fn generic_assignment_map(
    generics: &[GenericParam],
    concrete_types: &[String],
) -> BTreeMap<String, String> {
    generics
        .iter()
        .zip(concrete_types.iter())
        .map(|(generic, value)| (generic.name.clone(), value.clone()))
        .collect()
}

fn subtract_union(source: &str, removed: &str) -> String {
    let removed = split_top_level_pipes(removed)
        .into_iter()
        .map(|item| item.trim().to_owned())
        .collect::<HashSet<_>>();
    split_top_level_pipes(source)
        .into_iter()
        .filter(|item| !removed.contains(item.trim()))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn intersect_union(source: &str, kept: &str) -> String {
    let kept = split_top_level_pipes(kept)
        .into_iter()
        .map(|item| item.trim().to_owned())
        .collect::<HashSet<_>>();
    split_top_level_pipes(source)
        .into_iter()
        .filter(|item| kept.contains(item.trim()))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn keys_to_set(keys: &str) -> HashSet<String> {
    split_top_level_pipes(keys)
        .into_iter()
        .map(|key| key.trim().trim_matches('"').trim_matches('\'').to_owned())
        .collect()
}

fn split_top_level_pipes(text: &str) -> Vec<String> {
    split_top_level_with_separators(text, '|')
}

fn split_top_level_commas(text: &str) -> Vec<String> {
    split_top_level_with_separators(text, ',')
}

fn split_top_level_angles(text: &str, separator: char) -> Vec<String> {
    split_top_level_with_separators(text, separator)
}

fn split_top_level_with_separators(text: &str, separator: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut paren = 0usize;
    let mut brace = 0usize;
    let mut bracket = 0usize;
    let mut angle = 0usize;

    for ch in text.chars() {
        match ch {
            '(' => paren += 1,
            ')' => paren = paren.saturating_sub(1),
            '{' => brace += 1,
            '}' => brace = brace.saturating_sub(1),
            '[' => bracket += 1,
            ']' => bracket = bracket.saturating_sub(1),
            '<' => angle += 1,
            '>' => angle = angle.saturating_sub(1),
            _ => {}
        }

        if ch == separator && paren == 0 && brace == 0 && bracket == 0 && angle == 0 {
            if !current.trim().is_empty() {
                parts.push(current.trim().to_owned());
            }
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

fn split_top_level_once(text: &str, separator: char) -> Option<(String, String)> {
    let mut paren = 0usize;
    let mut brace = 0usize;
    let mut bracket = 0usize;
    let mut angle = 0usize;

    for (index, ch) in text.char_indices() {
        match ch {
            '(' => paren += 1,
            ')' => paren = paren.saturating_sub(1),
            '{' => brace += 1,
            '}' => brace = brace.saturating_sub(1),
            '[' => bracket += 1,
            ']' => bracket = bracket.saturating_sub(1),
            '<' => angle += 1,
            '>' => angle = angle.saturating_sub(1),
            _ => {}
        }

        if ch == separator && paren == 0 && brace == 0 && bracket == 0 && angle == 0 {
            return Some((
                text[..index].to_owned(),
                text[index + ch.len_utf8()..].to_owned(),
            ));
        }
    }

    None
}

fn split_keyword_once(text: &str, keyword: &str) -> Option<(String, String)> {
    let needle = format!(" {keyword} ");
    text.find(&needle).map(|index| {
        (
            text[..index].to_owned(),
            text[index + needle.len()..].to_owned(),
        )
    })
}

fn find_call_target_start(chars: &[char], lt_index: usize) -> Option<usize> {
    let mut cursor = lt_index;
    while cursor > 0 && chars[cursor - 1].is_whitespace() {
        cursor -= 1;
    }
    if cursor == 0 {
        return None;
    }
    let end = cursor;
    while cursor > 0 {
        let ch = chars[cursor - 1];
        if ch == '_' || ch.is_ascii_alphanumeric() || matches!(ch, '.' | ':' | ')') {
            cursor -= 1;
        } else {
            break;
        }
    }
    if cursor == end { None } else { Some(cursor) }
}

fn find_matching_angle(chars: &[char], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut paren = 0usize;
    let mut brace = 0usize;
    let mut bracket = 0usize;
    for (index, ch) in chars.iter().enumerate().skip(start) {
        match ch {
            '<' => depth += 1,
            '>' => {
                if paren == 0 && brace == 0 && bracket == 0 {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(index);
                    }
                }
            }
            '(' => paren += 1,
            ')' => paren = paren.saturating_sub(1),
            '{' => brace += 1,
            '}' => brace = brace.saturating_sub(1),
            '[' => bracket += 1,
            ']' => bracket = bracket.saturating_sub(1),
            _ => {}
        }
    }
    None
}

fn find_matching_paren_chars(chars: &[char], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (index, ch) in chars.iter().enumerate().skip(start) {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn find_matching_brace_chars(chars: &[char], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (index, ch) in chars.iter().enumerate().skip(start) {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn starts_with_word(chars: &[char], start: usize, word: &str) -> bool {
    let word_chars = word.chars().collect::<Vec<_>>();
    if start + word_chars.len() > chars.len() {
        return false;
    }
    if start > 0 {
        let prev = chars[start - 1];
        if prev == '_' || prev.is_ascii_alphanumeric() {
            return false;
        }
    }
    if chars[start..start + word_chars.len()] != word_chars[..] {
        return false;
    }
    if start + word_chars.len() < chars.len() {
        let next = chars[start + word_chars.len()];
        if next == '_' || next.is_ascii_alphanumeric() {
            return false;
        }
    }
    true
}

fn split_method_target(target: &str) -> (&str, &str) {
    target.rsplit_once(':').unwrap_or((target, ""))
}

fn strip_method_type_args_target(target: &str) -> String {
    target.trim().to_owned()
}

fn tail_callable_name(name: &str) -> &str {
    name.rsplit([':', '.']).next().unwrap_or(name)
}

fn statement_span_placeholder() -> crate::diagnostic::Span {
    crate::diagnostic::Span::new(0, 0)
}

fn function_body_mentions_constrained_member(body: &[Statement], name: &str) -> bool {
    let needle_method = format!("{}:", name);
    let needle_member = format!("{}.", name);
    body.iter().any(|statement| {
        let text = Emitter::new().emit_single_statement(statement);
        text.contains(needle_method.as_str()) || text.contains(needle_member.as_str())
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::PhaseFourTransformer;
    use crate::config::{LuauTarget, XLuauConfig};
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::source::{SourceFile, SourceKind};

    fn transform(text: &str) -> String {
        let source =
            SourceFile::virtual_file(PathBuf::from("test.xl"), SourceKind::XLuau, text.to_owned());
        let mut diagnostics = Vec::new();
        let tokens = Lexer::new(&source).lex(&mut diagnostics);
        let program = Parser::new(&source, &tokens).parse(&mut diagnostics);
        assert!(diagnostics.iter().all(|diagnostic| !diagnostic.is_error()));
        PhaseFourTransformer::new(XLuauConfig::default()).transform_program(
            &source,
            &program,
            &mut diagnostics,
        )
    }

    #[test]
    fn transforms_enum_keyword() {
        let output = transform("enum Direction { North, South }\n");
        assert!(output.contains("type Direction = \"North\" | \"South\""));
        assert!(output.contains("local Direction = {"));
        assert!(output.contains("table.freeze(Direction)"));
    }

    #[test]
    fn transforms_enum_with_explicit_values() {
        let output = transform("enum State { Ready = \"ready\", Waiting = \"waiting\" }\n");
        assert!(output.contains("type State = \"ready\" | \"waiting\""));
        assert!(output.contains("Ready = \"ready\" :: State"));
        assert!(output.contains("Waiting = \"waiting\" :: State"));
    }

    #[test]
    fn transforms_generics_defaults_and_explicit_type_args() {
        let output = transform(
            "function wrap<T extends string, U = {T}>(value: T): U\n    return value :: any\nend\nlocal boxed = wrap<number?>(nil)\n",
        );
        assert!(output.contains("function wrap(value: (T & string))"));
        assert!(output.contains("(nil :: (T & string))") || output.contains("(nil :: number?)"));
    }

    #[test]
    fn transforms_readonly_freeze_and_utilities() {
        let output = transform(
            "type Config = { readonly host: string, mutable timeout: number }\ntype PartialConfig = Partial<Config>\nconst defaults = freeze { host = \"localhost\" }\n",
        );
        assert!(output.contains("type Config = { read host: string, timeout: number }"));
        assert!(output.contains("type PartialConfig = { read host: string?, timeout: number? }"));
        assert!(output.contains("freeze"));
        assert!(output.contains("table.freeze({ host = \"localhost\" })"));
    }

    #[test]
    fn legacy_readonly_uses_comments() {
        let mut config = XLuauConfig::default();
        config.luau_target = LuauTarget::Legacy;

        let source = SourceFile::virtual_file(
            PathBuf::from("test.xl"),
            SourceKind::XLuau,
            "type Config = { readonly host: string }\n".to_owned(),
        );
        let mut diagnostics = Vec::new();
        let tokens = Lexer::new(&source).lex(&mut diagnostics);
        let program = Parser::new(&source, &tokens).parse(&mut diagnostics);
        let output = PhaseFourTransformer::new(config).transform_program(&source, &program, &mut diagnostics);
        assert!(output.contains("-- @readonly"));
    }
}
