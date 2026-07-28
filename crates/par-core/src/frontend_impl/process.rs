pub use super::captures::{Captures, VariableUsage};
use super::{
    language::{GlobalName, LocalName, TypeParameter},
    types::{GlobalNameWriter, Type},
};
use crate::{
    frontend_impl::program::{CheckedModule, DocComment, Docs},
    location::{Span, Spanning},
};
use indexmap::IndexSet;
use par_runtime::linker::Unlinked;
use par_runtime::primitive::Primitive;
use std::{
    fmt::{self, Write},
    sync::Arc,
};

#[derive(Clone, Debug)]
pub enum PollKind {
    Poll,
    Repoll,
}

#[derive(Clone, Debug)]
pub struct Process<Typ, S> {
    pub steps: Vec<Step<Typ, S>>,
    pub terminator: Terminator<Typ, S>,
}

#[derive(Clone, Debug, Default)]
pub struct ProcessBuilder<Typ, S> {
    steps: Vec<Step<Typ, S>>,
}

#[derive(Clone, Debug)]
pub enum Step<Typ, S> {
    Let {
        span: Span,
        name: LocalName,
        annotation: Option<Type<S>>,
        typ: Typ,
        value: Arc<Expression<Typ, S>>,
    },
    Do {
        span: Span,
        name: LocalName,
        usage: VariableUsage,
        typ: Typ,
        command: Command<Typ, S>,
    },
}

#[derive(Clone, Debug)]
pub enum Terminator<Typ, S> {
    Do {
        span: Span,
        name: LocalName,
        usage: VariableUsage,
        typ: Typ,
        command: TerminalCommand<Typ, S>,
    },
    Poll {
        span: Span,
        kind: PollKind,
        driver: LocalName,
        point: LocalName,
        clients: Vec<Arc<Expression<Typ, S>>>,
        name: LocalName,
        name_typ: Typ,
        captures: Captures,
        then: Arc<Process<Typ, S>>,
        else_: Arc<Process<Typ, S>>,
    },
    Submit {
        span: Span,
        driver: LocalName,
        point: LocalName,
        values: Vec<Arc<Expression<Typ, S>>>,
        captures: Captures,
    },
    Block(Span, usize, Arc<Process<Typ, S>>, Arc<Process<Typ, S>>),
    Goto(Span, usize, Captures),
    Unreachable(Span),
    ToDo(Span),
}

#[derive(Clone, Debug)]
pub enum Command<Typ, S> {
    /// Validate and consume a bare command subject before continuing. This has no runtime
    /// protocol effect, but preserves linearity diagnostics and hover information for source
    /// command chains of the form `subject; next`.
    Noop,
    Send(Arc<Expression<Typ, S>>),
    Receive(LocalName, Option<Type<S>>, Typ, Vec<TypeParameter>),
    Signal(LocalName),
    Continue,
    SendType(Type<S>),
    ReceiveType(TypeParameter),
}

#[derive(Clone, Debug)]
pub enum TerminalCommand<Typ, S> {
    Link(Arc<Expression<Typ, S>>),
    Case(
        Arc<[LocalName]>,
        Box<[Arc<Process<Typ, S>>]>,
        Option<Arc<Process<Typ, S>>>,
    ),
    Break,
    Begin {
        unfounded: bool,
        label: Option<LocalName>,
        captures: Captures,
        body: Arc<Process<Typ, S>>,
    },
    Loop(Option<LocalName>, LocalName, Captures),
}

#[derive(Clone, Debug)]
pub enum Expression<Typ, S> {
    Global(Span, GlobalName<S>, Typ),
    Variable(Span, LocalName, Typ, VariableUsage),
    Box(Span, Captures, Arc<Self>, Typ),
    Chan {
        span: Span,
        captures: Captures,
        chan_name: LocalName,
        chan_annotation: Option<Type<S>>,
        chan_type: Typ,
        expr_type: Typ,
        process: Arc<Process<Typ, S>>,
    },
    Primitive(Span, Primitive, Typ),
    External(Unlinked, Typ),
}

impl<Typ, S> Spanning for Process<Typ, S> {
    fn span(&self) -> Span {
        match self.steps.first() {
            Some(Step::Let { span, .. } | Step::Do { span, .. }) => span.clone(),
            None => self.terminator.span(),
        }
    }
}

impl<Typ, S> Spanning for Terminator<Typ, S> {
    fn span(&self) -> Span {
        match self {
            Self::Do { span, .. } => span.clone(),
            Self::Poll { span, .. } => span.clone(),
            Self::Submit { span, .. } => span.clone(),
            Self::Block(span, _, _, _) => span.clone(),
            Self::Goto(span, _, _) => span.clone(),
            Self::Unreachable(span) => span.clone(),
            Self::ToDo(span) => span.clone(),
        }
    }
}

impl<Typ, S> Process<Typ, S> {
    pub fn new(steps: Vec<Step<Typ, S>>, terminator: Terminator<Typ, S>) -> Self {
        Self { steps, terminator }
    }

    pub fn terminal(terminator: Terminator<Typ, S>) -> Arc<Self> {
        Arc::new(Self::new(Vec::new(), terminator))
    }

    pub fn prepend(step: Step<Typ, S>, process: Arc<Self>) -> Arc<Self>
    where
        Typ: Clone,
        S: Clone,
    {
        let process = Arc::unwrap_or_clone(process);
        let mut steps = Vec::with_capacity(process.steps.len() + 1);
        steps.push(step);
        steps.extend(process.steps);
        Arc::new(Self::new(steps, process.terminator))
    }

    pub fn let_step(
        span: Span,
        name: LocalName,
        annotation: Option<Type<S>>,
        typ: Typ,
        value: Arc<Expression<Typ, S>>,
        then: Arc<Self>,
    ) -> Arc<Self>
    where
        Typ: Clone,
        S: Clone,
    {
        Self::prepend(
            Step::Let {
                span,
                name,
                annotation,
                typ,
                value,
            },
            then,
        )
    }

    pub fn do_step(
        span: Span,
        name: LocalName,
        usage: VariableUsage,
        typ: Typ,
        command: Command<Typ, S>,
        then: Arc<Self>,
    ) -> Arc<Self>
    where
        Typ: Clone,
        S: Clone,
    {
        Self::prepend(
            Step::Do {
                span,
                name,
                usage,
                typ,
                command,
            },
            then,
        )
    }

    pub fn do_terminal(
        span: Span,
        name: LocalName,
        usage: VariableUsage,
        typ: Typ,
        command: TerminalCommand<Typ, S>,
    ) -> Arc<Self> {
        Self::terminal(Terminator::Do {
            span,
            name,
            usage,
            typ,
            command,
        })
    }

    pub fn todo(span: Span) -> Arc<Self> {
        Self::terminal(Terminator::ToDo(span))
    }
}

impl<Typ, S> ProcessBuilder<Typ, S> {
    pub fn new() -> Self {
        Self { steps: Vec::new() }
    }

    pub fn push(&mut self, step: Step<Typ, S>) {
        self.steps.push(step);
    }

    pub fn finish(self, terminator: Terminator<Typ, S>) -> Arc<Process<Typ, S>> {
        Arc::new(Process::new(self.steps, terminator))
    }

    pub fn finish_with(self, process: Arc<Process<Typ, S>>) -> Arc<Process<Typ, S>>
    where
        Typ: Clone,
        S: Clone,
    {
        let process = Arc::unwrap_or_clone(process);
        let mut steps = self.steps;
        steps.extend(process.steps);
        Arc::new(Process::new(steps, process.terminator))
    }
}

impl<S: Clone> Process<(), S> {
    pub fn optimize(&self) -> Arc<Self> {
        let mut steps = self
            .steps
            .iter()
            .map(|step| match step {
                Step::Let {
                    span,
                    name,
                    annotation,
                    typ,
                    value,
                } => Step::Let {
                    span: span.clone(),
                    name: name.clone(),
                    annotation: annotation.clone(),
                    typ: *typ,
                    value: value.optimize(),
                },
                Step::Do {
                    span,
                    name,
                    usage,
                    typ,
                    command,
                } => Step::Do {
                    span: span.clone(),
                    name: name.clone(),
                    usage: usage.clone(),
                    typ: *typ,
                    command: match command {
                        Command::Noop => Command::Noop,
                        Command::Send(argument) => Command::Send(argument.optimize()),
                        Command::Receive(parameter, annotation, typ, vars) => Command::Receive(
                            parameter.clone(),
                            annotation.clone(),
                            *typ,
                            vars.clone(),
                        ),
                        Command::Signal(chosen) => Command::Signal(chosen.clone()),
                        Command::Continue => Command::Continue,
                        Command::SendType(argument) => Command::SendType(argument.clone()),
                        Command::ReceiveType(parameter) => Command::ReceiveType(parameter.clone()),
                    },
                },
            })
            .collect::<Vec<_>>();

        let terminator = match &self.terminator {
            Terminator::Do {
                span,
                name,
                usage,
                typ,
                command: TerminalCommand::Link(expression),
            } => {
                let expression = expression.optimize();
                if let Expression::Chan {
                    chan_name: channel,
                    chan_annotation: annotation,
                    process,
                    ..
                } = expression.as_ref()
                {
                    let nested = process.optimize();
                    if name != channel || annotation.is_some() {
                        steps.push(Step::Let {
                            span: span.clone(),
                            name: channel.clone(),
                            annotation: annotation.clone(),
                            typ: (),
                            value: Arc::new(Expression::Variable(
                                span.clone(),
                                name.clone(),
                                (),
                                VariableUsage::Unknown,
                            )),
                        });
                    }
                    let nested = Arc::unwrap_or_clone(nested);
                    steps.extend(nested.steps);
                    nested.terminator
                } else {
                    Terminator::Do {
                        span: span.clone(),
                        name: name.clone(),
                        usage: usage.clone(),
                        typ: *typ,
                        command: TerminalCommand::Link(expression),
                    }
                }
            }
            Terminator::Do {
                span,
                name,
                usage,
                typ,
                command,
            } => Terminator::Do {
                span: span.clone(),
                name: name.clone(),
                usage: usage.clone(),
                typ: *typ,
                command: match command {
                    TerminalCommand::Link(_) => unreachable!(),
                    TerminalCommand::Case(branches, processes, else_process) => {
                        TerminalCommand::Case(
                            Arc::clone(branches),
                            processes.iter().map(|p| p.optimize()).collect(),
                            else_process.as_ref().map(|p| p.optimize()),
                        )
                    }
                    TerminalCommand::Break => TerminalCommand::Break,
                    TerminalCommand::Begin {
                        unfounded,
                        label,
                        captures,
                        body,
                    } => TerminalCommand::Begin {
                        unfounded: *unfounded,
                        label: label.clone(),
                        captures: captures.clone(),
                        body: body.optimize(),
                    },
                    TerminalCommand::Loop(label, driver, captures) => {
                        TerminalCommand::Loop(label.clone(), driver.clone(), captures.clone())
                    }
                },
            },
            Terminator::Poll {
                span,
                kind,
                driver,
                point,
                clients,
                name,
                name_typ,
                captures,
                then,
                else_,
            } => Terminator::Poll {
                span: span.clone(),
                kind: kind.clone(),
                driver: driver.clone(),
                point: point.clone(),
                clients: clients.iter().map(|e| e.optimize()).collect(),
                name: name.clone(),
                name_typ: name_typ.clone(),
                captures: captures.clone(),
                then: then.optimize(),
                else_: else_.optimize(),
            },
            Terminator::Submit {
                span,
                driver,
                point,
                values,
                captures,
            } => Terminator::Submit {
                span: span.clone(),
                driver: driver.clone(),
                point: point.clone(),
                values: values.iter().map(|e| e.optimize()).collect(),
                captures: captures.clone(),
            },
            Terminator::Block(span, index, body, process) => {
                Terminator::Block(span.clone(), *index, body.optimize(), process.optimize())
            }
            Terminator::Goto(span, index, caps) => {
                Terminator::Goto(span.clone(), *index, caps.clone())
            }
            Terminator::Unreachable(span) => Terminator::Unreachable(span.clone()),
            Terminator::ToDo(span) => Terminator::ToDo(span.clone()),
        };

        Arc::new(Process::new(steps, terminator))
    }
}

impl<S: Clone + Eq + std::hash::Hash + std::fmt::Display> Process<Type<S>, S> {
    pub fn types_at_spans(
        &self,
        program: &CheckedModule<S>,
        docs: &Docs<S>,
        consume: &mut impl FnMut(Span, HoverInfo<S>),
    ) {
        let consume_subject =
            |span: &Span,
             name: &LocalName,
             typ: &Type<S>,
             consume: &mut dyn FnMut(Span, HoverInfo<S>)| {
                consume(name.span(), HoverInfo::named(name, typ.clone()));
                if name == &LocalName::result() {
                    consume(
                        span.clone(),
                        HoverInfo::unnamed(typ.clone().dual(Span::None)),
                    );
                } else if name == &LocalName::object() {
                    consume(span.clone(), HoverInfo::unnamed(typ.clone()));
                } else {
                    consume(span.clone(), HoverInfo::named(name, typ.clone()));
                }
            };

        for step in &self.steps {
            match step {
                Step::Let {
                    name,
                    annotation,
                    typ,
                    value,
                    ..
                } => {
                    value.types_at_spans(program, docs, consume);
                    consume(name.span(), HoverInfo::named(name, typ.clone()));
                    if let Some(annotation) = annotation {
                        annotation.types_at_spans(&program.type_defs, docs, consume);
                    }
                }
                Step::Do {
                    span,
                    name,
                    typ,
                    command,
                    ..
                } => {
                    consume_subject(span, name, typ, consume);
                    command.types_at_spans(program, docs, consume);
                }
            }
        }

        match &self.terminator {
            Terminator::Do {
                span,
                name,
                typ,
                command,
                ..
            } => {
                consume_subject(span, name, typ, consume);
                command.types_at_spans(program, docs, consume);
            }
            Terminator::Poll {
                clients,
                name,
                name_typ,
                then,
                else_,
                ..
            } => {
                for client in clients {
                    client.types_at_spans(program, docs, consume);
                }
                consume(name.span(), HoverInfo::named(name, name_typ.clone()));
                then.types_at_spans(program, docs, consume);
                else_.types_at_spans(program, docs, consume);
            }
            Terminator::Submit { values, .. } => {
                for value in values {
                    value.types_at_spans(program, docs, consume);
                }
            }
            Terminator::Block(_, _, body, process) => {
                body.types_at_spans(program, docs, consume);
                process.types_at_spans(program, docs, consume);
            }
            Terminator::Goto(_, _, _) | Terminator::Unreachable(_) | Terminator::ToDo(_) => {}
        }
    }
}

impl<Typ, S> Command<Typ, S> {
    pub fn free_variables(&self) -> IndexSet<LocalName> {
        match self {
            Command::Noop => IndexSet::new(),
            Command::Send(argument) => argument.free_variables(),
            Command::Receive(..)
            | Command::Signal(_)
            | Command::Continue
            | Command::SendType(_)
            | Command::ReceiveType(_) => IndexSet::new(),
        }
    }
}

impl<Typ, S> TerminalCommand<Typ, S> {
    pub fn free_variables(&self) -> IndexSet<LocalName> {
        match self {
            TerminalCommand::Link(expression) => expression.free_variables(),
            TerminalCommand::Case(_, processes, else_process) => {
                let mut vars: IndexSet<LocalName> =
                    processes.iter().flat_map(|p| p.free_variables()).collect();
                if let Some(p) = else_process {
                    vars.extend(p.free_variables());
                }
                vars
            }
            TerminalCommand::Break => IndexSet::new(),
            TerminalCommand::Begin { captures, body, .. } => {
                let mut vars: IndexSet<LocalName> = captures.names.keys().cloned().collect();
                vars.extend(body.free_variables());
                vars
            }
            TerminalCommand::Loop(_, _, captures) => captures.names.keys().cloned().collect(),
        }
    }
}

impl<S: Clone + Eq + std::hash::Hash + std::fmt::Display> Command<Type<S>, S> {
    pub fn types_at_spans(
        &self,
        program: &CheckedModule<S>,
        docs: &Docs<S>,
        consume: &mut impl FnMut(Span, HoverInfo<S>),
    ) {
        match self {
            Self::Noop => {}
            Self::Send(argument) => {
                argument.types_at_spans(program, docs, consume);
            }
            Self::Receive(param, annotation, param_type, _) => {
                consume(param.span(), HoverInfo::named(param, param_type.clone()));
                if let Some(annotation) = annotation {
                    annotation.types_at_spans(&program.type_defs, docs, consume);
                }
            }
            Self::Signal(_) | Self::Continue | Self::ReceiveType(_) => {}
            Self::SendType(typ) => {
                typ.types_at_spans(&program.type_defs, docs, consume);
            }
        }
    }
}

impl<S: Clone + Eq + std::hash::Hash + std::fmt::Display> TerminalCommand<Type<S>, S> {
    pub fn types_at_spans(
        &self,
        program: &CheckedModule<S>,
        docs: &Docs<S>,
        consume: &mut impl FnMut(Span, HoverInfo<S>),
    ) {
        match self {
            Self::Link(expression) => expression.types_at_spans(program, docs, consume),
            Self::Case(_, branches, else_process) => {
                for process in branches {
                    process.types_at_spans(program, docs, consume);
                }
                if let Some(process) = else_process {
                    process.types_at_spans(program, docs, consume);
                }
            }
            Self::Break | Self::Loop(..) => {}
            Self::Begin { body, .. } => body.types_at_spans(program, docs, consume),
        }
    }
}

impl<S: Clone> Expression<(), S> {
    pub fn optimize(&self) -> Arc<Self> {
        match self {
            Self::Global(span, name, typ) => {
                Arc::new(Self::Global(span.clone(), name.clone(), typ.clone()))
            }
            Self::Variable(span, name, typ, usage) => Arc::new(Self::Variable(
                span.clone(),
                name.clone(),
                typ.clone(),
                usage.clone(),
            )),
            Self::Box(span, caps, expression, typ) => Arc::new(Self::Box(
                span.clone(),
                caps.clone(),
                expression.optimize(),
                typ.clone(),
            )),
            Self::Chan {
                span,
                captures,
                chan_name,
                chan_annotation,
                chan_type,
                expr_type,
                process,
            } => Arc::new(Self::Chan {
                span: span.clone(),
                captures: captures.clone(),
                chan_name: chan_name.clone(),
                chan_annotation: chan_annotation.clone(),
                chan_type: chan_type.clone(),
                expr_type: expr_type.clone(),
                process: process.optimize(),
            }),
            Self::Primitive(span, value, typ) => {
                Arc::new(Self::Primitive(span.clone(), value.clone(), typ.clone()))
            }
            Self::External(f, typ) => Arc::new(Self::External(f.clone(), typ.clone())),
        }
    }
}

#[derive(Clone, Debug)]
pub struct HoverInfo<S> {
    inner: HoverInfoInner<S>,
}

#[derive(Clone, Debug)]
pub enum TypeHoverHeader<S> {
    Parameters(Vec<TypeParameter>),
    Arguments(Vec<Type<S>>),
}

#[derive(Clone, Debug)]
enum HoverInfoInner<S> {
    Type {
        name: GlobalName<S>,
        header: TypeHoverHeader<S>,
        typ: Type<S>,
        doc: Option<DocComment>,
        span: Span,
    },
    Declaration {
        name: GlobalName<S>,
        typ: Type<S>,
        doc: Option<DocComment>,
        def_span: Span,
        decl_span: Span,
    },
    Variable {
        name: String,
        typ: Type<S>,
    },
    Anonymous {
        typ: Type<S>,
    },
    Module {
        module: S,
        doc: Option<DocComment>,
        types: Vec<(GlobalName<S>, Vec<TypeParameter>, Type<S>)>,
        declarations: Vec<(GlobalName<S>, Type<S>)>,
    },
}

impl<S> HoverInfo<S> {
    pub fn type_definition(
        name: GlobalName<S>,
        params: Vec<TypeParameter>,
        typ: Type<S>,
        doc: Option<DocComment>,
        span: Span,
    ) -> Self {
        Self {
            inner: HoverInfoInner::Type {
                name,
                header: TypeHoverHeader::Parameters(params),
                typ,
                doc,
                span,
            },
        }
    }

    pub fn type_instantiation(
        name: GlobalName<S>,
        args: Vec<Type<S>>,
        typ: Type<S>,
        doc: Option<DocComment>,
        span: Span,
    ) -> Self {
        Self {
            inner: HoverInfoInner::Type {
                name,
                header: TypeHoverHeader::Arguments(args),
                typ,
                doc,
                span,
            },
        }
    }

    pub fn declaration(
        name: GlobalName<S>,
        typ: Type<S>,
        doc: Option<DocComment>,
        def_span: Span,
        decl_span: Span,
    ) -> Self {
        Self {
            inner: HoverInfoInner::Declaration {
                name,
                typ,
                doc,
                def_span,
                decl_span,
            },
        }
    }

    pub fn named(name: impl ToString, typ: Type<S>) -> Self {
        Self {
            inner: HoverInfoInner::Variable {
                name: name.to_string(),
                typ,
            },
        }
    }

    pub fn unnamed(typ: Type<S>) -> Self {
        Self {
            inner: HoverInfoInner::Anonymous { typ },
        }
    }

    pub fn module(
        module: S,
        doc: Option<DocComment>,
        types: Vec<(GlobalName<S>, Vec<TypeParameter>, Type<S>)>,
        declarations: Vec<(GlobalName<S>, Type<S>)>,
    ) -> Self {
        Self {
            inner: HoverInfoInner::Module {
                module,
                doc,
                types,
                declarations,
            },
        }
    }

    pub fn is_module(&self) -> bool {
        matches!(self.inner, HoverInfoInner::Module { .. })
    }

    pub fn typ(&self) -> Option<&Type<S>> {
        match &self.inner {
            HoverInfoInner::Type { typ, .. }
            | HoverInfoInner::Declaration { typ, .. }
            | HoverInfoInner::Variable { typ, .. }
            | HoverInfoInner::Anonymous { typ, .. } => Some(typ),
            HoverInfoInner::Module { .. } => None,
        }
    }

    pub fn doc(&self) -> Option<&DocComment> {
        match &self.inner {
            HoverInfoInner::Type { doc, .. } | HoverInfoInner::Declaration { doc, .. } => {
                doc.as_ref()
            }
            HoverInfoInner::Variable { .. }
            | HoverInfoInner::Anonymous { .. }
            | HoverInfoInner::Module { doc: None, .. } => None,
            HoverInfoInner::Module { doc: Some(doc), .. } => Some(doc),
        }
    }

    pub fn global_name(&self) -> Option<&GlobalName<S>> {
        match &self.inner {
            HoverInfoInner::Type { name, .. } | HoverInfoInner::Declaration { name, .. } => {
                Some(name)
            }
            HoverInfoInner::Variable { .. }
            | HoverInfoInner::Anonymous { .. }
            | HoverInfoInner::Module { .. } => None,
        }
    }

    pub fn type_header(&self) -> Option<&TypeHoverHeader<S>> {
        match &self.inner {
            HoverInfoInner::Type { header, .. } => Some(header),
            HoverInfoInner::Declaration { .. }
            | HoverInfoInner::Variable { .. }
            | HoverInfoInner::Anonymous { .. }
            | HoverInfoInner::Module { .. } => None,
        }
    }

    pub fn variable_name(&self) -> Option<&str> {
        match &self.inner {
            HoverInfoInner::Variable { name, .. } => Some(name.as_str()),
            HoverInfoInner::Type { .. }
            | HoverInfoInner::Declaration { .. }
            | HoverInfoInner::Anonymous { .. }
            | HoverInfoInner::Module { .. } => None,
        }
    }

    pub fn prefer_display_hints(&self) -> bool {
        match &self.inner {
            HoverInfoInner::Type { .. } | HoverInfoInner::Module { .. } => false,
            HoverInfoInner::Declaration { .. }
            | HoverInfoInner::Variable { .. }
            | HoverInfoInner::Anonymous { .. } => true,
        }
    }

    pub fn is_type(&self) -> bool {
        matches!(self.inner, HoverInfoInner::Type { .. })
    }

    pub fn is_declaration(&self) -> bool {
        matches!(self.inner, HoverInfoInner::Declaration { .. })
    }

    pub fn decl_span(&self) -> Span {
        match &self.inner {
            HoverInfoInner::Type { span, .. } => span.clone(),
            HoverInfoInner::Declaration { decl_span, .. } => decl_span.clone(),
            HoverInfoInner::Variable { .. }
            | HoverInfoInner::Anonymous { .. }
            | HoverInfoInner::Module { .. } => Span::None,
        }
    }

    pub fn def_span(&self) -> Span {
        match &self.inner {
            HoverInfoInner::Type { span, .. } => span.clone(),
            HoverInfoInner::Declaration { def_span, .. } => def_span.clone(),
            HoverInfoInner::Variable { .. }
            | HoverInfoInner::Anonymous { .. }
            | HoverInfoInner::Module { .. } => Span::None,
        }
    }

    pub fn module_items(
        &self,
    ) -> Option<(
        &S,
        &[(GlobalName<S>, Vec<TypeParameter>, Type<S>)],
        &[(GlobalName<S>, Type<S>)],
    )> {
        match &self.inner {
            HoverInfoInner::Module {
                module,
                types,
                declarations,
                ..
            } => Some((module, types, declarations)),
            _ => None,
        }
    }
}

impl<S: Clone + Eq + std::hash::Hash + std::fmt::Display> Expression<Type<S>, S> {
    pub fn types_at_spans(
        &self,
        program: &CheckedModule<S>,
        docs: &Docs<S>,
        consume: &mut impl FnMut(Span, HoverInfo<S>),
    ) {
        match self {
            Self::Global(_, name, typ) => {
                let def_span = (program.definitions.get(name))
                    .map(|(def, _typ)| def.span.clone())
                    .unwrap_or_default();
                let decl_span = (program.declarations.get(name))
                    .map(|decl| decl.span.clone())
                    .unwrap_or_else(|| def_span.clone());
                consume(
                    name.span(),
                    HoverInfo::declaration(
                        name.clone(),
                        typ.clone(),
                        docs.declaration_doc(name).cloned(),
                        def_span,
                        decl_span,
                    ),
                );
            }
            Self::Variable(_, name, typ, _usage) => {
                consume(name.span(), HoverInfo::named(name, typ.clone()));
            }
            Self::Box(span, _, expression, typ) => {
                consume(span.clone(), HoverInfo::unnamed(typ.clone()));
                expression.types_at_spans(program, docs, consume);
            }
            Self::Chan {
                chan_name,
                chan_annotation,
                chan_type,
                process,
                ..
            } => {
                consume(
                    chan_name.span(),
                    HoverInfo::named(chan_name, chan_type.clone()),
                );
                if let Some(chan_annotation) = chan_annotation {
                    chan_annotation.types_at_spans(&program.type_defs, docs, consume);
                }
                process.types_at_spans(program, docs, consume);
            }
            Self::Primitive(_, _, _) => {}
            Self::External(_, _) => {}
        }
    }
}

impl<Typ: Clone, S> Expression<Typ, S> {
    pub fn get_type(&self) -> Typ {
        match self {
            Self::Global(_, _, typ) => typ.clone(),
            Self::Variable(_, _, typ, _usage) => typ.clone(),
            Self::Box(_, _, _, typ) => typ.clone(),
            Self::Chan { expr_type, .. } => expr_type.clone(),
            Self::Primitive(_, _, typ) => typ.clone(),
            Self::External(_, typ) => typ.clone(),
        }
    }
}

impl<S: Clone> Process<Type<S>, S> {
    pub fn map_types(&self, f: &mut impl FnMut(Type<S>) -> Type<S>) -> Arc<Self> {
        let steps = self
            .steps
            .iter()
            .map(|step| match step {
                Step::Let {
                    span,
                    name,
                    annotation,
                    typ,
                    value,
                } => Step::Let {
                    span: span.clone(),
                    name: name.clone(),
                    annotation: annotation.clone().map(&mut *f),
                    typ: f(typ.clone()),
                    value: value.map_types(f),
                },
                Step::Do {
                    span,
                    name,
                    usage,
                    typ,
                    command,
                } => Step::Do {
                    span: span.clone(),
                    name: name.clone(),
                    usage: usage.clone(),
                    typ: f(typ.clone()),
                    command: command.map_types(f),
                },
            })
            .collect();
        let terminator = match &self.terminator {
            Terminator::Do {
                span,
                name,
                usage,
                typ,
                command,
            } => Terminator::Do {
                span: span.clone(),
                name: name.clone(),
                usage: usage.clone(),
                typ: f(typ.clone()),
                command: command.map_types(f),
            },
            Terminator::Poll {
                span,
                kind,
                driver,
                point,
                clients,
                name,
                name_typ,
                captures,
                then,
                else_,
            } => Terminator::Poll {
                span: span.clone(),
                kind: kind.clone(),
                driver: driver.clone(),
                point: point.clone(),
                clients: map_expression_types_vec(clients, f),
                name: name.clone(),
                name_typ: f(name_typ.clone()),
                captures: captures.clone(),
                then: then.map_types(f),
                else_: else_.map_types(f),
            },
            Terminator::Submit {
                span,
                driver,
                point,
                values,
                captures,
            } => Terminator::Submit {
                span: span.clone(),
                driver: driver.clone(),
                point: point.clone(),
                values: map_expression_types_vec(values, f),
                captures: captures.clone(),
            },
            Terminator::Block(span, index, body, then) => {
                Terminator::Block(span.clone(), *index, body.map_types(f), then.map_types(f))
            }
            Terminator::Goto(span, index, captures) => {
                Terminator::Goto(span.clone(), *index, captures.clone())
            }
            Terminator::Unreachable(span) => Terminator::Unreachable(span.clone()),
            Terminator::ToDo(span) => Terminator::ToDo(span.clone()),
        };
        Arc::new(Process::new(steps, terminator))
    }
}

impl<S: Clone> Command<Type<S>, S> {
    pub fn map_types(&self, f: &mut impl FnMut(Type<S>) -> Type<S>) -> Self {
        match self {
            Command::Noop => Command::Noop,
            Command::Send(argument) => Command::Send(argument.map_types(f)),
            Command::Receive(parameter, annotation, typ, vars) => Command::Receive(
                parameter.clone(),
                annotation.clone().map(&mut *f),
                f(typ.clone()),
                vars.clone(),
            ),
            Command::Signal(chosen) => Command::Signal(chosen.clone()),
            Command::Continue => Command::Continue,
            Command::SendType(argument) => Command::SendType(f(argument.clone())),
            Command::ReceiveType(parameter) => Command::ReceiveType(parameter.clone()),
        }
    }
}

impl<S: Clone> TerminalCommand<Type<S>, S> {
    pub fn map_types(&self, f: &mut impl FnMut(Type<S>) -> Type<S>) -> Self {
        match self {
            TerminalCommand::Link(expression) => TerminalCommand::Link(expression.map_types(f)),
            TerminalCommand::Case(branches, processes, else_process) => TerminalCommand::Case(
                Arc::clone(branches),
                map_process_types_boxed_slice(processes, f),
                else_process.as_ref().map(|process| process.map_types(f)),
            ),
            TerminalCommand::Break => TerminalCommand::Break,
            TerminalCommand::Begin {
                unfounded,
                label,
                captures,
                body,
            } => TerminalCommand::Begin {
                unfounded: *unfounded,
                label: label.clone(),
                captures: captures.clone(),
                body: body.map_types(f),
            },
            TerminalCommand::Loop(label, driver, captures) => {
                TerminalCommand::Loop(label.clone(), driver.clone(), captures.clone())
            }
        }
    }
}

impl<S: Clone> Expression<Type<S>, S> {
    pub fn map_types(&self, f: &mut impl FnMut(Type<S>) -> Type<S>) -> Arc<Self> {
        match self {
            Expression::Global(span, name, typ) => Arc::new(Expression::Global(
                span.clone(),
                name.clone(),
                f(typ.clone()),
            )),
            Expression::Variable(span, name, typ, usage) => Arc::new(Expression::Variable(
                span.clone(),
                name.clone(),
                f(typ.clone()),
                usage.clone(),
            )),
            Expression::Box(span, captures, expression, typ) => Arc::new(Expression::Box(
                span.clone(),
                captures.clone(),
                expression.map_types(f),
                f(typ.clone()),
            )),
            Expression::Chan {
                span,
                captures,
                chan_name,
                chan_annotation,
                chan_type,
                expr_type,
                process,
            } => Arc::new(Expression::Chan {
                span: span.clone(),
                captures: captures.clone(),
                chan_name: chan_name.clone(),
                chan_annotation: chan_annotation.clone().map(&mut *f),
                chan_type: f(chan_type.clone()),
                expr_type: f(expr_type.clone()),
                process: process.map_types(f),
            }),
            Expression::Primitive(span, primitive, typ) => Arc::new(Expression::Primitive(
                span.clone(),
                primitive.clone(),
                f(typ.clone()),
            )),
            Expression::External(external, typ) => {
                Arc::new(Expression::External(external.clone(), f(typ.clone())))
            }
        }
    }
}

fn map_process_types_boxed_slice<S: Clone>(
    processes: &[Arc<Process<Type<S>, S>>],
    f: &mut impl FnMut(Type<S>) -> Type<S>,
) -> Box<[Arc<Process<Type<S>, S>>]> {
    processes
        .iter()
        .map(|process| process.map_types(f))
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

fn map_expression_types_vec<S: Clone>(
    expressions: &[Arc<Expression<Type<S>, S>>],
    f: &mut impl FnMut(Type<S>) -> Type<S>,
) -> Vec<Arc<Expression<Type<S>, S>>> {
    expressions
        .iter()
        .map(|expression| expression.map_types(f))
        .collect()
}

impl<S: Clone> Process<(), S> {
    pub fn map_global_names<T, E>(
        self,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Process<(), T>, E> {
        let mut steps = Vec::with_capacity(self.steps.len());
        for step in self.steps {
            steps.push(match step {
                Step::Let {
                    span,
                    name,
                    annotation,
                    typ: (),
                    value,
                } => Step::Let {
                    span,
                    name,
                    annotation: annotation.map(|typ| typ.map_global_names(f)).transpose()?,
                    typ: (),
                    value: map_arc_expression(value, f)?,
                },
                Step::Do {
                    span,
                    name,
                    usage,
                    typ: (),
                    command,
                } => Step::Do {
                    span,
                    name,
                    usage,
                    typ: (),
                    command: command.map_global_names(f)?,
                },
            });
        }

        let terminator = match self.terminator {
            Terminator::Do {
                span,
                name,
                usage,
                typ: (),
                command,
            } => Terminator::Do {
                span,
                name,
                usage,
                typ: (),
                command: command.map_global_names(f)?,
            },
            Terminator::Poll {
                span,
                kind,
                driver,
                point,
                clients,
                name,
                name_typ: (),
                captures,
                then,
                else_,
            } => Terminator::Poll {
                span,
                kind,
                driver,
                point,
                clients: map_expression_vec(clients, f)?,
                name,
                name_typ: (),
                captures,
                then: map_arc_process(then, f)?,
                else_: map_arc_process(else_, f)?,
            },
            Terminator::Submit {
                span,
                driver,
                point,
                values,
                captures,
            } => Terminator::Submit {
                span,
                driver,
                point,
                values: map_expression_vec(values, f)?,
                captures,
            },
            Terminator::Block(span, index, body, then) => Terminator::Block(
                span,
                index,
                map_arc_process(body, f)?,
                map_arc_process(then, f)?,
            ),
            Terminator::Goto(span, index, captures) => Terminator::Goto(span, index, captures),
            Terminator::Unreachable(span) => Terminator::Unreachable(span),
            Terminator::ToDo(span) => Terminator::ToDo(span),
        };
        Ok(Process::new(steps, terminator))
    }
}

impl<S: Clone> Command<(), S> {
    pub fn map_global_names<T, E>(
        self,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Command<(), T>, E> {
        match self {
            Command::Noop => Ok(Command::Noop),
            Command::Send(argument) => Ok(Command::Send(map_arc_expression(argument, f)?)),
            Command::Receive(parameter, annotation, (), vars) => Ok(Command::Receive(
                parameter,
                annotation.map(|typ| typ.map_global_names(f)).transpose()?,
                (),
                vars,
            )),
            Command::Signal(chosen) => Ok(Command::Signal(chosen)),
            Command::Continue => Ok(Command::Continue),
            Command::SendType(argument) => Ok(Command::SendType(argument.map_global_names(f)?)),
            Command::ReceiveType(parameter) => Ok(Command::ReceiveType(parameter)),
        }
    }
}

impl<S: Clone> TerminalCommand<(), S> {
    pub fn map_global_names<T, E>(
        self,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<TerminalCommand<(), T>, E> {
        match self {
            TerminalCommand::Link(expression) => {
                Ok(TerminalCommand::Link(map_arc_expression(expression, f)?))
            }
            TerminalCommand::Case(branches, processes, else_process) => Ok(TerminalCommand::Case(
                branches,
                map_process_boxed_slice(processes, f)?,
                else_process
                    .map(|process| map_arc_process(process, f))
                    .transpose()?,
            )),
            TerminalCommand::Break => Ok(TerminalCommand::Break),
            TerminalCommand::Begin {
                unfounded,
                label,
                captures,
                body,
            } => Ok(TerminalCommand::Begin {
                unfounded,
                label,
                captures,
                body: map_arc_process(body, f)?,
            }),
            TerminalCommand::Loop(label, driver, captures) => {
                Ok(TerminalCommand::Loop(label, driver, captures))
            }
        }
    }
}

impl<S: Clone> Expression<(), S> {
    pub fn map_global_names<T, E>(
        self,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Expression<(), T>, E> {
        match self {
            Expression::Global(span, name, ()) => Ok(Expression::Global(span, f(name)?, ())),
            Expression::Variable(span, name, (), usage) => {
                Ok(Expression::Variable(span, name, (), usage))
            }
            Expression::Box(span, captures, expression, ()) => {
                Self::map_global_names_box(span, captures, expression, f)
            }
            Expression::Chan {
                span,
                captures,
                chan_name,
                chan_annotation,
                chan_type: (),
                expr_type: (),
                process,
            } => {
                Self::map_global_names_chan(span, captures, chan_name, chan_annotation, process, f)
            }
            Expression::Primitive(span, primitive, ()) => {
                Ok(Expression::Primitive(span, primitive, ()))
            }
            Expression::External(external, ()) => Ok(Expression::External(external, ())),
        }
    }

    fn map_global_names_box<T, E>(
        span: Span,
        captures: Captures,
        expression: Arc<Expression<(), S>>,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Expression<(), T>, E> {
        Ok(Expression::Box(
            span,
            captures,
            map_arc_expression(expression, f)?,
            (),
        ))
    }

    fn map_global_names_chan<T, E>(
        span: Span,
        captures: Captures,
        chan_name: LocalName,
        chan_annotation: Option<Type<S>>,
        process: Arc<Process<(), S>>,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Expression<(), T>, E> {
        Ok(Expression::Chan {
            span,
            captures,
            chan_name,
            chan_annotation: chan_annotation
                .map(|typ| typ.map_global_names(f))
                .transpose()?,
            chan_type: (),
            expr_type: (),
            process: map_arc_process(process, f)?,
        })
    }
}

fn map_arc_process<S: Clone, T, E>(
    process: Arc<Process<(), S>>,
    f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
) -> Result<Arc<Process<(), T>>, E> {
    Ok(Arc::new(Arc::unwrap_or_clone(process).map_global_names(f)?))
}

fn map_arc_expression<S: Clone, T, E>(
    expression: Arc<Expression<(), S>>,
    f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
) -> Result<Arc<Expression<(), T>>, E> {
    Ok(Arc::new(
        Arc::unwrap_or_clone(expression).map_global_names(f)?,
    ))
}

fn map_process_vec<S: Clone, T, E>(
    processes: Vec<Arc<Process<(), S>>>,
    f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
) -> Result<Vec<Arc<Process<(), T>>>, E> {
    let mut mapped = Vec::with_capacity(processes.len());
    for process in processes {
        mapped.push(map_arc_process(process, f)?);
    }
    Ok(mapped)
}

fn map_process_boxed_slice<S: Clone, T, E>(
    processes: Box<[Arc<Process<(), S>>]>,
    f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
) -> Result<Box<[Arc<Process<(), T>>]>, E> {
    Ok(map_process_vec(processes.into_vec(), f)?.into_boxed_slice())
}

fn map_expression_vec<S: Clone, T, E>(
    expressions: Vec<Arc<Expression<(), S>>>,
    f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
) -> Result<Vec<Arc<Expression<(), T>>>, E> {
    let mut mapped = Vec::with_capacity(expressions.len());
    for expression in expressions {
        mapped.push(map_arc_expression(expression, f)?);
    }
    Ok(mapped)
}

impl<Typ, S> Process<Typ, S> {
    pub fn free_variables(&self) -> IndexSet<LocalName> {
        let mut vars = match &self.terminator {
            Terminator::Do { name, command, .. } => {
                let mut vars = command.free_variables();
                vars.insert(name.clone());
                vars
            }
            Terminator::Poll {
                driver,
                clients,
                name,
                then,
                else_,
                ..
            } => {
                let mut vars = IndexSet::new();
                for client in clients {
                    vars.extend(client.free_variables());
                }
                let mut then_vars = then.free_variables();
                then_vars.shift_remove(name);
                then_vars.shift_remove(driver);
                vars.extend(then_vars);
                let mut else_vars = else_.free_variables();
                else_vars.shift_remove(driver);
                vars.extend(else_vars);
                vars
            }
            Terminator::Submit {
                driver,
                values,
                captures,
                ..
            } => {
                let mut vars: IndexSet<LocalName> = captures.names.keys().cloned().collect();
                vars.insert(driver.clone());
                for value in values {
                    vars.extend(value.free_variables());
                }
                vars
            }
            Terminator::Block(_, _, _body, process) => process.free_variables(),
            Terminator::Goto(_, _, caps) => caps.names.keys().cloned().collect(),
            Terminator::Unreachable(_) | Terminator::ToDo(_) => IndexSet::new(),
        };

        for step in self.steps.iter().rev() {
            match step {
                Step::Let { name, value, .. } => {
                    vars.shift_remove(name);
                    vars.extend(value.free_variables());
                }
                Step::Do { name, command, .. } => {
                    if let Command::Receive(parameter, ..) = command {
                        vars.shift_remove(parameter);
                    }
                    vars.extend(command.free_variables());
                    vars.insert(name.clone());
                }
            }
        }
        vars
    }
}

impl<Typ, S: Clone + std::fmt::Display> Process<Typ, S> {
    pub fn pretty(&self, f: &mut impl Write, indent: usize) -> fmt::Result {
        for step in &self.steps {
            indentation(f, indent)?;
            match step {
                Step::Let { name, value, .. } => {
                    write!(f, "let {} = ", name)?;
                    value.pretty(f, indent)?;
                }
                Step::Do { name, command, .. } => {
                    write!(f, "{}", name)?;
                    command.pretty(f, indent)?;
                }
            }
        }

        indentation(f, indent)?;
        match &self.terminator {
            Terminator::Unreachable(_) => {
                write!(f, "unreachable")
            }

            Terminator::ToDo(_) => {
                write!(f, "todo")
            }

            Terminator::Poll {
                kind,
                driver,
                point,
                clients,
                name,
                then,
                else_,
                ..
            } => {
                match kind {
                    PollKind::Poll => write!(f, "poll")?,
                    PollKind::Repoll => write!(f, "repoll")?,
                }
                write!(f, "[{} -> {}](", driver, point)?;
                if let Some(first) = clients.first() {
                    first.pretty(f, indent)?;
                    for client in &clients[1..] {
                        write!(f, ", ")?;
                        client.pretty(f, indent)?;
                    }
                }
                write!(f, ") {{")?;
                indentation(f, indent + 1)?;
                write!(f, "{} => {{", name)?;
                then.pretty(f, indent + 2)?;
                indentation(f, indent + 1)?;
                write!(f, "}}")?;
                indentation(f, indent + 1)?;
                write!(f, "else => {{")?;
                else_.pretty(f, indent + 2)?;
                indentation(f, indent + 1)?;
                write!(f, "}}")?;
                indentation(f, indent)?;
                write!(f, "}}")?;
                Ok(())
            }

            Terminator::Submit {
                driver,
                point,
                values,
                ..
            } => {
                write!(f, "submit[{} -> {}](", driver, point)?;
                if let Some(first) = values.first() {
                    first.pretty(f, indent)?;
                    for value in &values[1..] {
                        write!(f, ", ")?;
                        value.pretty(f, indent)?;
                    }
                }
                write!(f, ")")?;
                Ok(())
            }

            Terminator::Do { name, command, .. } => {
                write!(f, "{}", name)?;
                match command {
                    TerminalCommand::Link(expression) => {
                        write!(f, " <> ")?;
                        expression.pretty(f, indent)
                    }
                    TerminalCommand::Case(choices, branches, else_process) => {
                        write!(f, ".case {{")?;
                        for (choice, process) in choices.iter().zip(branches.iter()) {
                            indentation(f, indent + 1)?;
                            write!(f, ".{} => {{", choice)?;
                            process.pretty(f, indent + 2)?;
                            indentation(f, indent + 1)?;
                            write!(f, "}}")?;
                        }
                        if let Some(process) = else_process {
                            indentation(f, indent + 1)?;
                            write!(f, "else => {{")?;
                            process.pretty(f, indent + 2)?;
                            indentation(f, indent + 1)?;
                            write!(f, "}}")?;
                        }
                        indentation(f, indent)?;
                        write!(f, "}}")
                    }
                    TerminalCommand::Break => write!(f, "!"),
                    TerminalCommand::Begin {
                        unfounded,
                        label,
                        body: process,
                        ..
                    } => {
                        if *unfounded {
                            write!(f, ".unfounded")?;
                        } else {
                            write!(f, ".begin")?;
                        }
                        if let Some(label) = label {
                            write!(f, "@{}", label)?;
                        }
                        process.pretty(f, indent)
                    }
                    TerminalCommand::Loop(label, driver, caps) => {
                        write!(f, ".loop")?;
                        if let Some(label) = label {
                            write!(f, "@{} ", label)?;
                        }
                        write!(f, "{{{} |", driver)?;
                        for var in caps.names.keys() {
                            write!(f, " {}", var)?;
                        }
                        write!(f, "}}")?;
                        Ok(())
                    }
                }
            }

            Terminator::Block(_, index, body, process) => {
                write!(f, "block@{} {{", index)?;
                body.pretty(f, indent + 1)?;
                indentation(f, indent)?;
                write!(f, "}}")?;
                process.pretty(f, indent)
            }

            Terminator::Goto(_, index, _) => {
                write!(f, "goto@{}", index)
            }
        }
    }
}

impl<Typ, S: Clone + std::fmt::Display> Command<Typ, S> {
    fn pretty(&self, f: &mut impl Write, indent: usize) -> fmt::Result {
        match self {
            Command::Noop => Ok(()),
            Command::Send(argument) => {
                write!(f, "(")?;
                argument.pretty(f, indent)?;
                write!(f, ")")
            }
            Command::Receive(parameter, _, _, vars) => {
                write!(f, "[")?;
                if !vars.is_empty() {
                    write!(f, "<{}", vars[0])?;
                    for var in &vars[1..] {
                        write!(f, ", {}", var)?;
                    }
                    write!(f, "> ")?;
                }
                write!(f, "{}]", parameter)
            }
            Command::Signal(chosen) => write!(f, ".{}", chosen),
            Command::Continue => write!(f, "?"),
            Command::SendType(argument) => {
                write!(f, "(type ")?;
                argument.pretty(f, &CanonicalGlobalNameWriter, indent)?;
                write!(f, ")")
            }
            Command::ReceiveType(parameter) => write!(f, "[type {}]", parameter),
        }
    }
}

impl<Typ, S> Expression<Typ, S> {
    pub fn free_variables(&self) -> IndexSet<LocalName> {
        match self {
            Expression::Global(_, _, _) => IndexSet::new(),
            Expression::Variable(_, name, _, _) => {
                let mut set = IndexSet::new();
                set.insert(name.clone());
                set
            }
            Expression::Box(_, _, expression, _) => expression.free_variables(),
            Expression::Chan { captures, .. } => captures.names.keys().cloned().collect(),
            Expression::Primitive(_, _, _) => IndexSet::new(),
            Expression::External(_, _) => IndexSet::new(),
        }
    }
}

impl<Typ, S: Clone + std::fmt::Display> Expression<Typ, S> {
    pub fn pretty(&self, f: &mut impl Write, indent: usize) -> fmt::Result {
        match self {
            Self::Global(_, name, _) => write!(f, "{name}"),

            Self::Variable(_, name, _, _) => {
                write!(f, "{}", name)
            }

            Self::Box(_, _, expression, _) => {
                write!(f, "box ")?;
                expression.pretty(f, indent)
            }

            Self::Chan {
                chan_name: channel,
                process,
                ..
            } => {
                write!(f, "chan {} {{", channel)?;
                process.pretty(f, indent + 1)?;
                indentation(f, indent)?;
                write!(f, "}}")
            }

            Self::Primitive(_, value, _) => value.pretty(f, indent),

            Self::External(_, _) => {
                write!(f, "<external>")
            }
        }
    }
}

fn indentation(f: &mut impl Write, indent: usize) -> fmt::Result {
    write!(f, "\n")?;
    for _ in 0..indent {
        write!(f, "  ")?;
    }
    Ok(())
}

struct CanonicalGlobalNameWriter;

impl<S: std::fmt::Display> GlobalNameWriter<S> for CanonicalGlobalNameWriter {
    fn write_global_name<W: Write>(&self, f: &mut W, name: &GlobalName<S>) -> fmt::Result {
        write!(f, "{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend_impl::language::Unresolved;
    use arcstr::literal;

    #[test]
    fn implicit_receive_parameters_render_inside_brackets() {
        let command = Command::<(), Unresolved>::Receive(
            LocalName::from(literal!("value")),
            None,
            (),
            vec![TypeParameter::any(LocalName::from(literal!("a")))],
        );
        let mut rendered = String::new();
        command.pretty(&mut rendered, 0).unwrap();
        assert_eq!(rendered, "[<a> value]");
    }
}
