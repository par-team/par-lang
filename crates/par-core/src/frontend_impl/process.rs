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
pub enum Process<Typ, S> {
    Let {
        span: Span,
        name: LocalName,
        annotation: Option<Type<S>>,
        typ: Typ,
        value: Arc<Expression<Typ, S>>,
        then: Arc<Self>,
    },
    Do {
        span: Span,
        name: LocalName,
        usage: VariableUsage,
        typ: Typ,
        command: Command<Typ, S>,
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
        then: Arc<Self>,
        else_: Arc<Self>,
    },
    Submit {
        span: Span,
        driver: LocalName,
        point: LocalName,
        values: Vec<Arc<Expression<Typ, S>>>,
        captures: Captures,
    },
    Block(Span, usize, Arc<Self>, Arc<Self>),
    Goto(Span, usize, Captures),
    Unreachable(Span),
}

#[derive(Clone, Debug)]
pub enum Command<Typ, S> {
    Noop(Arc<Process<Typ, S>>),
    Link(Arc<Expression<Typ, S>>),
    Send(Arc<Expression<Typ, S>>, Arc<Process<Typ, S>>),
    Receive(
        LocalName,
        Option<Type<S>>,
        Typ,
        Arc<Process<Typ, S>>,
        Vec<TypeParameter>,
    ),
    Signal(LocalName, Arc<Process<Typ, S>>),
    Case(
        Arc<[LocalName]>,
        Box<[Arc<Process<Typ, S>>]>,
        Option<Arc<Process<Typ, S>>>,
    ),
    Break,
    Continue(Arc<Process<Typ, S>>),
    Begin {
        unfounded: bool,
        label: Option<LocalName>,
        captures: Captures,
        body: Arc<Process<Typ, S>>,
    },
    Loop(Option<LocalName>, LocalName, Captures),
    SendType(Type<S>, Arc<Process<Typ, S>>),
    ReceiveType(TypeParameter, Arc<Process<Typ, S>>),
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
        match self {
            Self::Let { span, .. } => span.clone(),
            Self::Do { span, .. } => span.clone(),
            Self::Poll { span, .. } => span.clone(),
            Self::Submit { span, .. } => span.clone(),
            Self::Block(span, _, _, _) => span.clone(),
            Self::Goto(span, _, _) => span.clone(),
            Self::Unreachable(span) => span.clone(),
        }
    }
}

impl<S: Clone> Process<(), S> {
    pub fn optimize(&self) -> Arc<Self> {
        match self {
            Self::Let {
                span,
                name,
                annotation,
                typ,
                value: expression,
                then: process,
            } => Arc::new(Self::Let {
                span: span.clone(),
                name: name.clone(),
                annotation: annotation.clone(),
                typ: typ.clone(),
                value: expression.optimize(),
                then: process.optimize(),
            }),
            Self::Do {
                span,
                name,
                usage,
                typ,
                command,
            } => Arc::new(Self::Do {
                span: span.clone(),
                name: name.clone(),
                typ: typ.clone(),
                usage: usage.clone(),
                command: match command {
                    Command::Noop(process) => Command::Noop(process.optimize()),
                    Command::Link(expression) => {
                        let expression = expression.optimize();
                        match expression.optimize().as_ref() {
                            Expression::Chan {
                                chan_name: channel,
                                chan_annotation: annotation,
                                process,
                                ..
                            } => {
                                if name == channel && annotation.is_none() {
                                    return Arc::clone(process);
                                } else {
                                    return Arc::new(Process::Let {
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
                                        then: Arc::clone(process),
                                    });
                                }
                            }
                            _ => Command::Link(expression),
                        }
                    }
                    Command::Send(argument, process) => {
                        Command::Send(argument.optimize(), process.optimize())
                    }
                    Command::Receive(parameter, annotation, typ, process, vars) => {
                        Command::Receive(
                            parameter.clone(),
                            annotation.clone(),
                            typ.clone(),
                            process.optimize(),
                            vars.clone(),
                        )
                    }
                    Command::Signal(chosen, process) => {
                        Command::Signal(chosen.clone(), process.optimize())
                    }
                    Command::Case(branches, processes, else_process) => {
                        let processes = processes.iter().map(|p| p.optimize()).collect();
                        let else_process = else_process.clone().map(|p| p.optimize());
                        Command::Case(Arc::clone(branches), processes, else_process)
                    }
                    Command::Break => Command::Break,
                    Command::Continue(process) => Command::Continue(process.optimize()),
                    Command::Begin {
                        unfounded,
                        label,
                        captures,
                        body: process,
                    } => Command::Begin {
                        unfounded: unfounded.clone(),
                        label: label.clone(),
                        captures: captures.clone(),
                        body: process.optimize(),
                    },
                    Command::Loop(label, driver, captures) => {
                        Command::Loop(label.clone(), driver.clone(), captures.clone())
                    }
                    Command::SendType(argument, process) => {
                        Command::SendType(argument.clone(), process.optimize())
                    }
                    Command::ReceiveType(parameter, process) => {
                        Command::ReceiveType(parameter.clone(), process.optimize())
                    }
                },
            }),
            Self::Poll {
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
            } => Arc::new(Self::Poll {
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
            }),
            Self::Submit {
                span,
                driver,
                point,
                values,
                captures,
            } => Arc::new(Self::Submit {
                span: span.clone(),
                driver: driver.clone(),
                point: point.clone(),
                values: values.iter().map(|e| e.optimize()).collect(),
                captures: captures.clone(),
            }),
            Self::Block(span, index, body, process) => Arc::new(Self::Block(
                span.clone(),
                *index,
                body.optimize(),
                process.optimize(),
            )),
            Self::Goto(span, index, caps) => {
                Arc::new(Self::Goto(span.clone(), *index, caps.clone()))
            }
            Self::Unreachable(span) => Arc::new(Self::Unreachable(span.clone())),
        }
    }
}

impl<S: Clone + Eq + std::hash::Hash + std::fmt::Display> Process<Type<S>, S> {
    pub fn types_at_spans(
        &self,
        program: &CheckedModule<S>,
        docs: &Docs<S>,
        consume: &mut impl FnMut(Span, HoverInfo<S>),
    ) {
        match self {
            Process::Let {
                name,
                annotation,
                typ,
                value,
                then,
                ..
            } => {
                value.types_at_spans(program, docs, consume);
                consume(name.span(), HoverInfo::named(name, typ.clone()));
                if let Some(annotation) = annotation {
                    annotation.types_at_spans(&program.type_defs, docs, consume);
                }
                then.types_at_spans(program, docs, consume);
            }
            Process::Do {
                span,
                name,
                typ,
                command,
                ..
            } => {
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
                command.types_at_spans(program, docs, consume);
            }
            Process::Poll {
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
            Process::Submit { values, .. } => {
                for value in values {
                    value.types_at_spans(program, docs, consume);
                }
            }
            Process::Block(_, _, body, process) => {
                body.types_at_spans(program, docs, consume);
                process.types_at_spans(program, docs, consume);
            }
            Process::Goto(_, _, _) => {}
            Process::Unreachable(_) => {}
        }
    }
}

impl<Typ, S> Command<Typ, S> {
    pub fn free_variables(&self) -> IndexSet<LocalName> {
        match self {
            Command::Noop(process) => process.free_variables(),
            Command::Link(expression) => expression.free_variables(),
            Command::Send(argument, process) => {
                let mut vars = argument.free_variables();
                vars.extend(process.free_variables());
                vars
            }
            Command::Receive(parameter, _annot, _typ, process, _vars) => {
                let mut vars = process.free_variables();
                vars.shift_remove(parameter);
                vars
            }
            Command::Signal(_, process) => process.free_variables(),
            Command::Case(_, processes, else_process) => {
                let mut vars: IndexSet<LocalName> =
                    processes.iter().flat_map(|p| p.free_variables()).collect();
                if let Some(p) = else_process {
                    vars.extend(p.free_variables());
                }
                vars
            }
            Command::Break => IndexSet::new(),
            Command::Continue(process) => process.free_variables(),
            Command::Begin { captures, body, .. } => {
                let mut vars: IndexSet<LocalName> = captures.names.keys().cloned().collect();
                vars.extend(body.free_variables());
                vars
            }
            Command::Loop(_, _, captures) => captures.names.keys().cloned().collect(),
            Command::SendType(_, process) => process.free_variables(),
            Command::ReceiveType(_, process) => process.free_variables(),
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
            Self::Noop(process) => {
                process.types_at_spans(program, docs, consume);
            }
            Self::Link(expression) => {
                expression.types_at_spans(program, docs, consume);
            }
            Self::Send(argument, process) => {
                argument.types_at_spans(program, docs, consume);
                process.types_at_spans(program, docs, consume);
            }
            Self::Receive(param, annotation, param_type, process, _) => {
                consume(param.span(), HoverInfo::named(param, param_type.clone()));
                if let Some(annotation) = annotation {
                    annotation.types_at_spans(&program.type_defs, docs, consume);
                }
                process.types_at_spans(program, docs, consume);
            }
            Self::Signal(_, process) => {
                process.types_at_spans(program, docs, consume);
            }
            Self::Case(_, branches, else_process) => {
                for process in branches {
                    process.types_at_spans(program, docs, consume);
                }
                if let Some(process) = else_process {
                    process.types_at_spans(program, docs, consume);
                }
            }
            Self::Break => {}
            Self::Continue(process) => {
                process.types_at_spans(program, docs, consume);
            }
            Self::Begin { body, .. } => {
                body.types_at_spans(program, docs, consume);
            }
            Self::Loop(_, _, _) => {}
            Self::SendType(typ, process) => {
                typ.types_at_spans(&program.type_defs, docs, consume);
                process.types_at_spans(program, docs, consume);
            }
            Self::ReceiveType(_, process) => {
                process.types_at_spans(program, docs, consume);
            }
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
        match self {
            Process::Let {
                span,
                name,
                annotation,
                typ,
                value,
                then,
            } => Arc::new(Process::Let {
                span: span.clone(),
                name: name.clone(),
                annotation: annotation.clone().map(&mut *f),
                typ: f(typ.clone()),
                value: value.map_types(f),
                then: then.map_types(f),
            }),
            Process::Do {
                span,
                name,
                usage,
                typ,
                command,
            } => Arc::new(Process::Do {
                span: span.clone(),
                name: name.clone(),
                usage: usage.clone(),
                typ: f(typ.clone()),
                command: command.map_types(f),
            }),
            Process::Poll {
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
            } => Arc::new(Process::Poll {
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
            }),
            Process::Submit {
                span,
                driver,
                point,
                values,
                captures,
            } => Arc::new(Process::Submit {
                span: span.clone(),
                driver: driver.clone(),
                point: point.clone(),
                values: map_expression_types_vec(values, f),
                captures: captures.clone(),
            }),
            Process::Block(span, index, body, then) => Arc::new(Process::Block(
                span.clone(),
                *index,
                body.map_types(f),
                then.map_types(f),
            )),
            Process::Goto(span, index, captures) => {
                Arc::new(Process::Goto(span.clone(), *index, captures.clone()))
            }
            Process::Unreachable(span) => Arc::new(Process::Unreachable(span.clone())),
        }
    }
}

impl<S: Clone> Command<Type<S>, S> {
    pub fn map_types(&self, f: &mut impl FnMut(Type<S>) -> Type<S>) -> Self {
        match self {
            Command::Noop(process) => Command::Noop(process.map_types(f)),
            Command::Link(expression) => Command::Link(expression.map_types(f)),
            Command::Send(argument, process) => {
                Command::Send(argument.map_types(f), process.map_types(f))
            }
            Command::Receive(parameter, annotation, typ, process, vars) => Command::Receive(
                parameter.clone(),
                annotation.clone().map(&mut *f),
                f(typ.clone()),
                process.map_types(f),
                vars.clone(),
            ),
            Command::Signal(chosen, process) => {
                Command::Signal(chosen.clone(), process.map_types(f))
            }
            Command::Case(branches, processes, else_process) => Command::Case(
                Arc::clone(branches),
                map_process_types_boxed_slice(processes, f),
                else_process.as_ref().map(|process| process.map_types(f)),
            ),
            Command::Break => Command::Break,
            Command::Continue(process) => Command::Continue(process.map_types(f)),
            Command::Begin {
                unfounded,
                label,
                captures,
                body,
            } => Command::Begin {
                unfounded: *unfounded,
                label: label.clone(),
                captures: captures.clone(),
                body: body.map_types(f),
            },
            Command::Loop(label, driver, captures) => {
                Command::Loop(label.clone(), driver.clone(), captures.clone())
            }
            Command::SendType(argument, process) => {
                Command::SendType(f(argument.clone()), process.map_types(f))
            }
            Command::ReceiveType(parameter, process) => {
                Command::ReceiveType(parameter.clone(), process.map_types(f))
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
        match self {
            Process::Let {
                span,
                name,
                annotation,
                typ: (),
                value,
                then,
            } => Self::map_global_names_let(span, name, annotation, value, then, f),
            Process::Do {
                span,
                name,
                usage,
                typ: (),
                command,
            } => Self::map_global_names_do(span, name, usage, command, f),
            Process::Poll {
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
            } => Self::map_global_names_poll(
                span, kind, driver, point, clients, name, captures, then, else_, f,
            ),
            Process::Submit {
                span,
                driver,
                point,
                values,
                captures,
            } => Self::map_global_names_submit(span, driver, point, values, captures, f),
            Process::Block(span, index, body, then) => {
                Self::map_global_names_block(span, index, body, then, f)
            }
            Process::Goto(span, index, captures) => Ok(Process::Goto(span, index, captures)),
            Process::Unreachable(span) => Ok(Process::Unreachable(span)),
        }
    }

    fn map_global_names_let<T, E>(
        span: Span,
        name: LocalName,
        annotation: Option<Type<S>>,
        value: Arc<Expression<(), S>>,
        then: Arc<Process<(), S>>,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Process<(), T>, E> {
        Ok(Process::Let {
            span,
            name,
            annotation: annotation.map(|typ| typ.map_global_names(f)).transpose()?,
            typ: (),
            value: map_arc_expression(value, f)?,
            then: map_arc_process(then, f)?,
        })
    }

    fn map_global_names_do<T, E>(
        span: Span,
        name: LocalName,
        usage: VariableUsage,
        command: Command<(), S>,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Process<(), T>, E> {
        Ok(Process::Do {
            span,
            name,
            usage,
            typ: (),
            command: command.map_global_names(f)?,
        })
    }

    fn map_global_names_poll<T, E>(
        span: Span,
        kind: PollKind,
        driver: LocalName,
        point: LocalName,
        clients: Vec<Arc<Expression<(), S>>>,
        name: LocalName,
        captures: Captures,
        then: Arc<Process<(), S>>,
        else_: Arc<Process<(), S>>,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Process<(), T>, E> {
        Ok(Process::Poll {
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
        })
    }

    fn map_global_names_submit<T, E>(
        span: Span,
        driver: LocalName,
        point: LocalName,
        values: Vec<Arc<Expression<(), S>>>,
        captures: Captures,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Process<(), T>, E> {
        Ok(Process::Submit {
            span,
            driver,
            point,
            values: map_expression_vec(values, f)?,
            captures,
        })
    }

    fn map_global_names_block<T, E>(
        span: Span,
        index: usize,
        body: Arc<Process<(), S>>,
        then: Arc<Process<(), S>>,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Process<(), T>, E> {
        Ok(Process::Block(
            span,
            index,
            map_arc_process(body, f)?,
            map_arc_process(then, f)?,
        ))
    }
}

impl<S: Clone> Command<(), S> {
    pub fn map_global_names<T, E>(
        self,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Command<(), T>, E> {
        match self {
            Command::Noop(process) => Self::map_global_names_noop(process, f),
            Command::Link(expression) => Self::map_global_names_link(expression, f),
            Command::Send(argument, process) => Self::map_global_names_send(argument, process, f),
            Command::Receive(parameter, annotation, (), process, vars) => {
                Self::map_global_names_receive(parameter, annotation, process, vars, f)
            }
            Command::Signal(chosen, process) => Self::map_global_names_signal(chosen, process, f),
            Command::Case(branches, processes, else_process) => {
                Self::map_global_names_case(branches, processes, else_process, f)
            }
            Command::Break => Ok(Command::Break),
            Command::Continue(process) => Self::map_global_names_continue(process, f),
            Command::Begin {
                unfounded,
                label,
                captures,
                body,
            } => Self::map_global_names_begin(unfounded, label, captures, body, f),
            Command::Loop(label, driver, captures) => Ok(Command::Loop(label, driver, captures)),
            Command::SendType(argument, process) => {
                Self::map_global_names_send_type(argument, process, f)
            }
            Command::ReceiveType(parameter, process) => {
                Self::map_global_names_receive_type(parameter, process, f)
            }
        }
    }

    fn map_global_names_noop<T, E>(
        process: Arc<Process<(), S>>,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Command<(), T>, E> {
        Ok(Command::Noop(map_arc_process(process, f)?))
    }

    fn map_global_names_link<T, E>(
        expression: Arc<Expression<(), S>>,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Command<(), T>, E> {
        Ok(Command::Link(map_arc_expression(expression, f)?))
    }

    fn map_global_names_send<T, E>(
        argument: Arc<Expression<(), S>>,
        process: Arc<Process<(), S>>,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Command<(), T>, E> {
        Ok(Command::Send(
            map_arc_expression(argument, f)?,
            map_arc_process(process, f)?,
        ))
    }

    fn map_global_names_receive<T, E>(
        parameter: LocalName,
        annotation: Option<Type<S>>,
        process: Arc<Process<(), S>>,
        vars: Vec<TypeParameter>,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Command<(), T>, E> {
        Ok(Command::Receive(
            parameter,
            annotation.map(|typ| typ.map_global_names(f)).transpose()?,
            (),
            map_arc_process(process, f)?,
            vars,
        ))
    }

    fn map_global_names_signal<T, E>(
        chosen: LocalName,
        process: Arc<Process<(), S>>,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Command<(), T>, E> {
        Ok(Command::Signal(chosen, map_arc_process(process, f)?))
    }

    fn map_global_names_case<T, E>(
        branches: Arc<[LocalName]>,
        processes: Box<[Arc<Process<(), S>>]>,
        else_process: Option<Arc<Process<(), S>>>,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Command<(), T>, E> {
        Ok(Command::Case(
            branches,
            map_process_boxed_slice(processes, f)?,
            else_process
                .map(|process| map_arc_process(process, f))
                .transpose()?,
        ))
    }

    fn map_global_names_continue<T, E>(
        process: Arc<Process<(), S>>,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Command<(), T>, E> {
        Ok(Command::Continue(map_arc_process(process, f)?))
    }

    fn map_global_names_begin<T, E>(
        unfounded: bool,
        label: Option<LocalName>,
        captures: Captures,
        body: Arc<Process<(), S>>,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Command<(), T>, E> {
        Ok(Command::Begin {
            unfounded,
            label,
            captures,
            body: map_arc_process(body, f)?,
        })
    }

    fn map_global_names_send_type<T, E>(
        argument: Type<S>,
        process: Arc<Process<(), S>>,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Command<(), T>, E> {
        Ok(Command::SendType(
            argument.map_global_names(f)?,
            map_arc_process(process, f)?,
        ))
    }

    fn map_global_names_receive_type<T, E>(
        parameter: TypeParameter,
        process: Arc<Process<(), S>>,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Command<(), T>, E> {
        Ok(Command::ReceiveType(
            parameter,
            map_arc_process(process, f)?,
        ))
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
        match self {
            Process::Let {
                name, value, then, ..
            } => {
                let mut vars = then.free_variables();
                vars.shift_remove(name);
                vars.extend(value.free_variables());
                vars
            }
            Process::Do { name, command, .. } => {
                let mut vars = command.free_variables();
                vars.insert(name.clone());
                vars
            }
            Process::Poll {
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
            Process::Submit {
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
            Process::Block(_, _, _body, process) => process.free_variables(),
            Process::Goto(_, _, caps) => caps.names.keys().cloned().collect(),
            Process::Unreachable(_) => IndexSet::new(),
        }
    }
}

impl<Typ, S: Clone + std::fmt::Display> Process<Typ, S> {
    pub fn pretty(&self, f: &mut impl Write, indent: usize) -> fmt::Result {
        match self {
            Self::Let {
                span: _,
                name,
                annotation: _,
                typ: _,
                value: expression,
                then: process,
            } => {
                indentation(f, indent)?;
                write!(f, "let {} = ", name)?;
                expression.pretty(f, indent)?;
                process.pretty(f, indent)
            }

            Self::Unreachable(_) => {
                indentation(f, indent)?;
                write!(f, "unreachable")
            }

            Self::Poll {
                kind,
                driver,
                point,
                clients,
                name,
                then,
                else_,
                ..
            } => {
                indentation(f, indent)?;
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

            Self::Submit {
                driver,
                point,
                values,
                ..
            } => {
                indentation(f, indent)?;
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

            Self::Do {
                span: _,
                name: subject,
                usage: _,
                typ: _,
                command,
            } => {
                indentation(f, indent)?;
                write!(f, "{}", subject)?;

                match command {
                    Command::Noop(process) => process.pretty(f, indent),
                    Command::Link(expression) => {
                        write!(f, " <> ")?;
                        expression.pretty(f, indent)
                    }

                    Command::Send(argument, process) => {
                        write!(f, "(")?;
                        argument.pretty(f, indent)?;
                        write!(f, ")")?;
                        process.pretty(f, indent)
                    }

                    Command::Receive(parameter, _, _, process, vars) => {
                        if !vars.is_empty() {
                            write!(f, "<")?;
                            write!(f, "{}", vars[0])?;
                            for var in &vars[1..] {
                                write!(f, ", {}", var)?;
                            }
                            write!(f, ">")?;
                        }
                        write!(f, "[{}]", parameter)?;
                        process.pretty(f, indent)
                    }

                    Command::Signal(chosen, process) => {
                        write!(f, ".{}", chosen)?;
                        process.pretty(f, indent)
                    }

                    Command::Case(choices, branches, else_process) => {
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

                    Command::Break => {
                        write!(f, "!")
                    }

                    Command::Continue(process) => {
                        write!(f, "?")?;
                        process.pretty(f, indent)
                    }

                    Command::Begin {
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

                    Command::Loop(label, driver, caps) => {
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

                    Command::SendType(argument, process) => {
                        write!(f, "(type ")?;
                        argument.pretty(f, &CanonicalGlobalNameWriter, indent)?;
                        write!(f, ")")?;
                        process.pretty(f, indent)
                    }

                    Command::ReceiveType(parameter, process) => {
                        write!(f, "[type {}]", parameter)?;
                        process.pretty(f, indent)
                    }
                }
            }

            Self::Block(_, index, body, process) => {
                indentation(f, indent)?;
                write!(f, "block@{} {{", index)?;
                body.pretty(f, indent + 1)?;
                indentation(f, indent)?;
                write!(f, "}}")?;
                process.pretty(f, indent)
            }

            Self::Goto(_, index, _) => {
                indentation(f, indent)?;
                write!(f, "goto@{}", index)
            }
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
