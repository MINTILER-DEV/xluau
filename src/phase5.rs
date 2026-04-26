use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::path::PathBuf;

use crate::ast::{
    ConditionalKeyword, ExportKind, LocalKeyword, Program, Statement, StatementKind, StatementNode,
    SwitchLabel,
};
use crate::config::XLuauConfig;
use crate::diagnostic::Diagnostic;
use crate::emitter::Emitter;
use crate::lexer::{Keyword, Lexer, TokenKind};
use crate::source::{SourceFile, SourceKind};

const DECORATOR_REGISTRY_NAME: &str = "_xluau_decorators";

#[derive(Debug, Clone)]
pub struct PhaseFiveTransformer {
    config: XLuauConfig,
    interfaces: HashMap<String, InterfaceDef>,
    classes: HashMap<String, ClassDef>,
    uses_custom_decorators: bool,
}

#[derive(Debug, Clone)]
struct InterfaceDef {
    name: String,
    exported: bool,
    members: Vec<InterfaceMember>,
}

#[derive(Debug, Clone)]
struct InterfaceMember {
    name: String,
    ty: String,
    requires_self: bool,
}

#[derive(Debug, Clone)]
struct ClassDef {
    name: String,
    exported: bool,
    abstract_class: bool,
    extends: Option<String>,
    implements: Vec<String>,
    decorators: Vec<Decorator>,
    fields: Vec<FieldDef>,
    methods: Vec<MethodDef>,
}

#[derive(Debug, Clone)]
struct FieldDef {
    name: String,
    ty: String,
    decorators: Vec<Decorator>,
    readonly: bool,
}

#[derive(Debug, Clone)]
struct MethodDef {
    name: String,
    params: String,
    return_type: Option<String>,
    body: Option<String>,
    decorators: Vec<Decorator>,
    is_static: bool,
    is_abstract: bool,
    is_constructor: bool,
}

#[derive(Debug, Clone)]
struct Decorator {
    name: String,
    args: Option<String>,
}

impl PhaseFiveTransformer {
    pub fn new(config: XLuauConfig) -> Self {
        Self {
            config,
            interfaces: HashMap::new(),
            classes: HashMap::new(),
            uses_custom_decorators: false,
        }
    }

    pub fn transform_program(
        mut self,
        source: &SourceFile,
        program: &Program,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        self.collect_statement_list(&program.statements);
        self.validate(source, diagnostics);

        let mut output = String::new();
        if self.uses_custom_decorators {
            if let Some(path) = &self.config.decorator_library {
                writeln!(
                    output,
                    "local {} = require(\"{}\")",
                    DECORATOR_REGISTRY_NAME,
                    escape_string(path.to_string_lossy().as_ref())
                )
                .ok();
            }
        }

        output.push_str(
            self.transform_statement_list(source.path.clone(), &program.statements, diagnostics)
                .as_str(),
        );
        output
    }

    fn collect_statement_list(&mut self, statements: &[Statement]) {
        let mut cursor = 0usize;
        let mut pending_decorators = Vec::new();

        while cursor < statements.len() {
            if let Some(decorator) = statement_decorator(&statements[cursor]) {
                pending_decorators.push(decorator);
                cursor += 1;
                continue;
            }

            if let Some(interface) = parse_interface_from_statement(
                &statements[cursor],
                pending_decorators.clone(),
            ) {
                self.uses_custom_decorators |= has_custom_decorators(&pending_decorators);
                self.interfaces.insert(interface.name.clone(), interface);
                pending_decorators.clear();
            } else if let Some(class) =
                parse_class_from_statement(&statements[cursor], pending_decorators.clone())
            {
                self.uses_custom_decorators |= has_custom_decorators(&pending_decorators);
                self.uses_custom_decorators |= has_custom_class_decorators(&class);
                self.classes.insert(class.name.clone(), class);
                pending_decorators.clear();
            } else {
                pending_decorators.clear();
                self.collect_nested_definitions(&statements[cursor]);
            }

            cursor += 1;
        }
    }

    fn collect_nested_definitions(&mut self, statement: &Statement) {
        match &statement.node {
            StatementNode::Function(function) => self.collect_statement_list(&function.body),
            StatementNode::If(if_stmt) => {
                for clause in &if_stmt.clauses {
                    self.collect_statement_list(&clause.body);
                }
                if let Some(body) = &if_stmt.else_body {
                    self.collect_statement_list(body);
                }
            }
            StatementNode::While(while_stmt) => self.collect_statement_list(&while_stmt.body),
            StatementNode::Repeat(repeat_stmt) => self.collect_statement_list(&repeat_stmt.body),
            StatementNode::For(for_stmt) => self.collect_statement_list(&for_stmt.body),
            StatementNode::Do(block) => self.collect_statement_list(&block.body),
            StatementNode::Switch(switch) => {
                for section in &switch.sections {
                    self.collect_statement_list(&section.body);
                }
            }
            StatementNode::Export(export) => {
                if let ExportKind::Declaration(node) = &export.kind {
                    if let StatementNode::Function(function) = node.as_ref() {
                        self.collect_statement_list(&function.body);
                    }
                }
            }
            _ => {}
        }
    }

    fn validate(&self, source: &SourceFile, diagnostics: &mut Vec<Diagnostic>) {
        if self.uses_custom_decorators && self.config.decorator_library.is_none() {
            diagnostics.push(Diagnostic::error(
                Some(&source.path),
                None,
                "custom decorators require `decoratorLibrary` in xluau.config.json",
            ));
        }

        for class in self.classes.values() {
            if let Some(parent_name) = &class.extends {
                if let Some(parent) = self.classes.get(parent_name) {
                    if class_has_decorator(parent, "sealed") {
                        diagnostics.push(Diagnostic::error(
                            Some(&source.path),
                            None,
                            format!(
                                "class `{}` cannot extend sealed class `{}`",
                                class.name, parent_name
                            ),
                        ));
                    }
                }
            }

            if !class.abstract_class {
                let missing = self.missing_abstract_methods(class);
                if !missing.is_empty() {
                    diagnostics.push(Diagnostic::error(
                        Some(&source.path),
                        None,
                        format!(
                            "class `{}` must implement abstract method(s): {}",
                            class.name,
                            missing.join(", ")
                        ),
                    ));
                }
            }

            let readonly_fields = class
                .fields
                .iter()
                .filter(|field| field.readonly)
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>();
            if !readonly_fields.is_empty() {
                for method in class.methods.iter().filter(|method| !method.is_constructor) {
                    if let Some(body) = &method.body {
                        for field in &readonly_fields {
                            if body.contains(format!("self.{} =", field).as_str()) {
                                diagnostics.push(Diagnostic::error(
                                    Some(&source.path),
                                    None,
                                    format!(
                                        "readonly property `{}.{}` cannot be assigned outside the constructor",
                                        class.name, field
                                    ),
                                ));
                            }
                        }
                    }
                }
            }

            let instance_members = self.instance_member_names(class.name.as_str());
            let static_members = self.static_member_names(class.name.as_str());
            for interface_name in &class.implements {
                let Some(interface) = self.interfaces.get(interface_name) else {
                    diagnostics.push(Diagnostic::error(
                        Some(&source.path),
                        None,
                        format!(
                            "class `{}` implements unknown interface `{}`",
                            class.name, interface_name
                        ),
                    ));
                    continue;
                };

                let mut missing = Vec::new();
                for member in &interface.members {
                    let present = if member.requires_self {
                        instance_members.contains(member.name.as_str())
                    } else {
                        static_members.contains(member.name.as_str())
                            || instance_members.contains(member.name.as_str())
                    };
                    if !present {
                        missing.push(member.name.clone());
                    }
                }

                if !missing.is_empty() {
                    diagnostics.push(Diagnostic::error(
                        Some(&source.path),
                        None,
                        format!(
                            "class `{}` is missing interface member(s) from `{}`: {}",
                            class.name,
                            interface_name,
                            missing.join(", ")
                        ),
                    ));
                }
            }
        }
    }

    fn missing_abstract_methods(&self, class: &ClassDef) -> Vec<String> {
        let mut required = HashSet::new();
        self.collect_inherited_abstract_methods(class.extends.as_deref(), &mut required);

        for method in class.methods.iter().filter(|method| !method.is_constructor) {
            if method.is_abstract {
                required.insert(method.name.clone());
            } else {
                required.remove(method.name.as_str());
            }
        }

        let mut missing = required.into_iter().collect::<Vec<_>>();
        missing.sort();
        missing
    }

    fn collect_inherited_abstract_methods(&self, class_name: Option<&str>, required: &mut HashSet<String>) {
        let Some(class_name) = class_name else {
            return;
        };
        let Some(class) = self.classes.get(class_name) else {
            return;
        };

        self.collect_inherited_abstract_methods(class.extends.as_deref(), required);
        for method in class.methods.iter().filter(|method| !method.is_constructor) {
            if method.is_abstract {
                required.insert(method.name.clone());
            } else {
                required.remove(method.name.as_str());
            }
        }
    }

    fn instance_member_names(&self, class_name: &str) -> HashSet<String> {
        let mut names = HashSet::new();
        self.collect_instance_members(class_name, &mut names);
        names
    }

    fn collect_instance_members(&self, class_name: &str, names: &mut HashSet<String>) {
        let Some(class) = self.classes.get(class_name) else {
            return;
        };
        if let Some(parent) = &class.extends {
            self.collect_instance_members(parent, names);
        }
        for field in &class.fields {
            names.insert(field.name.clone());
        }
        for method in class
            .methods
            .iter()
            .filter(|method| !method.is_static && !method.is_constructor)
        {
            names.insert(method.name.clone());
        }
    }

    fn static_member_names(&self, class_name: &str) -> HashSet<String> {
        let mut names = HashSet::new();
        self.collect_static_members(class_name, &mut names);
        names
    }

    fn collect_static_members(&self, class_name: &str, names: &mut HashSet<String>) {
        let Some(class) = self.classes.get(class_name) else {
            return;
        };
        if let Some(parent) = &class.extends {
            self.collect_static_members(parent, names);
        }
        names.insert("new".to_owned());
        for method in class.methods.iter().filter(|method| method.is_static) {
            names.insert(method.name.clone());
        }
    }

    fn transform_statement_list(
        &self,
        path: PathBuf,
        statements: &[Statement],
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let mut output = String::new();
        let mut cursor = 0usize;
        let mut pending_decorators = Vec::new();

        while cursor < statements.len() {
            if let Some(raw) = statement_source(&statements[cursor]) {
                if let Some(decorator) = parse_decorator(raw.trim()) {
                    pending_decorators.push((decorator, raw));
                    cursor += 1;
                    continue;
                }
            }

            let decorators = pending_decorators
                .iter()
                .map(|(decorator, _)| decorator.clone())
                .collect::<Vec<_>>();

            if let Some(interface) = parse_interface_from_statement(&statements[cursor], decorators.clone()) {
                output.push_str(
                    self.emit_interface(interface, diagnostics)
                        .as_str(),
                );
                output.push_str(statements[cursor].trailing.as_str());
                pending_decorators.clear();
                cursor += 1;
                continue;
            }

            if let Some(class) = parse_class_from_statement(&statements[cursor], decorators) {
                output.push_str(self.emit_class(path.clone(), &class, diagnostics).as_str());
                output.push_str(statements[cursor].trailing.as_str());
                pending_decorators.clear();
                cursor += 1;
                continue;
            }

            for (_, raw) in pending_decorators.drain(..) {
                output.push_str(raw.as_str());
            }

            output.push_str(
                self.transform_statement(path.clone(), &statements[cursor], diagnostics)
                    .as_str(),
            );
            cursor += 1;
        }

        for (_, raw) in pending_decorators {
            output.push_str(raw.as_str());
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
            StatementNode::Text(text) => {
                self.emit_abstract_instantiation_diagnostics(&path, text, diagnostics);
                text.clone()
            }
            StatementNode::Import(_) => Emitter::new().emit_single_statement(statement),
            StatementNode::Export(export) => self.transform_export(path, export, diagnostics),
            StatementNode::Local(local) => match &local.value {
                Some(value) => {
                    self.emit_abstract_instantiation_diagnostics(&path, value, diagnostics);
                    let keyword = match local.keyword {
                        LocalKeyword::Local => "local",
                        LocalKeyword::Let => "let",
                        LocalKeyword::Const => "const",
                    };
                    format!("{keyword} {} = {}", local.bindings, value)
                }
                None => {
                    let keyword = match local.keyword {
                        LocalKeyword::Local => "local",
                        LocalKeyword::Let => "let",
                        LocalKeyword::Const => "const",
                    };
                    format!("{keyword} {}", local.bindings)
                }
            },
            StatementNode::Return(ret) => match &ret.values {
                Some(values) => {
                    self.emit_abstract_instantiation_diagnostics(&path, values, diagnostics);
                    format!("return {values}")
                }
                None => "return".to_owned(),
            },
            StatementNode::If(if_stmt) => {
                let mut output = String::new();
                for clause in &if_stmt.clauses {
                    self.emit_abstract_instantiation_diagnostics(
                        &path,
                        clause.condition.as_str(),
                        diagnostics,
                    );
                    let keyword = match clause.keyword {
                        ConditionalKeyword::If => "if",
                        ConditionalKeyword::ElseIf => "elseif",
                    };
                    writeln!(output, "{} {} then", keyword, clause.condition).ok();
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
            StatementNode::While(while_stmt) => {
                self.emit_abstract_instantiation_diagnostics(
                    &path,
                    while_stmt.condition.as_str(),
                    diagnostics,
                );
                format!(
                    "while {} do\n{}end",
                    while_stmt.condition,
                    self.transform_statement_list(path.clone(), &while_stmt.body, diagnostics)
                )
            }
            StatementNode::Repeat(repeat_stmt) => {
                self.emit_abstract_instantiation_diagnostics(
                    &path,
                    repeat_stmt.condition.as_str(),
                    diagnostics,
                );
                format!(
                    "repeat\n{}until {}",
                    self.transform_statement_list(path.clone(), &repeat_stmt.body, diagnostics),
                    repeat_stmt.condition
                )
            }
            StatementNode::For(for_stmt) => {
                self.emit_abstract_instantiation_diagnostics(&path, &for_stmt.head, diagnostics);
                format!(
                    "for {} do\n{}end",
                    for_stmt.head,
                    self.transform_statement_list(path.clone(), &for_stmt.body, diagnostics)
                )
            }
            StatementNode::Function(function) => {
                self.emit_abstract_instantiation_diagnostics(&path, &function.header_prefix, diagnostics);
                let mut output = String::new();
                output.push_str(function.header_prefix.as_str());
                output.push_str(function.params.as_str());
                output.push_str(function.header_suffix.as_str());
                if !output.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str(
                    self.transform_statement_list(path.clone(), &function.body, diagnostics)
                        .as_str(),
                );
                output.push_str("end");
                output
            }
            StatementNode::Do(block) => format!(
                "do\n{}end",
                self.transform_statement_list(path, &block.body, diagnostics)
            ),
            StatementNode::Switch(switch) => {
                self.emit_abstract_instantiation_diagnostics(
                    &path,
                    switch.expression.as_str(),
                    diagnostics,
                );
                let mut output = format!("switch {}\n", switch.expression);
                for section in &switch.sections {
                    match &section.label {
                        SwitchLabel::Case(values) => {
                            output.push_str(format!("case {}:\n", values.join(", ")).as_str())
                        }
                        SwitchLabel::Default => output.push_str("default:\n"),
                    }
                    output.push_str(
                        self.transform_statement_list(path.clone(), &section.body, diagnostics)
                            .as_str(),
                    );
                }
                output.push_str("end");
                output
            }
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
            ExportKind::Declaration(node) => {
                let statement = Statement {
                    kind: StatementKind::ExportDeclaration,
                    node: node.as_ref().clone(),
                    trailing: String::new(),
                    span: crate::diagnostic::Span::new(0, 0),
                };
                format!("export {}", self.transform_statement(path, &statement, diagnostics))
            }
            _ => Emitter::new().emit_single_statement(&Statement {
                kind: StatementKind::ExportDeclaration,
                node: StatementNode::Export(export.clone()),
                trailing: String::new(),
                span: crate::diagnostic::Span::new(0, 0),
            }),
        }
    }

    fn emit_interface(
        &self,
        interface: InterfaceDef,
        _diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let mut output = String::new();
        let prefix = if interface.exported { "export " } else { "" };
        writeln!(output, "{}type {} = {{", prefix, interface.name).ok();
        for member in interface.members {
            writeln!(output, "    {}: {},", member.name, member.ty).ok();
        }
        output.push('}');
        output
    }

    fn emit_class(
        &self,
        path: PathBuf,
        class: &ClassDef,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let mut output = String::new();
        let type_prefix = if class.exported { "export " } else { "" };
        let extends_clause = class
            .extends
            .as_ref()
            .map(|parent| format!("{parent} & "))
            .unwrap_or_default();

        writeln!(output, "{}type {} = {}{{", type_prefix, class.name, extends_clause).ok();
        for field in &class.fields {
            writeln!(output, "    {}: {},", field.name, field.ty).ok();
        }
        for method in class
            .methods
            .iter()
            .filter(|method| !method.is_static && !method.is_constructor && !method.is_abstract)
        {
            writeln!(
                output,
                "    {}: {},",
                method.name,
                instance_method_type(class.name.as_str(), method)
            )
            .ok();
        }
        output.push_str("}\n");

        let constructor_params = class
            .methods
            .iter()
            .find(|method| method.is_constructor)
            .map(|method| method.params.clone())
            .unwrap_or_else(|| "...any".to_owned());

        writeln!(output, "type {}Class = {{", class.name).ok();
        writeln!(
            output,
            "    new: ({}) -> {},",
            constructor_params,
            class.name
        )
        .ok();
        for method in class.methods.iter().filter(|method| method.is_static && !method.is_abstract) {
            writeln!(
                output,
                "    {}: {},",
                method.name,
                static_method_type(class.name.as_str(), method)
            )
            .ok();
        }
        output.push_str("}\n");

        writeln!(
            output,
            "local {}: {}Class = {{}} :: {}Class",
            class.name, class.name, class.name
        )
        .ok();
        writeln!(output, "{}.__index = {}", class.name, class.name).ok();
        if let Some(parent) = &class.extends {
            writeln!(output, "setmetatable({}, {{ __index = {} }})", class.name, parent).ok();
        }

        let readonly_fields = class
            .fields
            .iter()
            .filter(|field| field.readonly)
            .map(|field| field.name.clone())
            .collect::<Vec<_>>();
        if !readonly_fields.is_empty() {
            writeln!(output, "{}.__readonly = {{", class.name).ok();
            for field in &readonly_fields {
                writeln!(output, "    [\"{}\"] = true,", field).ok();
            }
            output.push_str("}\n");
        }

        let singleton = class_has_decorator(class, "singleton");
        if singleton {
            writeln!(output, "local _{}_singleton: {}? = nil", class.name, class.name).ok();
        }

        let constructor = class.methods.iter().find(|method| method.is_constructor);
        output.push_str(
            self.emit_constructor(path.clone(), class, constructor, singleton, diagnostics)
                .as_str(),
        );

        for method in class.methods.iter().filter(|method| !method.is_constructor && !method.is_abstract) {
            output.push_str(
                self.emit_method(path.clone(), class, method, diagnostics)
                    .as_str(),
            );
        }

        for decorator in class.decorators.iter().filter(|decorator| !is_builtin_decorator(decorator.name.as_str())) {
            output.push_str(
                emit_custom_class_decorator(
                    class.name.as_str(),
                    decorator,
                )
                .as_str(),
            );
        }

        for field in &class.fields {
            for decorator in field
                .decorators
                .iter()
                .filter(|decorator| !is_builtin_decorator(decorator.name.as_str()))
            {
                output.push_str(
                    emit_custom_property_decorator(class.name.as_str(), field.name.as_str(), decorator)
                        .as_str(),
                );
            }
        }

        if class.exported {
            writeln!(output, "export {{ {} }}", class.name).ok();
        }

        output
    }

    fn emit_constructor(
        &self,
        path: PathBuf,
        class: &ClassDef,
        constructor: Option<&MethodDef>,
        singleton: bool,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let params = constructor
            .map(|method| method.params.clone())
            .unwrap_or_default();
        let mut body = constructor
            .and_then(|method| method.body.clone())
            .unwrap_or_default();
        let mut super_args = None;

        if let Some(parent) = &class.extends {
            let (found, stripped) = extract_super_constructor_call(body.as_str());
            body = stripped;
            super_args = found;
            if constructor.is_some() && super_args.is_none() {
                diagnostics.push(Diagnostic::error(
                    Some(&path),
                    None,
                    format!(
                        "constructor for `{}` must call super(...) before accessing inherited state",
                        class.name
                    ),
                ));
            }
            body = rewrite_super_method_calls(body.as_str(), parent.as_str());
        }

        self.emit_abstract_instantiation_diagnostics(&path, body.as_str(), diagnostics);

        let mut output = String::new();
        writeln!(
            output,
            "function {}.new({}): {}",
            class.name,
            params,
            class.name
        )
        .ok();
        if singleton {
            writeln!(
                output,
                "    if _{}_singleton ~= nil then",
                class.name
            )
            .ok();
            writeln!(output, "        return _{}_singleton", class.name).ok();
            output.push_str("    end\n");
        }

        if let Some(parent) = &class.extends {
            let args = super_args.unwrap_or_default();
            writeln!(
                output,
                "    local self = {}.new({}) :: {}",
                parent,
                args,
                class.name
            )
            .ok();
            writeln!(output, "    setmetatable(self, {})", class.name).ok();
        } else {
            writeln!(
                output,
                "    local self = setmetatable({}, {}) :: {}",
                "{}",
                class.name,
                class.name
            )
            .ok();
        }

        if !body.trim().is_empty() {
            output.push_str(indent_block(body.as_str(), "    ").as_str());
        }
        if singleton {
            writeln!(output, "    _{}_singleton = self", class.name).ok();
        }
        output.push_str("    return self\nend\n");
        output
    }

    fn emit_method(
        &self,
        path: PathBuf,
        class: &ClassDef,
        method: &MethodDef,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> String {
        let mut body = method.body.clone().unwrap_or_default();
        if let Some(parent) = &class.extends {
            body = rewrite_super_method_calls(body.as_str(), parent.as_str());
        }
        self.emit_abstract_instantiation_diagnostics(&path, body.as_str(), diagnostics);

        let deprecated = method_decorator(method, "deprecated")
            .and_then(|decorator| decorator.args.clone())
            .map(|args| args.trim().to_owned());
        if let Some(message) = deprecated {
            body = format!("warn({})\n{}", message, body);
        }

        let mut output = String::new();
        if method_decorator(method, "memoize").is_some() {
            writeln!(
                output,
                "local _{}_{}_cache = {{}}",
                class.name, method.name
            )
            .ok();
        }

        let accessor = if method.is_static { "." } else { ":" };
        let signature = format!(
            "function {}{}{}({}){}",
            class.name,
            accessor,
            method.name,
            method.params,
            method
                .return_type
                .as_ref()
                .map(|ret| format!(": {}", ret))
                .unwrap_or_default()
        );
        writeln!(output, "{}", signature).ok();

        if method_decorator(method, "memoize").is_some() {
            output.push_str(
                indent_block(
                    memoized_method_body(class.name.as_str(), method.name.as_str(), method.params.as_str(), body.as_str()).as_str(),
                    "    ",
                )
                .as_str(),
            );
        } else if !body.trim().is_empty() {
            output.push_str(indent_block(body.as_str(), "    ").as_str());
        }
        output.push_str("end\n");

        for decorator in method
            .decorators
            .iter()
            .filter(|decorator| !is_builtin_decorator(decorator.name.as_str()))
        {
            output.push_str(
                emit_custom_method_decorator(class.name.as_str(), method.name.as_str(), decorator)
                    .as_str(),
            );
        }

        output
    }

    fn emit_abstract_instantiation_diagnostics(
        &self,
        path: &PathBuf,
        text: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        for class in self.classes.values().filter(|class| class.abstract_class) {
            let pattern = format!("{}.new(", class.name);
            if text.contains(pattern.as_str()) {
                diagnostics.push(Diagnostic::error(
                    Some(path),
                    None,
                    format!("abstract class `{}` cannot be instantiated directly", class.name),
                ));
            }
        }
    }
}

fn parse_interface_from_statement(statement: &Statement, _decorators: Vec<Decorator>) -> Option<InterfaceDef> {
    match &statement.node {
        StatementNode::Text(text) => parse_interface_definition(text, false),
        StatementNode::Export(export) => match &export.kind {
            ExportKind::Declaration(node) => match node.as_ref() {
                StatementNode::Text(text) => parse_interface_definition(text, true),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

fn parse_class_from_statement(statement: &Statement, decorators: Vec<Decorator>) -> Option<ClassDef> {
    match &statement.node {
        StatementNode::Text(text) => parse_class_definition(text, decorators, false),
        StatementNode::Export(export) => match &export.kind {
            ExportKind::Declaration(node) => match node.as_ref() {
                StatementNode::Text(text) => parse_class_definition(text, decorators, true),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

fn parse_interface_definition(text: &str, exported: bool) -> Option<InterfaceDef> {
    let trimmed = text.trim();
    let body_start = trimmed.find('{')?;
    let body_end = trimmed.rfind('}')?;
    let header = trimmed[..body_start].trim();
    let name = header.strip_prefix("interface ")?.trim().to_owned();
    let members = split_interface_members(&trimmed[body_start + 1..body_end])
        .into_iter()
        .filter_map(|line| parse_interface_member(line.as_str()))
        .collect::<Vec<_>>();
    Some(InterfaceDef {
        name,
        exported,
        members,
    })
}

fn parse_class_definition(text: &str, decorators: Vec<Decorator>, exported: bool) -> Option<ClassDef> {
    let trimmed = text.trim();
    let abstract_prefix = "abstract class ";
    let class_prefix = "class ";
    let (abstract_class, rest) = if let Some(rest) = trimmed.strip_prefix(abstract_prefix) {
        (true, rest)
    } else if let Some(rest) = trimmed.strip_prefix(class_prefix) {
        (false, rest)
    } else {
        return None;
    };

    let body_start = rest.find('{')?;
    let body_end = rest.rfind('}')?;
    let header = rest[..body_start].trim();
    let body = &rest[body_start + 1..body_end];

    let (name, extends, implements) = parse_class_header(header)?;
    let members = split_class_members(body);
    let mut fields = Vec::new();
    let mut methods = Vec::new();
    let mut pending = Vec::new();

    for member in members {
        let trimmed = member.trim();
        if trimmed.is_empty() || trimmed.starts_with("--") {
            continue;
        }
        if let Some(decorator) = parse_decorator(trimmed) {
            pending.push(decorator);
            continue;
        }

        if let Some(method) = parse_method(member.as_str(), pending.clone()) {
            methods.push(method);
            pending.clear();
            continue;
        }

        if let Some(field) = parse_field(member.as_str(), pending.clone()) {
            fields.push(field);
            pending.clear();
            continue;
        }

        pending.clear();
    }

    let abstract_class = abstract_class || decorators.iter().any(|decorator| decorator.name == "abstract");
    Some(ClassDef {
        name,
        exported,
        abstract_class,
        extends,
        implements,
        decorators,
        fields,
        methods,
    })
}

fn parse_class_header(header: &str) -> Option<(String, Option<String>, Vec<String>)> {
    let name_end = header
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(index, _)| index)
        .unwrap_or(header.len());
    let name = header[..name_end].trim().to_owned();
    if name.is_empty() {
        return None;
    }
    let mut rest = header[name_end..].trim();
    let mut extends = None;
    let mut implements = Vec::new();

    if let Some(after) = rest.strip_prefix("extends ") {
        if let Some((parent, remainder)) = split_keyword(after, "implements") {
            extends = Some(parent.trim().to_owned());
            rest = remainder.trim();
        } else {
            extends = Some(after.trim().to_owned());
            rest = "";
        }
    }

    if let Some(after) = rest.strip_prefix("implements ") {
        implements = after
            .split(',')
            .map(|part| part.trim().to_owned())
            .filter(|part| !part.is_empty())
            .collect();
    } else if !rest.is_empty() {
        implements = rest
            .split(',')
            .map(|part| part.trim().to_owned())
            .filter(|part| !part.is_empty())
            .collect();
    }

    Some((name, extends, implements))
}

fn split_class_members(body: &str) -> Vec<String> {
    let mut members = Vec::new();
    let mut current = String::new();
    let mut depth = 0isize;
    let lines = body.lines().collect::<Vec<_>>();

    for (index, line) in lines.iter().enumerate() {
        let line_with_newline = if index + 1 == lines.len() {
            (*line).to_owned()
        } else {
            format!("{line}\n")
        };
        let trimmed = line.trim();
        if current.is_empty() && trimmed.is_empty() {
            continue;
        }

        let begins_block = if current.is_empty() {
            member_starts_block(trimmed)
        } else {
            false
        };
        if current.is_empty() && begins_block {
            depth = 1;
        }

        current.push_str(line_with_newline.as_str());
        if depth > 0 {
            let mut delta = block_delta_for_line(trimmed);
            if begins_block {
                delta -= 1;
            }
            depth += delta;
            if depth <= 0 {
                members.push(current.trim_end_matches('\n').to_owned());
                current.clear();
            }
        } else {
            members.push(current.trim_end_matches('\n').to_owned());
            current.clear();
        }
    }

    if !current.trim().is_empty() {
        members.push(current.trim().to_owned());
    }

    members
}

fn split_interface_members(body: &str) -> Vec<String> {
    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("--"))
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_interface_member(line: &str) -> Option<InterfaceMember> {
    let (name, ty) = split_top_level_once(line, ':')?;
    let ty = ty.trim().trim_end_matches(',').to_owned();
    Some(InterfaceMember {
        name: name.trim().to_owned(),
        requires_self: ty.contains("self:"),
        ty,
    })
}

fn parse_field(text: &str, decorators: Vec<Decorator>) -> Option<FieldDef> {
    let trimmed = text.trim().trim_end_matches(',');
    if trimmed.starts_with("function ")
        || trimmed.starts_with("static function ")
        || trimmed.starts_with("constructor(")
        || trimmed.starts_with("abstract function ")
    {
        return None;
    }
    let (name, ty) = split_top_level_once(trimmed, ':')?;
    let readonly = decorators.iter().any(|decorator| decorator.name == "readonly");
    Some(FieldDef {
        name: name.trim().to_owned(),
        ty: ty.trim().to_owned(),
        decorators,
        readonly,
    })
}

fn parse_method(text: &str, decorators: Vec<Decorator>) -> Option<MethodDef> {
    let trimmed = text.trim();
    let mut is_static = false;
    let is_abstract = decorators.iter().any(|decorator| decorator.name == "abstract");
    let mut header = trimmed;

    if let Some(rest) = header.strip_prefix("abstract function ") {
        return parse_function_like(rest, decorators, false, true, false);
    }

    if let Some(rest) = header.strip_prefix("static function ") {
        is_static = true;
        header = rest;
    } else if let Some(rest) = header.strip_prefix("function ") {
        header = rest;
    } else if let Some(rest) = header.strip_prefix("constructor") {
        return parse_constructor(rest, decorators);
    } else {
        return None;
    }

    parse_function_like(header, decorators, is_static, is_abstract, false)
}

fn parse_constructor(rest: &str, decorators: Vec<Decorator>) -> Option<MethodDef> {
    let rest = format!("constructor{}", rest);
    let lines = rest.lines().collect::<Vec<_>>();
    let header = lines.first()?.trim();
    let params = extract_paren_contents(header.strip_prefix("constructor")?)?;
    let body = if lines.len() > 1 {
        Some(lines[1..lines.len().saturating_sub(1)].join("\n"))
    } else {
        Some(String::new())
    };
    Some(MethodDef {
        name: "constructor".to_owned(),
        params,
        return_type: None,
        body,
        decorators,
        is_static: false,
        is_abstract: false,
        is_constructor: true,
    })
}

fn parse_function_like(
    header_text: &str,
    decorators: Vec<Decorator>,
    is_static: bool,
    is_abstract: bool,
    is_constructor: bool,
) -> Option<MethodDef> {
    let lines = header_text.lines().collect::<Vec<_>>();
    let header = lines.first()?.trim();
    let open_paren = header.find('(')?;
    let name = header[..open_paren].trim().to_owned();
    let after_name = &header[open_paren..];
    let params = extract_paren_contents(after_name)?;
    let return_type = header
        .rsplit_once("):")
        .map(|(_, tail)| tail.trim().to_owned())
        .filter(|tail| !tail.is_empty());
    let body = if is_abstract {
        None
    } else if lines.len() > 1 {
        Some(lines[1..lines.len().saturating_sub(1)].join("\n"))
    } else {
        Some(String::new())
    };

    Some(MethodDef {
        name,
        params,
        return_type,
        body,
        decorators,
        is_static,
        is_abstract,
        is_constructor,
    })
}

fn statement_decorator(statement: &Statement) -> Option<Decorator> {
    match &statement.node {
        StatementNode::Text(text) => parse_decorator(text.trim()),
        _ => None,
    }
}

fn parse_decorator(text: &str) -> Option<Decorator> {
    let trimmed = text.trim();
    let rest = trimmed.strip_prefix('@')?;
    let name_end = rest
        .char_indices()
        .find(|(_, ch)| !(*ch == '_' || ch.is_ascii_alphanumeric()))
        .map(|(index, _)| index)
        .unwrap_or(rest.len());
    let name = rest[..name_end].trim().to_owned();
    if name.is_empty() {
        return None;
    }
    let args = rest[name_end..]
        .trim()
        .strip_prefix('(')
        .and_then(|tail| tail.strip_suffix(')'))
        .map(|tail| tail.trim().to_owned());
    Some(Decorator { name, args })
}

fn statement_source(statement: &Statement) -> Option<String> {
    match &statement.node {
        StatementNode::Trivia(text) | StatementNode::Text(text) => {
            Some(format!("{text}{}", statement.trailing))
        }
        _ => None,
    }
}

fn method_decorator<'a>(method: &'a MethodDef, name: &str) -> Option<&'a Decorator> {
    method
        .decorators
        .iter()
        .find(|decorator| decorator.name == name)
}

fn class_has_decorator(class: &ClassDef, name: &str) -> bool {
    class.decorators.iter().any(|decorator| decorator.name == name)
}

fn has_custom_decorators(decorators: &[Decorator]) -> bool {
    decorators
        .iter()
        .any(|decorator| !is_builtin_decorator(decorator.name.as_str()))
}

fn has_custom_class_decorators(class: &ClassDef) -> bool {
    has_custom_decorators(&class.decorators)
        || class.fields.iter().any(|field| has_custom_decorators(&field.decorators))
        || class.methods.iter().any(|method| has_custom_decorators(&method.decorators))
}

fn is_builtin_decorator(name: &str) -> bool {
    matches!(
        name,
        "singleton" | "memoize" | "deprecated" | "readonly" | "sealed" | "abstract"
    )
}

fn emit_custom_class_decorator(class_name: &str, decorator: &Decorator) -> String {
    let args = decorator
        .args
        .as_ref()
        .map(|args| format!(", {args}"))
        .unwrap_or_default();
    format!(
        "if {0}.{1} then\n    local _decorated = {0}.{1}({2}, \"{2}\"{3})\n    if _decorated ~= nil then\n        {2} = _decorated\n    end\nend\n",
        DECORATOR_REGISTRY_NAME,
        decorator.name,
        class_name,
        args
    )
}

fn emit_custom_method_decorator(class_name: &str, method_name: &str, decorator: &Decorator) -> String {
    let args = decorator
        .args
        .as_ref()
        .map(|args| format!(", {args}"))
        .unwrap_or_default();
    format!(
        "if {0}.{1} then\n    local _decorated = {0}.{1}({2}.{3}, \"{2}\", \"{3}\"{4})\n    if _decorated ~= nil then\n        {2}.{3} = _decorated\n    end\nend\n",
        DECORATOR_REGISTRY_NAME,
        decorator.name,
        class_name,
        method_name,
        args
    )
}

fn emit_custom_property_decorator(class_name: &str, field_name: &str, decorator: &Decorator) -> String {
    let args = decorator
        .args
        .as_ref()
        .map(|args| format!(", {args}"))
        .unwrap_or_default();
    format!(
        "if {0}.{1} then\n    {0}.{1}({2}, \"{3}\"{4})\nend\n",
        DECORATOR_REGISTRY_NAME,
        decorator.name,
        class_name,
        field_name,
        args
    )
}

fn memoized_method_body(class_name: &str, method_name: &str, params: &str, body: &str) -> String {
    let cache_name = format!("_{}_{}_cache", class_name, method_name);
    let key_expr = build_memoize_key(params);
    format!(
        "local _xluau_key = {key_expr}\nif {cache_name}[_xluau_key] ~= nil then\n    return {cache_name}[_xluau_key]\nend\nlocal _xluau_result = (function()\n{body}\nend)()\n{cache_name}[_xluau_key] = _xluau_result\nreturn _xluau_result"
    )
}

fn build_memoize_key(params: &str) -> String {
    let names = split_top_level_commas(params)
        .into_iter()
        .filter_map(|param| {
            let trimmed = param.trim();
            if trimmed.is_empty() || trimmed == "self" || trimmed.starts_with("self:") {
                return None;
            }
            let before_type = split_top_level_once(trimmed, ':')
                .map(|(name, _)| name)
                .unwrap_or_else(|| trimmed.to_owned());
            let name = before_type.trim();
            if name == "..." {
                Some("table.concat({ ... }, \"::\")".to_owned())
            } else {
                Some(format!("tostring({})", name))
            }
        })
        .collect::<Vec<_>>();

    if names.is_empty() {
        "\"__memoized__\"".to_owned()
    } else {
        format!("table.concat({{{}}}, \"::\")", names.join(", "))
    }
}

fn rewrite_super_method_calls(body: &str, parent: &str) -> String {
    let mut output = String::new();
    let chars = body.chars().collect::<Vec<_>>();
    let mut index = 0usize;

    while index < chars.len() {
        if starts_with_word(&chars, index, "super.") {
            let mut cursor = index + "super.".len();
            let mut name = String::new();
            while cursor < chars.len() && (chars[cursor] == '_' || chars[cursor].is_ascii_alphanumeric()) {
                name.push(chars[cursor]);
                cursor += 1;
            }
            if cursor < chars.len() && chars[cursor] == '(' {
                output.push_str(parent);
                output.push('.');
                output.push_str(name.as_str());
                output.push_str("(self");
                cursor += 1;
                if cursor < chars.len() && chars[cursor] != ')' {
                    output.push_str(", ");
                }
                index = cursor;
                continue;
            }
        }

        output.push(chars[index]);
        index += 1;
    }

    output
}

fn extract_super_constructor_call(body: &str) -> (Option<String>, String) {
    let mut lines = body.lines().collect::<Vec<_>>();
    let Some((index, line)) = lines
        .iter()
        .enumerate()
        .find(|(_, line)| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with("--")
        })
    else {
        return (None, body.to_owned());
    };

    let trimmed = line.trim();
    if !(trimmed.starts_with("super(") && trimmed.ends_with(')')) {
        return (None, body.to_owned());
    }

    let args = trimmed
        .strip_prefix("super(")
        .and_then(|tail| tail.strip_suffix(')'))
        .map(|tail| tail.trim().to_owned());
    lines.remove(index);
    (args, lines.join("\n"))
}

fn member_starts_block(trimmed: &str) -> bool {
    (trimmed.starts_with("function ")
        || trimmed.starts_with("static function ")
        || trimmed.starts_with("constructor("))
        && !trimmed.starts_with("abstract function ")
}

fn block_delta_for_line(line: &str) -> isize {
    let source = SourceFile::virtual_file(
        PathBuf::from("__phase5_line.xl"),
        SourceKind::XLuau,
        line.to_owned(),
    );
    let tokens = Lexer::new(&source).lex(&mut Vec::new());
    let mut delta = 0isize;
    let mut significant = Vec::new();
    for token in tokens
        .into_iter()
        .filter(|token| !token.is_trivia() && token.kind != TokenKind::Eof)
    {
        significant.push(token);
    }

    if significant.is_empty() {
        return 0;
    }

    if line.trim().starts_with("constructor(") {
        delta += 1;
    }

    for token in significant {
        match token.kind {
            TokenKind::Keyword(Keyword::If)
            | TokenKind::Keyword(Keyword::For)
            | TokenKind::Keyword(Keyword::While)
            | TokenKind::Keyword(Keyword::Repeat)
            | TokenKind::Keyword(Keyword::Switch)
            | TokenKind::Keyword(Keyword::Function) => delta += 1,
            TokenKind::Keyword(Keyword::End) | TokenKind::Keyword(Keyword::Until) => delta -= 1,
            _ => {}
        }
    }

    delta
}

fn extract_paren_contents(text: &str) -> Option<String> {
    let start = text.find('(')?;
    let end = text.rfind(')')?;
    Some(text[start + 1..end].trim().to_owned())
}

fn instance_method_type(class_name: &str, method: &MethodDef) -> String {
    format!(
        "(self: {class_name}{params}) -> {ret}",
        params = if method.params.trim().is_empty() {
            String::new()
        } else {
            format!(", {}", method.params)
        },
        ret = method.return_type.as_deref().unwrap_or("nil")
    )
}

fn static_method_type(_class_name: &str, method: &MethodDef) -> String {
    format!(
        "({}) -> {}",
        method.params,
        method.return_type.as_deref().unwrap_or("nil")
    )
}

fn split_keyword<'a>(text: &'a str, keyword: &str) -> Option<(&'a str, &'a str)> {
    let needle = format!(" {keyword} ");
    let index = text.find(needle.as_str())?;
    Some((&text[..index], &text[index + needle.len()..]))
}

fn split_top_level_commas(text: &str) -> Vec<String> {
    split_top_level_with_separator(text, ',')
}

fn split_top_level_with_separator(text: &str, separator: char) -> Vec<String> {
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

fn indent_block(text: &str, prefix: &str) -> String {
    if text.trim().is_empty() {
        return String::new();
    }

    text.lines()
        .map(|line| format!("{prefix}{line}\n"))
        .collect::<String>()
}

fn escape_string(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
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
    true
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::PhaseFiveTransformer;
    use crate::config::XLuauConfig;
    use crate::lowering::Lowerer;
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::phase4::PhaseFourTransformer;
    use crate::source::{SourceFile, SourceKind};

    fn transform(text: &str) -> (String, Vec<crate::diagnostic::Diagnostic>) {
        let source =
            SourceFile::virtual_file(PathBuf::from("test.xl"), SourceKind::XLuau, text.to_owned());
        let mut diagnostics = Vec::new();
        let tokens = Lexer::new(&source).lex(&mut diagnostics);
        let program = Parser::new(&source, &tokens).parse(&mut diagnostics);
        assert!(
            diagnostics.iter().all(|diagnostic| !diagnostic.is_error()),
            "{diagnostics:?}"
        );
        let output = PhaseFiveTransformer::new(XLuauConfig::default())
            .transform_program(&source, &program, &mut diagnostics);
        let transformed = SourceFile::virtual_file(
            PathBuf::from("test.luau"),
            SourceKind::Luau,
            output.clone(),
        );
        let reparsed_tokens = Lexer::new(&transformed).lex(&mut diagnostics);
        let _ = Parser::new(&transformed, &reparsed_tokens).parse(&mut diagnostics);
        assert!(
            diagnostics.iter().all(|diagnostic| !diagnostic.is_error()),
            "{diagnostics:?}\n{output}"
        );
        (output, diagnostics)
    }

    #[test]
    fn transforms_basic_class_and_static_method() {
        let (output, diagnostics) = transform(
            "class Animal {\n    name: string\n    constructor(name: string)\n        self.name = name\n    end\n    function speak(): string\n        return self.name\n    end\n    static function create(name: string): Animal\n        return Animal.new(name)\n    end\n}\n",
        );

        assert!(diagnostics.iter().all(|diagnostic| !diagnostic.is_error()));
        assert!(output.contains("type Animal = {"));
        assert!(output.contains("local Animal: AnimalClass = {} :: AnimalClass"));
        assert!(output.contains("function Animal.new(name: string): Animal"));
        assert!(output.contains("function Animal:speak(): string"));
        assert!(output.contains("function Animal.create(name: string): Animal"));
    }

    #[test]
    fn transforms_inheritance_and_super_calls() {
        let (output, diagnostics) = transform(
            "class Animal {\n    constructor(name: string)\n        self.name = name\n    end\n    function speak(): string\n        return self.name\n    end\n}\nclass Dog extends Animal {\n    constructor(name: string)\n        super(name)\n        self.kind = super.speak()\n    end\n}\n",
        );

        assert!(diagnostics.iter().all(|diagnostic| !diagnostic.is_error()));
        assert!(output.contains("setmetatable(Dog, { __index = Animal })"));
        assert!(output.contains("local self = Animal.new(name) :: Dog"));
        assert!(output.contains("self.kind = Animal.speak(self)"));
    }

    #[test]
    fn validates_interfaces_abstract_classes_and_readonly() {
        let source = SourceFile::virtual_file(
            PathBuf::from("test.xl"),
            SourceKind::XLuau,
            "interface Serializable {\n    serialize: (self: Serializable) -> string\n}\nabstract class Base {\n    abstract function serialize(): string\n}\nclass Broken extends Base implements Serializable {\n    @readonly\n    id: string\n    function mutate()\n        self.id = \"nope\"\n    end\n}\nlocal item = Base.new()\n".to_owned(),
        );
        let mut diagnostics = Vec::new();
        let tokens = Lexer::new(&source).lex(&mut diagnostics);
        let program = Parser::new(&source, &tokens).parse(&mut diagnostics);
        assert!(diagnostics.iter().all(|diagnostic| !diagnostic.is_error()));
        let _ = PhaseFiveTransformer::new(XLuauConfig::default())
            .transform_program(&source, &program, &mut diagnostics);

        assert!(diagnostics.iter().any(|diagnostic| diagnostic.message.contains("must implement abstract method")));
        assert!(diagnostics.iter().any(|diagnostic| diagnostic.message.contains("readonly property")));
        assert!(diagnostics.iter().any(|diagnostic| diagnostic.message.contains("cannot be instantiated directly")));
    }

    #[test]
    fn applies_builtin_and_custom_decorators() {
        let mut config = XLuauConfig::default();
        config.decorator_library = Some(PathBuf::from("./decorators"));

        let source = SourceFile::virtual_file(
            PathBuf::from("test.xl"),
            SourceKind::XLuau,
            "@singleton\nclass Cache {\n    @memoize\n    @deprecated(\"old\")\n    function get(key: string): string\n        return key\n    end\n    @trace\n    static function warm(): nil\n        return nil\n    end\n}\n".to_owned(),
        );
        let mut diagnostics = Vec::new();
        let tokens = Lexer::new(&source).lex(&mut diagnostics);
        let program = Parser::new(&source, &tokens).parse(&mut diagnostics);
        let output = PhaseFiveTransformer::new(config).transform_program(&source, &program, &mut diagnostics);

        assert!(diagnostics.iter().all(|diagnostic| !diagnostic.is_error()));
        assert!(output.contains("local _xluau_decorators = require(\"./decorators\")"));
        assert!(output.contains("local _Cache_singleton: Cache? = nil"));
        assert!(output.contains("local _Cache_get_cache = {}"));
        assert!(output.contains("warn(\"old\")"));
        assert!(output.contains("_xluau_decorators.trace"));
    }

    #[test]
    fn combined_phase_five_output_reparses() {
        let mut config = XLuauConfig::default();
        config.decorator_library = Some(PathBuf::from("./decorators"));
        let source = SourceFile::virtual_file(
            PathBuf::from("test.xl"),
            SourceKind::XLuau,
            "interface Serializable {\n    serialize: (self: Serializable) -> string\n}\n\nabstract class Animal implements Serializable {\n    @readonly\n    name: string\n\n    constructor(name: string)\n        self.name = name\n    end\n\n    abstract function serialize(): string\n\n    function speak(): string\n        return self.name\n    end\n}\n\n@singleton\nclass Dog extends Animal {\n    breed: string\n\n    constructor(name: string, breed: string)\n        super(name)\n        self.breed = breed\n    end\n\n    @deprecated(\"use serialize\")\n    @memoize\n    function serialize(): string\n        return self.name .. \":\" .. self.breed\n    end\n\n    @trace\n    static function create(name: string, breed: string): Dog\n        return Dog.new(name, breed)\n    end\n}\n\nlocal pet = Dog.create(\"Fido\", \"Collie\")\nprint(pet:serialize())\n".to_owned(),
        );
        let mut diagnostics = Vec::new();
        let tokens = Lexer::new(&source).lex(&mut diagnostics);
        let program = Parser::new(&source, &tokens).parse(&mut diagnostics);
        assert!(diagnostics.iter().all(|diagnostic| !diagnostic.is_error()));
        let output =
            PhaseFiveTransformer::new(config).transform_program(&source, &program, &mut diagnostics);
        let transformed =
            SourceFile::virtual_file(PathBuf::from("test.luau"), SourceKind::Luau, output.clone());
        let reparsed_tokens = Lexer::new(&transformed).lex(&mut diagnostics);
        let _ = Parser::new(&transformed, &reparsed_tokens).parse(&mut diagnostics);

        assert!(diagnostics.iter().all(|diagnostic| !diagnostic.is_error()));
        assert!(output.contains("type Dog = Animal & {"));
        assert!(output.contains("function Dog.create(name: string, breed: string): Dog"));
    }

    #[test]
    fn combined_phase_five_pipeline_survives_lowering_and_phase_four() {
        let mut config = XLuauConfig::default();
        config.decorator_library = Some(PathBuf::from("./decorators"));
        let source = SourceFile::virtual_file(
            PathBuf::from("test.xl"),
            SourceKind::XLuau,
            "interface Serializable {\n    serialize: (self: Serializable) -> string\n}\n\nabstract class Animal implements Serializable {\n    @readonly\n    name: string\n\n    constructor(name: string)\n        self.name = name\n    end\n\n    abstract function serialize(): string\n\n    function speak(): string\n        return self.name\n    end\n}\n\n@singleton\nclass Dog extends Animal {\n    breed: string\n\n    constructor(name: string, breed: string)\n        super(name)\n        self.breed = breed\n    end\n\n    @deprecated(\"use serialize\")\n    @memoize\n    function serialize(): string\n        return self.name .. \":\" .. self.breed\n    end\n\n    @trace\n    static function create(name: string, breed: string): Dog\n        return Dog.new(name, breed)\n    end\n}\n\nlocal pet = Dog.create(\"Fido\", \"Collie\")\nprint(pet:serialize())\n".to_owned(),
        );
        let mut diagnostics = Vec::new();
        let tokens = Lexer::new(&source).lex(&mut diagnostics);
        let program = Parser::new(&source, &tokens).parse(&mut diagnostics);
        assert!(diagnostics.iter().all(|diagnostic| !diagnostic.is_error()), "{diagnostics:?}");

        let phase_five = PhaseFiveTransformer::new(config.clone())
            .transform_program(&source, &program, &mut diagnostics);
        let phase_five_source =
            SourceFile::virtual_file(PathBuf::from("test.xl"), SourceKind::XLuau, phase_five);
        let phase_five_tokens = Lexer::new(&phase_five_source).lex(&mut diagnostics);
        let phase_five_program = Parser::new(&phase_five_source, &phase_five_tokens).parse(&mut diagnostics);
        assert!(diagnostics.iter().all(|diagnostic| !diagnostic.is_error()), "{diagnostics:?}");

        let lowered =
            Lowerer::new().lower_program(&phase_five_source, &phase_five_program, &mut diagnostics);
        let lowered_source =
            SourceFile::virtual_file(PathBuf::from("test.luau"), SourceKind::Luau, lowered);
        let lowered_tokens = Lexer::new(&lowered_source).lex(&mut diagnostics);
        let lowered_program = Parser::new(&lowered_source, &lowered_tokens).parse(&mut diagnostics);
        assert!(
            diagnostics.iter().all(|diagnostic| !diagnostic.is_error()),
            "{diagnostics:?}\n{}",
            lowered_source.text
        );

        let phase_four = PhaseFourTransformer::new(config)
            .transform_program(&lowered_source, &lowered_program, &mut diagnostics);
        let phase_four_source =
            SourceFile::virtual_file(PathBuf::from("test.luau"), SourceKind::Luau, phase_four);
        let phase_four_tokens = Lexer::new(&phase_four_source).lex(&mut diagnostics);
        let _ = Parser::new(&phase_four_source, &phase_four_tokens).parse(&mut diagnostics);
        assert!(diagnostics.iter().all(|diagnostic| !diagnostic.is_error()), "{diagnostics:?}");
    }
}
