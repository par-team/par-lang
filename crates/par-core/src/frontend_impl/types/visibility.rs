use super::{Type, TypeError, visit};
use crate::frontend_impl::language::{GlobalName, Universal};
use crate::frontend_impl::process;
use crate::frontend_impl::program::{DefinitionBody, Module};
use crate::location::{FileName, Span};
use crate::workspace::FileImportScope;
use indexmap::IndexSet;
use std::collections::{BTreeMap, HashMap, btree_map::Entry};
use std::fmt::{self, Display, Formatter};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Visibility {
    Module,
    Package,
    Public,
}

impl Display for Visibility {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Module => write!(f, "available only inside its module"),
            Self::Package => write!(f, "exported, but only inside its package"),
            Self::Public => write!(f, "exported from its package"),
        }
    }
}

#[derive(Debug, Clone)]
struct ModuleExportState {
    exported: bool,
    span: Span,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct VisibilityIndex {
    modules: BTreeMap<Universal, ModuleExportState>,
    module_export_mismatches: Vec<(Span, Span, Universal)>,
    type_defs: BTreeMap<GlobalName<Universal>, Visibility>,
    declarations: BTreeMap<GlobalName<Universal>, Visibility>,
}

impl VisibilityIndex {
    pub(crate) fn record_module_export(&mut self, module: &Universal, span: Span, exported: bool) {
        match self.modules.entry(module.clone()) {
            Entry::Vacant(vacant) => {
                vacant.insert(ModuleExportState { exported, span });
            }
            Entry::Occupied(mut occupied) => {
                let state = occupied.get_mut();
                if state.exported != exported {
                    self.module_export_mismatches.push((
                        span.clone(),
                        state.span.clone(),
                        module.clone(),
                    ));
                    state.exported |= exported;
                }
            }
        }
    }

    fn module_exported(&self, module: &Universal) -> bool {
        self.modules.get(module).is_some_and(|state| state.exported)
    }

    fn item_visibility(&self, module: &Universal, exported: bool) -> Visibility {
        if !exported {
            Visibility::Module
        } else if self.module_exported(module) {
            Visibility::Public
        } else {
            Visibility::Package
        }
    }

    pub(crate) fn record_module(
        &mut self,
        module: &Module<Arc<process::Expression<(), Universal>>, Universal>,
    ) {
        for type_def in &module.type_defs {
            let visibility = self.item_visibility(&type_def.name.module, type_def.exported);
            self.type_defs
                .entry(type_def.name.clone())
                .and_modify(|current| {
                    *current = (*current).max(visibility);
                })
                .or_insert(visibility);
        }
        for declaration in &module.declarations {
            let visibility = self.item_visibility(&declaration.name.module, declaration.exported);
            self.declarations
                .entry(declaration.name.clone())
                .and_modify(|current| {
                    *current = (*current).max(visibility);
                })
                .or_insert(visibility);
        }
        for definition in &module.definitions {
            self.declarations
                .entry(definition.name.clone())
                .or_insert(Visibility::Module);
        }
    }

    fn type_visibility(&self, name: &GlobalName<Universal>) -> Visibility {
        self.type_defs
            .get(name)
            .copied()
            .unwrap_or(Visibility::Module)
    }

    pub(crate) fn declaration_visibility(&self, name: &GlobalName<Universal>) -> Visibility {
        self.declarations
            .get(name)
            .copied()
            .unwrap_or(Visibility::Module)
    }

    fn contains_type(&self, name: &GlobalName<Universal>) -> bool {
        self.type_defs.contains_key(name)
    }

    fn contains_declaration(&self, name: &GlobalName<Universal>) -> bool {
        self.declarations.contains_key(name)
    }

    fn required_visibility(from: &Universal, target: &Universal) -> Visibility {
        if from == target {
            Visibility::Module
        } else if from.package == target.package {
            Visibility::Package
        } else {
            Visibility::Public
        }
    }

    fn module_visible_from(&self, from: &Universal, target: &Universal) -> bool {
        from.package == target.package || self.module_exported(target)
    }

    pub(crate) fn type_visible_from(
        &self,
        from: &Universal,
        target: &GlobalName<Universal>,
    ) -> bool {
        self.type_visibility(target) >= Self::required_visibility(from, &target.module)
    }

    pub(crate) fn declaration_visible_from(
        &self,
        from: &Universal,
        target: &GlobalName<Universal>,
    ) -> bool {
        self.declaration_visibility(target) >= Self::required_visibility(from, &target.module)
    }
}

pub(crate) fn validate_visibility(
    lowered: &Module<Arc<process::Expression<(), Universal>>, Universal>,
    visibility: &VisibilityIndex,
    file_scopes: &HashMap<FileName, FileImportScope<Universal>>,
    import_spans: &HashMap<FileName, Vec<(Span, Universal)>>,
) -> Vec<TypeError<Universal>> {
    let mut errors = IndexSet::new();

    for (span, other_span, module) in &visibility.module_export_mismatches {
        errors.insert(TypeError::ModuleExportInconsistent(
            span.clone(),
            other_span.clone(),
            module.clone(),
        ));
    }

    for (file, imports) in import_spans {
        let Some(scope) = file_scopes.get(file) else {
            continue;
        };
        for (span, imported_module) in imports {
            if !visibility.module_visible_from(&scope.current_module, imported_module) {
                errors.insert(TypeError::ImportedModuleNotExported(
                    span.clone(),
                    imported_module.clone(),
                ));
            }
        }
    }

    for type_def in &lowered.type_defs {
        validate_type_visibility_in_type(
            &type_def.name.module,
            &type_def.typ,
            visibility,
            &mut errors,
        );

        let item_visibility = visibility.type_visibility(&type_def.name);
        if item_visibility > Visibility::Module {
            validate_exposed_type_visibility(
                &type_def.name,
                item_visibility,
                &type_def.typ,
                visibility,
                &mut errors,
            );
        }
    }

    for declaration in &lowered.declarations {
        validate_type_visibility_in_type(
            &declaration.name.module,
            &declaration.typ,
            visibility,
            &mut errors,
        );

        let item_visibility = visibility.declaration_visibility(&declaration.name);
        if item_visibility > Visibility::Module {
            validate_exposed_type_visibility(
                &declaration.name,
                item_visibility,
                &declaration.typ,
                visibility,
                &mut errors,
            );
        }
    }

    for definition in &lowered.definitions {
        validate_definition_visibility(
            &definition.name.module,
            &definition.body,
            visibility,
            &mut errors,
        );
    }

    errors.into_iter().collect()
}

fn validate_type_visibility_in_type(
    current_module: &Universal,
    typ: &Type<Universal>,
    visibility: &VisibilityIndex,
    errors: &mut IndexSet<TypeError<Universal>>,
) {
    visit_type_global_names(typ, &mut |span, name| {
        if !visibility.contains_type(name) {
            return;
        }
        if !visibility.type_visible_from(current_module, name) {
            errors.insert(TypeError::GlobalNameNotVisible(
                span.clone(),
                name.clone(),
                visibility.type_visibility(name),
            ));
        }
    });
}

fn validate_exposed_type_visibility(
    item_name: &GlobalName<Universal>,
    item_visibility: Visibility,
    typ: &Type<Universal>,
    visibility: &VisibilityIndex,
    errors: &mut IndexSet<TypeError<Universal>>,
) {
    visit_type_global_names(typ, &mut |span, name| {
        if !visibility.contains_type(name) {
            return;
        }
        let type_visibility = visibility.type_visibility(name);
        if type_visibility < item_visibility {
            errors.insert(TypeError::VisibleItemExposesHiddenType(
                span.clone(),
                item_name.clone(),
                item_visibility,
                name.clone(),
                type_visibility,
            ));
        }
    });
}

fn validate_definition_visibility(
    current_module: &Universal,
    body: &DefinitionBody<Arc<process::Expression<(), Universal>>>,
    visibility: &VisibilityIndex,
    errors: &mut IndexSet<TypeError<Universal>>,
) {
    match body {
        DefinitionBody::Par(expression) => {
            validate_expression_visibility(current_module, expression, visibility, errors);
        }
        DefinitionBody::External(_) => {}
    }
}

fn validate_expression_visibility(
    current_module: &Universal,
    expression: &Arc<process::Expression<(), Universal>>,
    visibility: &VisibilityIndex,
    errors: &mut IndexSet<TypeError<Universal>>,
) {
    match expression.as_ref() {
        process::Expression::Global(span, name, ()) => {
            if !visibility.contains_declaration(name) {
                return;
            }
            if !visibility.declaration_visible_from(current_module, name) {
                errors.insert(TypeError::GlobalNameNotVisible(
                    span.clone(),
                    name.clone(),
                    visibility.declaration_visibility(name),
                ));
            }
        }
        process::Expression::Variable(..)
        | process::Expression::Primitive(..)
        | process::Expression::External(..)
        | process::Expression::ToDo(..) => {}
        process::Expression::Box(_, _, expression, _) => {
            validate_expression_visibility(current_module, expression, visibility, errors);
        }
        process::Expression::Chan {
            chan_annotation,
            process,
            ..
        } => {
            if let Some(annotation) = chan_annotation {
                validate_type_visibility_in_type(current_module, annotation, visibility, errors);
            }
            validate_process_visibility(current_module, process, visibility, errors);
        }
    }
}

fn validate_process_visibility(
    current_module: &Universal,
    process: &Arc<process::Process<(), Universal>>,
    visibility: &VisibilityIndex,
    errors: &mut IndexSet<TypeError<Universal>>,
) {
    for step in &process.steps {
        match step {
            process::Step::Let {
                annotation, value, ..
            } => {
                if let Some(annotation) = annotation {
                    validate_type_visibility_in_type(
                        current_module,
                        annotation,
                        visibility,
                        errors,
                    );
                }
                validate_expression_visibility(current_module, value, visibility, errors);
            }
            process::Step::Do { command, .. } => {
                validate_command_visibility(current_module, command, visibility, errors);
            }
        }
    }

    match &process.terminator {
        process::Terminator::Do { command, .. } => {
            validate_terminal_command_visibility(current_module, command, visibility, errors);
        }
        process::Terminator::Poll {
            clients,
            then,
            else_,
            ..
        } => {
            for client in clients {
                validate_expression_visibility(current_module, client, visibility, errors);
            }
            validate_process_visibility(current_module, then, visibility, errors);
            validate_process_visibility(current_module, else_, visibility, errors);
        }
        process::Terminator::Submit { values, .. } => {
            for value in values {
                validate_expression_visibility(current_module, value, visibility, errors);
            }
        }
        process::Terminator::Block(_, _, body, then) => {
            validate_process_visibility(current_module, body, visibility, errors);
            validate_process_visibility(current_module, then, visibility, errors);
        }
        process::Terminator::Goto(..)
        | process::Terminator::Unreachable(..)
        | process::Terminator::ToDo(..) => {}
    }
}

fn validate_command_visibility(
    current_module: &Universal,
    command: &process::Command<(), Universal>,
    visibility: &VisibilityIndex,
    errors: &mut IndexSet<TypeError<Universal>>,
) {
    match command {
        process::Command::Noop | process::Command::Continue => {}
        process::Command::Send(argument) => {
            validate_expression_visibility(current_module, argument, visibility, errors);
        }
        process::Command::Receive(_, annotation, _, _) => {
            if let Some(annotation) = annotation {
                validate_type_visibility_in_type(current_module, annotation, visibility, errors);
            }
        }
        process::Command::Signal(_) | process::Command::ReceiveType(_) => {}
        process::Command::SendType(argument) => {
            validate_type_visibility_in_type(current_module, argument, visibility, errors);
        }
    }
}

fn validate_terminal_command_visibility(
    current_module: &Universal,
    command: &process::TerminalCommand<(), Universal>,
    visibility: &VisibilityIndex,
    errors: &mut IndexSet<TypeError<Universal>>,
) {
    match command {
        process::TerminalCommand::Link(expression) => {
            validate_expression_visibility(current_module, expression, visibility, errors);
        }
        process::TerminalCommand::Case(_, processes, else_process) => {
            for process in processes {
                validate_process_visibility(current_module, process, visibility, errors);
            }
            if let Some(process) = else_process {
                validate_process_visibility(current_module, process, visibility, errors);
            }
        }
        process::TerminalCommand::Break | process::TerminalCommand::Loop(..) => {}
        process::TerminalCommand::Begin { body, .. } => {
            validate_process_visibility(current_module, body, visibility, errors);
        }
    }
}

fn visit_type_global_names(
    typ: &Type<Universal>,
    consume: &mut impl FnMut(&Span, &GlobalName<Universal>),
) {
    match typ {
        Type::Name(span, name, args)
        | Type::DualName(span, name, args)
        | Type::SizedName(span, _, name, args)
        | Type::SizedDualName(span, _, name, args) => {
            consume(span, name);
            for arg in args {
                visit_type_global_names(arg, consume);
            }
        }
        _ => {
            let _ = visit::continue_(typ, |child| {
                visit_type_global_names(child, consume);
                Ok::<(), ()>(())
            });
        }
    }
}
