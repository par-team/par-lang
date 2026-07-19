use super::super::language::{LocalName, TypeConstraint, TypeParameter};
use super::super::process::{
    Captures, Command, Expression, PollKind, Process, Step, TerminalCommand, Terminator,
    VariableUsage,
};
use super::context::{BlockPathContext, BlockScope, PollPointScope, PollScope};
use super::core::{LoopId, Operation, Type, get_primitive_type};
use super::error::TypeError;
use super::lattice::union_types;
use super::{Context, TypeDefs};
use crate::frontend_impl::types::implicit::{resolve_holes, substitute_holes};
use crate::frontend_impl::types::lattice::intersect_types;
use crate::location::Span;
use im::HashMap;
use indexmap::{IndexMap, IndexSet};
use par_runtime::primitive::Primitive;
use par_runtime::readback::Number;
use std::collections::BTreeMap;
use std::sync::Arc;

enum ProcessAnalyzerMode {
    Check,
    Infer(LocalName),
}

/// A deferred step whose type depends on the type inferred for the process suffix.
///
/// Inference walks the process forward, recording these frames until it can infer the subject's
/// type from the terminator. It then replays the frames in reverse to build the type outward and
/// fill in the typed steps, using this explicit stack instead of recursive calls for each step.
enum InferenceFrame<S> {
    Alias {
        index: usize,
        span: Span,
        name: LocalName,
        annotation: Option<Type<S>>,
        variable_span: Span,
        variable: LocalName,
        usage: VariableUsage,
    },
    Noop {
        index: usize,
        span: Span,
        name: LocalName,
        usage: VariableUsage,
    },
    Send {
        index: usize,
        span: Span,
        name: LocalName,
        usage: VariableUsage,
        argument: Arc<Expression<Type<S>, S>>,
        argument_type: Type<S>,
    },
    Receive {
        index: usize,
        span: Span,
        name: LocalName,
        usage: VariableUsage,
        parameter: LocalName,
        annotation: Option<Type<S>>,
        parameter_type: Type<S>,
        type_parameters: Vec<TypeParameter>,
        failed: bool,
    },
    Signal {
        index: usize,
        span: Span,
        name: LocalName,
        usage: VariableUsage,
        chosen: LocalName,
    },
    ReceiveType {
        index: usize,
        span: Span,
        name: LocalName,
        usage: VariableUsage,
        parameter: TypeParameter,
    },
    SendTypeFailure {
        index: usize,
        span: Span,
        name: LocalName,
        usage: VariableUsage,
        argument: Type<S>,
    },
}
impl<S: Clone + Eq + std::hash::Hash> Context<S> {
    fn analyze_process(
        &mut self,
        process: &Process<(), S>,
        mode: &ProcessAnalyzerMode,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Arc<Process<Type<S>, S>>, Option<Type<S>>) {
        match mode {
            ProcessAnalyzerMode::Check => {
                let process = self.check_process(process, emit);
                (process, None)
            }
            ProcessAnalyzerMode::Infer(inference_subject) => {
                let (process, typ) = self.infer_process(process, &inference_subject, emit);
                (process, Some(typ))
            }
        }
    }

    fn resolve_type_parameter(
        &self,
        parameter: &TypeParameter,
        expected: &TypeParameter,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> TypeParameter {
        if parameter.constraint != expected.constraint {
            emit(TypeError::TypeParameterConstraintMismatch(
                parameter.span(),
                parameter.name.clone(),
                parameter.constraint,
                expected.constraint,
            ));
        }
        TypeParameter {
            name: parameter.name.clone(),
            constraint: expected.constraint,
        }
    }

    fn resolve_type_parameters(
        &self,
        parameters: &[TypeParameter],
        expected: &[TypeParameter],
        emit: &mut impl FnMut(TypeError<S>),
    ) -> Vec<TypeParameter> {
        parameters
            .iter()
            .zip(expected)
            .map(|(parameter, expected)| self.resolve_type_parameter(parameter, expected, emit))
            .collect()
    }

    fn check_type_constraint(
        &self,
        span: &Span,
        parameter: &TypeParameter,
        typ: &Type<S>,
        emit: &mut impl FnMut(TypeError<S>),
    ) {
        match typ.satisfies_constraint(parameter.constraint, &self.type_defs) {
            Ok(true) => {}
            Ok(false) => emit(TypeError::TypeDoesNotSatisfyConstraint(
                span.clone(),
                parameter.name.clone(),
                typ.clone(),
                parameter.constraint,
            )),
            Err(error) => emit(error),
        }
    }

    pub(crate) fn check_process(
        &mut self,
        process: &Process<(), S>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> Arc<Process<Type<S>, S>> {
        let mut steps = Vec::with_capacity(process.steps.len());
        for (index, step) in process.steps.iter().enumerate() {
            match step {
                Step::Let {
                    span,
                    name,
                    annotation,
                    typ: (),
                    value,
                } => {
                    let (value, typ) = match annotation {
                        Some(typ) => {
                            if let Err(error) = self.type_defs.validate_type(typ) {
                                emit(error);
                            }
                            (self.check_expression(None, value, typ, emit), typ.clone())
                        }
                        None => self.infer_expression(None, value, emit),
                    };
                    if let Err(error) = self.put(span, name.clone(), typ.clone()) {
                        emit(error);
                    }
                    steps.push(Step::Let {
                        span: span.clone(),
                        name: name.clone(),
                        annotation: annotation.clone(),
                        typ,
                        value,
                    });
                }
                Step::Do {
                    span,
                    name,
                    usage,
                    typ: (),
                    command,
                } => {
                    let typ = self
                        .get_variable_or_error(span, name)
                        .unwrap_or_else(|error| {
                            emit(error);
                            Type::Fail(span.clone())
                        });
                    if let Type::Hole(_, _, hole) | Type::DualHole(_, _, hole) = &typ {
                        let is_dual = matches!(typ, Type::DualHole(..));
                        let suffix = Process::new(
                            process.steps[index..].to_vec(),
                            process.terminator.clone(),
                        );
                        let (typed_suffix, inferred) = self.infer_process(&suffix, name, emit);
                        if is_dual {
                            hole.add_lower_bound(inferred.dual(Span::None));
                        } else {
                            hole.add_upper_bound(inferred);
                        }
                        let typed_suffix = Arc::unwrap_or_clone(typed_suffix);
                        steps.extend(typed_suffix.steps);
                        return Arc::new(Process::new(steps, typed_suffix.terminator));
                    }
                    let command = self.check_step_command(span, name, &typ, command, emit);
                    steps.push(Step::Do {
                        span: span.clone(),
                        name: name.clone(),
                        usage: usage.clone(),
                        typ,
                        command,
                    });
                }
            }
        }

        let terminator = match &process.terminator {
            Terminator::Do {
                span,
                name,
                usage,
                typ: (),
                command,
            } => {
                let typ = self
                    .get_variable_or_error(span, name)
                    .unwrap_or_else(|error| {
                        emit(error);
                        Type::Fail(span.clone())
                    });
                let command = match &typ {
                    Type::Hole(_, _, hole) => {
                        let (command, inferred) =
                            self.infer_terminal_command(span, name, command, emit);
                        hole.add_upper_bound(inferred);
                        command
                    }
                    Type::DualHole(_, _, hole) => {
                        let (command, inferred) =
                            self.infer_terminal_command(span, name, command, emit);
                        hole.add_lower_bound(inferred.dual(Span::None));
                        command
                    }
                    _ => {
                        self.check_terminal_command(
                            None,
                            span,
                            name,
                            &typ,
                            command,
                            &ProcessAnalyzerMode::Check,
                            emit,
                        )
                        .0
                    }
                };
                Terminator::Do {
                    span: span.clone(),
                    name: name.clone(),
                    usage: usage.clone(),
                    typ,
                    command,
                }
            }
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
            } => self.check_process_poll(
                span, kind, driver, point, clients, name, captures, then, else_, emit,
            ),
            Terminator::Submit {
                span,
                driver,
                point,
                values,
                captures,
            } => self.check_process_submit(span, driver, point, values, captures, emit),
            Terminator::Unreachable(span) => self.check_process_unreachable(span, emit),
            Terminator::Block(span, index, body, then) => {
                self.check_process_block(span, *index, body, then, emit)
            }
            Terminator::Goto(span, index, caps) => {
                self.check_process_goto(span, *index, caps, emit)
            }
        };
        Arc::new(Process::new(steps, terminator))
    }

    fn check_process_poll(
        &mut self,
        span: &Span,
        kind: &PollKind,
        driver: &LocalName,
        point: &LocalName,
        clients: &[Arc<Expression<(), S>>],
        name: &LocalName,
        captures: &Captures,
        then: &Process<(), S>,
        else_: &Process<(), S>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> Terminator<Type<S>, S> {
        let is_repoll = matches!(kind, PollKind::Repoll);

        let preserved_vars: IndexMap<_, _> = self
            .variables
            .iter()
            .filter(|&(n, _)| captures.names.contains_key(n))
            .map(|(n, t)| (n.clone(), t.clone()))
            .collect();

        let mut typed_clients = Vec::with_capacity(clients.len());

        let mut base;
        let mut then_ctx;
        let name_typ;

        if is_repoll {
            let (poll_driver, poll_pool_type, poll_points, poll_current_point) =
                match self.poll.as_ref() {
                    Some(poll) => (
                        poll.driver.clone(),
                        poll.pool_type.clone(),
                        poll.points.clone(),
                        poll.current_point.clone(),
                    ),
                    None => {
                        emit(TypeError::RepollOutsidePoll(span.clone()));
                        return Terminator::Unreachable(span.clone());
                    }
                };
            if poll_driver != *driver {
                emit(TypeError::RepollOutsidePoll(span.clone()));
                return Terminator::Unreachable(span.clone());
            }
            if self.get_variable(driver).is_none() {
                emit(TypeError::RepollOutsidePoll(span.clone()));
                return Terminator::Unreachable(span.clone());
            }

            let mut point_client_type = poll_points
                .get(&poll_current_point)
                .expect("current poll-point missing from poll scope")
                .client_type
                .clone();

            for client in clients {
                let (typed, typ) = self.infer_expression(None, client, emit);
                typed_clients.push(typed);
                let mut typ = typ;
                loop {
                    let next = typ.expand_definition(&self.type_defs).unwrap_or_else(|e| {
                        emit(e);
                        Type::Fail(span.clone())
                    });
                    if next == typ {
                        break;
                    }
                    typ = next;
                }
                let Type::Recursive { .. } = typ else {
                    emit(TypeError::PollClientMustBeRecursive(span.clone(), typ));
                    continue;
                };
                if !typ
                    .require_assignable_to(&poll_pool_type, &self.type_defs)
                    .unwrap_or(true)
                {
                    emit(TypeError::SubmittedClientNotAssignableToPoll(
                        span.clone(),
                        typ.clone(),
                        poll_pool_type.clone(),
                    ));
                }
                point_client_type = union_types(&self.type_defs, span, &point_client_type, &typ)
                    .unwrap_or_else(|e| {
                        emit(e);
                        Type::Fail(span.clone())
                    });
            }

            base = self.clone();

            let Type::Recursive {
                asc: point_asc,
                label: point_label,
                body: point_body,
                display_hint,
                ..
            } = point_client_type.clone()
            else {
                panic!("poll point client type must be recursive");
            };
            name_typ = Type::expand_recursive(
                &point_asc,
                &point_label,
                &point_body,
                display_hint.0.as_ref(),
            )
            .unwrap_or_else(|e| {
                emit(e);
                Type::Fail(span.clone())
            });

            let Some(base_poll) = base.poll.as_mut() else {
                panic!("repoll without a poll scope after validation");
            };
            if base_poll.driver != *driver {
                panic!("repoll driver does not match poll scope");
            }
            if base_poll
                .points
                .insert(
                    point.clone(),
                    PollPointScope {
                        client_type: point_client_type,
                        preserved: Arc::new(preserved_vars),
                    },
                )
                .is_some()
            {
                panic!("poll-point {} already registered", point);
            }
            base_poll.current_point = point.clone();

            then_ctx = base.clone();
        } else {
            if clients.is_empty() {
                emit(TypeError::PollMustHaveAtLeastOneClient(span.clone()));
                return Terminator::Unreachable(span.clone());
            }

            let mut client_type = None;
            for client in clients {
                let (typed, typ) = self.infer_expression(None, client, emit);
                typed_clients.push(typed);
                client_type = Some(match client_type {
                    None => typ,
                    Some(prev) => {
                        union_types(&self.type_defs, span, &prev, &typ).unwrap_or_else(|e| {
                            emit(e);
                            Type::Fail(span.clone())
                        })
                    }
                });
            }

            let mut client_type = client_type.expect("clients is not empty");
            loop {
                let next = client_type
                    .expand_definition(&self.type_defs)
                    .unwrap_or_else(|e| {
                        emit(e);
                        Type::Fail(span.clone())
                    });
                if next == client_type {
                    break;
                }
                client_type = next;
            }

            base = self.clone();

            let Type::Recursive {
                span: typ_span,
                asc,
                label,
                body,
                display_hint,
            } = client_type.clone()
            else {
                emit(TypeError::PollClientMustBeRecursive(
                    span.clone(),
                    client_type,
                ));
                return Terminator::Unreachable(span.clone());
            };

            let pool_type = client_type.clone();

            let mut asc = asc.clone();
            let loop_id = LoopId::new();
            asc.insert(loop_id);
            let point_client_type = Type::Recursive {
                span: typ_span.clone(),
                asc: asc.clone(),
                label: label.clone(),
                body: body.clone(),
                display_hint: display_hint.clone(),
            };

            name_typ = Type::expand_recursive(&asc, &label, &body, display_hint.0.as_ref())
                .unwrap_or_else(|e| {
                    emit(e);
                    Type::Fail(span.clone())
                });

            then_ctx = base.clone();
            let prev_poll = then_ctx.poll.take();
            if let Some(prev_poll) = &prev_poll {
                then_ctx.variables.shift_remove(&prev_poll.driver);
            }
            then_ctx.poll_stash.push(prev_poll);
            then_ctx.poll = Some(PollScope {
                driver: driver.clone(),
                pool_type,
                points: IndexMap::from([(
                    point.clone(),
                    PollPointScope {
                        client_type: point_client_type,
                        preserved: Arc::new(preserved_vars),
                    },
                )]),
                current_point: point.clone(),
                token_span: span.clone(),
            });
        }

        if let Err(e) = then_ctx.put(span, driver.clone(), Type::Continue(span.clone())) {
            emit(e);
        }
        if let Err(e) = then_ctx.put(span, name.clone(), name_typ.clone()) {
            emit(e);
        }
        let typed_then = then_ctx.check_process(then, emit);

        base.blocks = then_ctx.blocks.clone();

        let mut else_ctx = base;
        if is_repoll {
            let current = else_ctx
                .poll
                .take()
                .expect("repoll else branch must have a poll scope");
            if current.driver != *driver {
                panic!("repoll else branch driver mismatch");
            }
            else_ctx.variables.shift_remove(&current.driver);
            let prev = else_ctx.poll_stash.pop().unwrap_or(None);
            if let Some(prev_poll) = &prev {
                if let Err(e) = else_ctx.put(
                    &prev_poll.token_span,
                    prev_poll.driver.clone(),
                    Type::Continue(prev_poll.token_span.clone()),
                ) {
                    emit(e);
                }
            }
            else_ctx.poll = prev;
        }

        let typed_else = else_ctx.check_process(else_, emit);

        self.variables.clear();

        Terminator::Poll {
            span: span.clone(),
            kind: kind.clone(),
            driver: driver.clone(),
            point: point.clone(),
            clients: typed_clients,
            name: name.clone(),
            name_typ,
            captures: captures.clone(),
            then: typed_then,
            else_: typed_else,
        }
    }

    fn check_process_submit(
        &mut self,
        span: &Span,
        driver: &LocalName,
        point: &LocalName,
        values: &[Arc<Expression<(), S>>],
        captures: &Captures,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> Terminator<Type<S>, S> {
        let (poll_pool_type, current_point_client_type, poll_point_client_type, preserved_vars) =
            match self.poll.as_ref() {
                Some(poll) => {
                    if &poll.driver != driver {
                        panic!("submit driver does not match poll scope");
                    }
                    let preserved = poll
                        .points
                        .get(point)
                        .cloned()
                        .unwrap_or_else(|| panic!("submit to unknown poll-point: {point}"));
                    let current_point_client_type = poll
                        .points
                        .get(&poll.current_point)
                        .expect("current poll-point missing from poll scope")
                        .client_type
                        .clone();
                    (
                        poll.pool_type.clone(),
                        current_point_client_type,
                        preserved.client_type.clone(),
                        preserved.preserved.clone(),
                    )
                }
                None => {
                    emit(TypeError::SubmitOutsidePoll(span.clone()));
                    return Terminator::Unreachable(span.clone());
                }
            };

        if !current_point_client_type
            .require_assignable_to(&poll_point_client_type, &self.type_defs)
            .unwrap_or(true)
        {
            emit(TypeError::SubmitCannotTargetPollPoint(
                span.clone(),
                current_point_client_type,
                poll_point_client_type.clone(),
            ));
        }

        let mut typed_values = Vec::with_capacity(values.len());
        for value in values {
            let (typed, typ) = self.infer_expression(None, value, emit);
            let mut typ = typ;
            loop {
                let next = typ.expand_definition(&self.type_defs).unwrap_or_else(|e| {
                    emit(e);
                    Type::Fail(span.clone())
                });
                if next == typ {
                    break;
                }
                typ = next;
            }
            if !typ
                .require_assignable_to(&poll_pool_type, &self.type_defs)
                .unwrap_or(true)
            {
                emit(TypeError::SubmittedClientNotAssignableToPoll(
                    span.clone(),
                    typ.clone(),
                    poll_pool_type.clone(),
                ));
            }
            if !typ
                .require_assignable_to(&poll_point_client_type, &self.type_defs)
                .unwrap_or(true)
            {
                emit(TypeError::SubmittedClientDoesNotDescend(span.clone()));
            }
            typed_values.push(typed);
        }

        for (var, type_at_poll) in preserved_vars.iter() {
            let Some(current_type) = self.get_variable(var) else {
                emit(TypeError::PollVariableNotPreserved(
                    span.clone(),
                    var.clone(),
                ));
                continue;
            };
            if !current_type
                .require_assignable_to(type_at_poll, &self.type_defs)
                .unwrap_or(true)
            {
                emit(TypeError::PollVariableChangedType(
                    span.clone(),
                    var.clone(),
                    current_type,
                    type_at_poll.clone(),
                ));
            }
        }

        if self.get_variable(driver).is_none() {
            emit(TypeError::SubmitOutsidePoll(span.clone()));
        }

        if let Err(e) = self.cannot_have_obligations(span) {
            emit(e);
        }
        self.variables.clear();

        Terminator::Submit {
            span: span.clone(),
            driver: driver.clone(),
            point: point.clone(),
            values: typed_values,
            captures: captures.clone(),
        }
    }

    fn check_process_unreachable(
        &mut self,
        span: &Span,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> Terminator<Type<S>, S> {
        let impossible = Type::either(vec![]);
        let mut exhaustive = false;
        for typ in self.variables.values() {
            match typ.is_definitely_assignable_to(&impossible, &self.type_defs) {
                Ok(true) => {
                    exhaustive = true;
                    break;
                }
                Ok(false) => {}
                Err(error) => {
                    emit(error);
                }
            }
        }
        if !exhaustive {
            emit(TypeError::NonExhaustiveIf(span.clone()));
        }
        self.variables.clear();
        Terminator::Unreachable(span.clone())
    }

    fn check_process_block(
        &mut self,
        span: &Span,
        index: usize,
        body: &Process<(), S>,
        then: &Process<(), S>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> Terminator<Type<S>, S> {
        let target_type_vars = self.type_defs.vars.clone();
        if self
            .blocks
            .insert(
                index,
                BlockScope {
                    target_type_vars,
                    paths: Vec::new(),
                },
            )
            .is_some()
        {
            panic!("block {} already defined", index);
        }
        let typed_then = self.check_process(then, emit);
        let scope = self
            .blocks
            .shift_remove(&index)
            .expect("block should have been registered");
        let mut target_type_defs = self.type_defs.clone();
        target_type_defs.vars = scope.target_type_vars;
        if scope.paths.is_empty() {
            self.type_defs = target_type_defs;
            // Ill-typed synthesized condition blocks can become unreachable during recovery.
            return Terminator::Block(
                span.clone(),
                index,
                Process::terminal(Terminator::Unreachable(span.clone())),
                typed_then,
            );
        }
        let free = body.free_variables();
        let contexts = filter_block_path_contexts(&target_type_defs, span, scope.paths, emit);
        let merged = merge_path_contexts(&target_type_defs, span, &contexts, &free, emit);

        let saved = self.variables.clone();
        self.variables = merged;
        self.type_defs = target_type_defs.clone();
        let typed_body = self.check_process(body, emit);
        self.variables = saved;
        self.type_defs = target_type_defs;

        Terminator::Block(span.clone(), index, typed_body, typed_then)
    }

    fn check_process_goto(
        &mut self,
        span: &Span,
        index: usize,
        caps: &Captures,
        _emit: &mut impl FnMut(TypeError<S>),
    ) -> Terminator<Type<S>, S> {
        let entry = self.blocks.get_mut(&index).unwrap();
        entry.paths.push(BlockPathContext {
            variables: self.variables.clone(),
            type_vars: self.type_defs.vars.clone(),
        });
        self.variables.clear();
        Terminator::Goto(span.clone(), index, caps.clone())
    }

    fn normalize_command_type(
        &self,
        span: &Span,
        typ: &Type<S>,
        expand_iterative: bool,
        expand_recursive: bool,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> Type<S> {
        let mut typ = typ.clone();
        loop {
            let next = match &typ {
                Type::Name(_, name, args) => self.type_defs.get(span, name, args),
                Type::DualName(_, name, args) => self.type_defs.get_dual(span, name, args),
                Type::Box(_, inner) => Ok((**inner).clone()),
                Type::DualBox(_, inner)
                    if inner
                        .satisfies_constraint(TypeConstraint::Box, &self.type_defs)
                        .unwrap_or(false) =>
                {
                    Ok(inner.clone().dual(Span::None))
                }
                Type::Iterative {
                    asc,
                    label,
                    body,
                    display_hint,
                    ..
                } if expand_iterative => {
                    Type::expand_iterative(span, asc, label, body, display_hint.0.as_ref())
                }
                Type::Recursive {
                    asc,
                    label,
                    body,
                    display_hint,
                    ..
                } if expand_recursive => {
                    Type::expand_recursive(asc, label, body, display_hint.0.as_ref())
                }
                _ => break,
            }
            .unwrap_or_else(|error| {
                emit(error);
                Type::Fail(span.clone())
            });
            if next == typ {
                break;
            }
            typ = next;
        }
        typ
    }

    fn check_step_command(
        &mut self,
        span: &Span,
        object: &LocalName,
        typ: &Type<S>,
        command: &Command<(), S>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> Command<Type<S>, S> {
        let typ = self.normalize_command_type(span, typ, true, true, emit);
        match command {
            Command::Noop => {
                self.put(span, object.clone(), typ).ok();
                Command::Noop
            }
            Command::Send(argument) => {
                let (argument_type, then_type, vars) = match &typ {
                    Type::Function(_, argument_type, then_type, vars) => (
                        (**argument_type).clone(),
                        (**then_type).clone(),
                        vars.as_slice(),
                    ),
                    _ => {
                        if !matches!(typ, Type::Fail(_)) {
                            emit(TypeError::InvalidOperation(
                                span.clone(),
                                Operation::Send,
                                typ.clone(),
                            ));
                        }
                        let fail = Type::Fail(span.clone());
                        (fail.clone(), fail, &[] as &[TypeParameter])
                    }
                };
                let (argument, then_type) = if vars.is_empty() {
                    (
                        self.check_expression(None, argument, &argument_type, emit),
                        then_type,
                    )
                } else {
                    let (argument_type, holes) = substitute_holes(&argument_type, vars)
                        .unwrap_or_else(|error| {
                            emit(error);
                            (Type::Fail(span.clone()), HashMap::new())
                        });
                    let argument = self.check_expression(None, argument, &argument_type, emit);
                    let inferred = resolve_holes(span, vars, &self.type_defs, holes)
                        .unwrap_or_else(|error| {
                            emit(error);
                            vars.iter()
                                .map(|var| (var.name.clone(), Type::Fail(span.clone())))
                                .collect()
                        });
                    let argument =
                        argument.map_types(&mut |typ| typ.substitute_inferred_holes(&inferred));
                    let then_type = then_type
                        .substitute(inferred.iter().map(|(name, typ)| (name, typ)).collect())
                        .unwrap_or_else(|error| {
                            emit(error);
                            Type::Fail(span.clone())
                        });
                    (argument, then_type)
                };
                self.put(span, object.clone(), then_type).ok();
                Command::Send(argument)
            }
            Command::Receive(parameter, annotation, (), type_parameters) => {
                let (param_type, then_type, expected_parameters) = match &typ {
                    Type::Pair(_, param_type, then_type, parameters)
                        if parameters.len() == type_parameters.len() =>
                    {
                        (
                            (**param_type).clone(),
                            (**then_type).clone(),
                            parameters.clone(),
                        )
                    }
                    _ => {
                        if !matches!(typ, Type::Fail(_)) {
                            emit(TypeError::InvalidOperation(
                                span.clone(),
                                Operation::Receive {
                                    generics: type_parameters.len(),
                                },
                                typ.clone(),
                            ));
                        }
                        let fail = Type::Fail(span.clone());
                        (fail.clone(), fail, type_parameters.clone())
                    }
                };
                let type_parameters =
                    self.resolve_type_parameters(type_parameters, &expected_parameters, emit);
                let substitutions: BTreeMap<_, _> = expected_parameters
                    .iter()
                    .map(|parameter| &parameter.name)
                    .zip(
                        type_parameters
                            .iter()
                            .map(|parameter| Type::Var(Span::None, parameter.name.clone())),
                    )
                    .collect();
                let param_type = param_type
                    .substitute(substitutions.iter().map(|(k, v)| (*k, v)).collect())
                    .unwrap_or_else(|error| {
                        emit(error);
                        Type::Fail(span.clone())
                    });
                let then_type = then_type
                    .substitute(substitutions.iter().map(|(k, v)| (*k, v)).collect())
                    .unwrap_or_else(|error| {
                        emit(error);
                        Type::Fail(span.clone())
                    });
                self.type_defs.extend_vars(type_parameters.iter().cloned());
                if let Some(annotation) = annotation {
                    if let Err(error) = self.type_defs.validate_type(annotation) {
                        emit(error);
                    }
                    if let Err(error) =
                        param_type.check_assignable(span, annotation, &self.type_defs)
                    {
                        emit(error);
                    }
                }
                self.put(span, parameter.clone(), param_type.clone()).ok();
                self.put(span, object.clone(), then_type).ok();
                Command::Receive(
                    parameter.clone(),
                    annotation.clone(),
                    param_type,
                    type_parameters,
                )
            }
            Command::Signal(chosen) => {
                let branch_type = match &typ {
                    Type::Choice(_, branches) => {
                        branches.get(chosen).cloned().unwrap_or_else(|| {
                            emit(TypeError::InvalidBranch(
                                span.clone(),
                                chosen.clone(),
                                typ.clone(),
                            ));
                            Type::Fail(span.clone())
                        })
                    }
                    _ => {
                        if !matches!(typ, Type::Fail(_)) {
                            emit(TypeError::InvalidOperation(
                                span.clone(),
                                Operation::Signal,
                                typ.clone(),
                            ));
                        }
                        Type::Fail(span.clone())
                    }
                };
                self.put(span, object.clone(), branch_type).ok();
                Command::Signal(chosen.clone())
            }
            Command::Continue => {
                if !matches!(typ, Type::Break(_) | Type::Fail(_)) {
                    emit(TypeError::InvalidOperation(
                        span.clone(),
                        Operation::Continue,
                        typ,
                    ));
                }
                Command::Continue
            }
            Command::SendType(argument) => {
                let then_type = match &typ {
                    Type::Forall(_, parameter, then_type) => {
                        self.check_type_constraint(span, parameter, argument, emit);
                        then_type
                            .clone()
                            .substitute(BTreeMap::from([(&parameter.name, argument)]))
                            .unwrap_or_else(|error| {
                                emit(error);
                                Type::Fail(span.clone())
                            })
                    }
                    _ => {
                        if !matches!(typ, Type::Fail(_)) {
                            emit(TypeError::InvalidOperation(
                                span.clone(),
                                Operation::SendType,
                                typ,
                            ));
                        }
                        Type::Fail(span.clone())
                    }
                };
                self.put(span, object.clone(), then_type).ok();
                Command::SendType(argument.clone())
            }
            Command::ReceiveType(parameter) => {
                let (parameter, then_type) = match &typ {
                    Type::Exists(_, expected, then_type) => {
                        let parameter = self.resolve_type_parameter(parameter, expected, emit);
                        let then_type = then_type
                            .clone()
                            .substitute(BTreeMap::from([(
                                &expected.name,
                                &Type::Var(span.clone(), parameter.name.clone()),
                            )]))
                            .unwrap_or_else(|error| {
                                emit(error);
                                Type::Fail(span.clone())
                            });
                        (parameter, then_type)
                    }
                    _ => {
                        if !matches!(typ, Type::Fail(_)) {
                            emit(TypeError::InvalidOperation(
                                span.clone(),
                                Operation::ReceiveType,
                                typ,
                            ));
                        }
                        (parameter.clone(), Type::Fail(span.clone()))
                    }
                };
                self.type_defs.insert_var(parameter.clone());
                self.put(span, object.clone(), then_type).ok();
                Command::ReceiveType(parameter)
            }
        }
    }

    fn check_terminal_command(
        &mut self,
        inference_subject: Option<&LocalName>,
        span: &Span,
        object: &LocalName,
        typ: &Type<S>,
        command: &TerminalCommand<(), S>,
        mode: &ProcessAnalyzerMode,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (TerminalCommand<Type<S>, S>, Option<Type<S>>) {
        let typ = self.normalize_command_type(
            span,
            typ,
            !matches!(command, TerminalCommand::Link(_)),
            !matches!(
                command,
                TerminalCommand::Begin { .. } | TerminalCommand::Loop(..)
            ),
            emit,
        );
        match command {
            TerminalCommand::Link(expression) => {
                let expression = self.check_expression(
                    inference_subject,
                    expression,
                    &typ.clone().dual(Span::None),
                    emit,
                );
                if let Err(error) = self.cannot_have_obligations(span) {
                    emit(error);
                }
                (TerminalCommand::Link(expression), None)
            }
            TerminalCommand::Case(branches, processes, else_process) => self.check_command_case(
                span,
                object,
                &typ,
                branches,
                processes,
                else_process,
                mode,
                emit,
            ),
            TerminalCommand::Break => {
                if !matches!(typ, Type::Continue(_) | Type::Fail(_)) {
                    emit(TypeError::InvalidOperation(
                        span.clone(),
                        Operation::Break,
                        typ,
                    ));
                }
                if let Err(error) = self.cannot_have_obligations(span) {
                    emit(error);
                }
                (TerminalCommand::Break, None)
            }
            TerminalCommand::Begin {
                unfounded,
                label,
                captures,
                body,
            } => self.check_command_begin(
                inference_subject,
                span,
                object,
                &typ,
                *unfounded,
                label,
                captures,
                body,
                mode,
                emit,
            ),
            TerminalCommand::Loop(label, driver, captures) => self.check_command_loop(
                inference_subject,
                span,
                object,
                &typ,
                label,
                driver,
                captures,
                emit,
            ),
        }
    }

    fn check_command_case(
        &mut self,
        span: &Span,
        object: &LocalName,
        typ: &Type<S>,
        branches: &Arc<[LocalName]>,
        processes: &Box<[Arc<Process<(), S>>]>,
        else_process: &Option<Arc<Process<(), S>>>,
        mode: &ProcessAnalyzerMode,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (TerminalCommand<Type<S>, S>, Option<Type<S>>) {
        let Type::Either(_, branch_types) = typ else {
            if !matches!(typ, Type::Fail(_)) {
                emit(TypeError::InvalidOperation(
                    span.clone(),
                    Operation::Case,
                    typ.clone(),
                ));
            }
            let fail = Type::Fail(span.clone());
            let mut original_context = self.clone();
            let mut typed_processes = Vec::new();
            for process in processes.iter() {
                *self = original_context.clone();
                self.put(span, object.clone(), fail.clone()).ok();
                let (typed, _) = self.analyze_process(process, mode, emit);
                typed_processes.push(typed);
                original_context.blocks = self.blocks.clone();
            }
            let typed_else = else_process.as_ref().map(|p| {
                *self = original_context.clone();
                self.put(span, object.clone(), fail.clone()).ok();
                let (typed, _) = self.analyze_process(p, mode, emit);
                typed
            });
            return (
                TerminalCommand::Case(
                    branches.clone(),
                    typed_processes.into_boxed_slice(),
                    typed_else,
                ),
                None,
            );
        };

        let mut remaining_branches = branch_types.clone();

        let mut original_context = self.clone();
        let mut typed_processes = Vec::new();
        let mut inferred_type: Option<Type<S>> = None;

        for (branch, process) in branches.iter().zip(processes.iter()) {
            self.check_command_case_branch(
                span,
                object,
                typ,
                branch,
                process,
                &mut remaining_branches,
                &mut original_context,
                &mut typed_processes,
                &mut inferred_type,
                mode,
                emit,
            );
        }

        let typed_else_process = match else_process {
            Some(process) => Some(self.check_command_case_else(
                span,
                object,
                &mut remaining_branches,
                &original_context,
                process,
                &mut inferred_type,
                mode,
                emit,
            )),
            None => None,
        };

        if let Some((missing, _)) = remaining_branches.pop_first() {
            emit(TypeError::MissingBranch(
                span.clone(),
                missing.clone(),
                typ.clone(),
            ));
        }

        (
            TerminalCommand::Case(
                Arc::clone(branches),
                Box::from(typed_processes),
                typed_else_process,
            ),
            inferred_type,
        )
    }

    fn check_command_case_branch(
        &mut self,
        span: &Span,
        object: &LocalName,
        typ: &Type<S>,
        branch: &LocalName,
        process: &Arc<Process<(), S>>,
        remaining_branches: &mut BTreeMap<LocalName, Type<S>>,
        original_context: &mut Self,
        typed_processes: &mut Vec<Arc<Process<Type<S>, S>>>,
        inferred_type: &mut Option<Type<S>>,
        mode: &ProcessAnalyzerMode,
        emit: &mut impl FnMut(TypeError<S>),
    ) {
        *self = original_context.clone();

        let Some(branch_type) = remaining_branches.remove(branch) else {
            emit(TypeError::RedundantBranch(
                span.clone(),
                branch.clone(),
                typ.clone(),
            ));
            return;
        };
        if let Err(e) = self.put(span, object.clone(), branch_type) {
            emit(e);
        }
        let (process, inferred_in_branch) = self.analyze_process(process, mode, emit);
        typed_processes.push(process);
        self.merge_command_case_inferred_type(span, inferred_type, inferred_in_branch, emit);
        original_context.blocks = self.blocks.clone();
    }

    fn check_command_case_else(
        &mut self,
        span: &Span,
        object: &LocalName,
        remaining_branches: &mut BTreeMap<LocalName, Type<S>>,
        original_context: &Self,
        process: &Arc<Process<(), S>>,
        inferred_type: &mut Option<Type<S>>,
        mode: &ProcessAnalyzerMode,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> Arc<Process<Type<S>, S>> {
        *self = original_context.clone();
        let object_type = Type::Either(Span::None, std::mem::take(remaining_branches));
        if let Err(e) = self.put(span, object.clone(), object_type) {
            emit(e);
        }
        let (process, inferred_in_branch) = self.analyze_process(process, mode, emit);
        self.merge_command_case_inferred_type(span, inferred_type, inferred_in_branch, emit);
        process
    }

    fn merge_command_case_inferred_type(
        &self,
        span: &Span,
        inferred_type: &mut Option<Type<S>>,
        inferred_in_branch: Option<Type<S>>,
        emit: &mut impl FnMut(TypeError<S>),
    ) {
        *inferred_type = match (inferred_type.take(), inferred_in_branch) {
            (None, Some(t2)) => Some(t2),
            (Some(t1), Some(t2)) => Some(
                intersect_types(&self.type_defs, span, &t1, &t2).unwrap_or_else(|e| {
                    emit(e);
                    Type::Fail(span.clone())
                }),
            ),
            (t1, _) => t1,
        };
    }

    fn check_command_begin(
        &mut self,
        inference_subject: Option<&LocalName>,
        span: &Span,
        object: &LocalName,
        typ: &Type<S>,
        unfounded: bool,
        label: &Option<LocalName>,
        captures: &Captures,
        process: &Arc<Process<(), S>>,
        mode: &ProcessAnalyzerMode,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (TerminalCommand<Type<S>, S>, Option<Type<S>>) {
        if let Some(inference_subject) = inference_subject {
            emit(TypeError::TypeMustBeKnownAtThisPoint(
                span.clone(),
                inference_subject.clone(),
            ));
            let fail = Type::Fail(span.clone());
            self.put(span, object.clone(), fail.clone()).ok();
            let (process, inferred) = self.analyze_process(process, mode, emit);
            return (
                TerminalCommand::Begin {
                    unfounded,
                    label: label.clone(),
                    captures: captures.clone(),
                    body: process,
                },
                inferred,
            );
        }
        let Type::Recursive {
            span: typ_span,
            asc: typ_asc,
            label: typ_label,
            body: typ_body,
            display_hint,
        } = typ
        else {
            if !matches!(typ, Type::Fail(_)) {
                emit(TypeError::InvalidOperation(
                    span.clone(),
                    Operation::Begin,
                    typ.clone(),
                ));
            }
            let fail = Type::Fail(span.clone());
            self.put(span, object.clone(), fail.clone()).ok();
            let (process, inferred) = self.analyze_process(process, mode, emit);
            return (
                TerminalCommand::Begin {
                    unfounded,
                    label: label.clone(),
                    captures: captures.clone(),
                    body: process,
                },
                inferred,
            );
        };

        let mut typ_asc = typ_asc.clone();

        if !unfounded {
            let loop_id = LoopId::new();
            typ_asc.insert(loop_id);
        }
        self.loop_points.insert(
            label.clone(),
            (
                Type::Recursive {
                    span: typ_span.clone(),
                    asc: typ_asc.clone(),
                    label: typ_label.clone(),
                    body: typ_body.clone(),
                    display_hint: display_hint.clone(),
                },
                Arc::new(
                    self.variables
                        .iter()
                        .filter(|&(name, _)| captures.names.contains_key(name))
                        .map(|(name, typ)| (name.clone(), typ.clone()))
                        .collect::<IndexMap<_, _>>(),
                ),
            ),
        );

        let expanded =
            Type::expand_recursive(&typ_asc, typ_label, typ_body, display_hint.0.as_ref())
                .unwrap_or_else(|e| {
                    emit(e);
                    Type::Fail(span.clone())
                });
        if let Err(e) = self.put(span, object.clone(), expanded) {
            emit(e);
        }
        let (process, _inferred_type) = self.analyze_process(process, mode, emit);
        (
            TerminalCommand::Begin {
                unfounded,
                label: label.clone(),
                captures: captures.clone(),
                body: process,
            },
            None,
        )
    }

    fn check_command_loop(
        &mut self,
        inference_subject: Option<&LocalName>,
        span: &Span,
        _object: &LocalName,
        typ: &Type<S>,
        label: &Option<LocalName>,
        driver: &LocalName,
        captures: &Captures,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (TerminalCommand<Type<S>, S>, Option<Type<S>>) {
        if !matches!(typ, Type::Recursive { .. }) {
            if !matches!(typ, Type::Fail(_)) {
                emit(TypeError::InvalidOperation(
                    span.clone(),
                    Operation::Loop,
                    typ.clone(),
                ));
            }
            return (
                TerminalCommand::Loop(label.clone(), driver.clone(), captures.clone()),
                None,
            );
        }
        let Some((driver_type, variables)) = self.loop_points.get(label).cloned() else {
            emit(TypeError::NoSuchLoopPoint(span.clone(), label.clone()));
            return (TerminalCommand::Break, None);
        };
        if let Err(e) = self.put(span, driver.clone(), typ.clone()) {
            emit(e);
        }

        if let (Type::Recursive { asc: asc1, .. }, Type::Recursive { asc: asc2, .. }) =
            (typ, &driver_type)
        {
            for loop_id in asc2 {
                if !asc1.contains(loop_id) {
                    emit(TypeError::DoesNotDescendSubjectOfBegin(
                        span.clone(),
                        loop_id.clone(),
                    ));
                }
            }
        }

        let mut inferred_loop = None;

        for (var, type_at_begin) in variables.iter().chain([(driver, &driver_type)]) {
            if Some(var) == inference_subject {
                inferred_loop = Some(type_at_begin.clone());
                continue;
            }
            let Some(current_type) = self.get_variable(var) else {
                emit(TypeError::LoopVariableNotPreserved(
                    span.clone(),
                    var.clone(),
                ));
                continue;
            };
            if !current_type
                .require_assignable_to(type_at_begin, &self.type_defs)
                .unwrap_or(true)
            {
                emit(TypeError::LoopVariableChangedType(
                    span.clone(),
                    var.clone(),
                    current_type,
                    type_at_begin.clone(),
                ));
            }
        }
        if let Err(e) = self.cannot_have_obligations(span) {
            emit(e);
        }

        (
            TerminalCommand::Loop(label.clone(), driver.clone(), captures.clone()),
            inferred_loop.or(Some(Type::Self_(span.clone(), label.clone()))),
        )
    }

    pub(crate) fn infer_process(
        &mut self,
        process: &Process<(), S>,
        inference_subject: &LocalName,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Arc<Process<Type<S>, S>>, Type<S>) {
        let mut subject = inference_subject.clone();
        let mut typed_steps = vec![None; process.steps.len()];
        let mut frames = Vec::new();
        let mut completed_suffix = None;

        for (index, step) in process.steps.iter().enumerate() {
            match step {
                Step::Let {
                    span,
                    name,
                    annotation,
                    typ: (),
                    value,
                } => {
                    if annotation.is_none()
                        && let Expression::Variable(variable_span, variable, (), usage) =
                            value.as_ref()
                        && *variable == subject
                        && matches!(usage, VariableUsage::Move)
                    {
                        frames.push(InferenceFrame::Alias {
                            index,
                            span: span.clone(),
                            name: name.clone(),
                            annotation: annotation.clone(),
                            variable_span: variable_span.clone(),
                            variable: variable.clone(),
                            usage: usage.clone(),
                        });
                        subject = name.clone();
                        continue;
                    }

                    let (value, typ) = match annotation {
                        Some(typ) => (
                            self.check_expression(Some(&subject), value, typ, emit),
                            typ.clone(),
                        ),
                        None => self.infer_expression(Some(&subject), value, emit),
                    };
                    if let Err(error) = self.put(span, name.clone(), typ.clone()) {
                        emit(error);
                    }
                    typed_steps[index] = Some(Step::Let {
                        span: span.clone(),
                        name: name.clone(),
                        annotation: annotation.clone(),
                        typ,
                        value,
                    });
                }
                Step::Do {
                    span,
                    name,
                    usage,
                    typ: (),
                    command,
                } if *name == subject => match command {
                    Command::Noop => frames.push(InferenceFrame::Noop {
                        index,
                        span: span.clone(),
                        name: name.clone(),
                        usage: usage.clone(),
                    }),
                    Command::Send(argument) => {
                        let (argument, argument_type) =
                            self.infer_expression(Some(&subject), argument, emit);
                        frames.push(InferenceFrame::Send {
                            index,
                            span: span.clone(),
                            name: name.clone(),
                            usage: usage.clone(),
                            argument,
                            argument_type,
                        });
                    }
                    Command::Receive(parameter, annotation, (), type_parameters) => {
                        self.type_defs.extend_vars(type_parameters.iter().cloned());
                        let (parameter_type, failed) = match annotation {
                            Some(typ) => (typ.clone(), false),
                            None => {
                                emit(TypeError::ParameterTypeMustBeKnown(
                                    span.clone(),
                                    parameter.clone(),
                                ));
                                (Type::Fail(span.clone()), true)
                            }
                        };
                        if let Err(error) =
                            self.put(span, parameter.clone(), parameter_type.clone())
                        {
                            emit(error);
                        }
                        frames.push(InferenceFrame::Receive {
                            index,
                            span: span.clone(),
                            name: name.clone(),
                            usage: usage.clone(),
                            parameter: parameter.clone(),
                            annotation: annotation.clone(),
                            parameter_type,
                            type_parameters: type_parameters.clone(),
                            failed,
                        });
                    }
                    Command::Signal(chosen) => frames.push(InferenceFrame::Signal {
                        index,
                        span: span.clone(),
                        name: name.clone(),
                        usage: usage.clone(),
                        chosen: chosen.clone(),
                    }),
                    Command::Continue => {
                        typed_steps[index] = Some(Step::Do {
                            span: span.clone(),
                            name: name.clone(),
                            usage: usage.clone(),
                            typ: Type::Break(span.clone()),
                            command: Command::Continue,
                        });
                        let suffix = Process::new(
                            process.steps[index + 1..].to_vec(),
                            process.terminator.clone(),
                        );
                        let suffix = Arc::unwrap_or_clone(self.check_process(&suffix, emit));
                        for (offset, step) in suffix.steps.into_iter().enumerate() {
                            typed_steps[index + 1 + offset] = Some(step);
                        }
                        completed_suffix = Some((suffix.terminator, Type::Break(span.clone())));
                        break;
                    }
                    Command::SendType(argument) => {
                        emit(TypeError::TypeMustBeKnownAtThisPoint(
                            span.clone(),
                            subject.clone(),
                        ));
                        frames.push(InferenceFrame::SendTypeFailure {
                            index,
                            span: span.clone(),
                            name: name.clone(),
                            usage: usage.clone(),
                            argument: argument.clone(),
                        });
                    }
                    Command::ReceiveType(parameter) => {
                        self.type_defs.insert_var(parameter.clone());
                        frames.push(InferenceFrame::ReceiveType {
                            index,
                            span: span.clone(),
                            name: name.clone(),
                            usage: usage.clone(),
                            parameter: parameter.clone(),
                        });
                    }
                },
                Step::Do {
                    span,
                    name,
                    usage,
                    typ: (),
                    command,
                } => {
                    let typ = self
                        .get_variable_or_error(span, name)
                        .unwrap_or_else(|error| {
                            emit(error);
                            Type::Fail(span.clone())
                        });
                    let command = self.check_step_command(span, name, &typ, command, emit);
                    typed_steps[index] = Some(Step::Do {
                        span: span.clone(),
                        name: name.clone(),
                        usage: usage.clone(),
                        typ,
                        command,
                    });
                }
            }
        }

        let (terminator, mut inferred_type) = match completed_suffix {
            Some(completed) => completed,
            None => self.infer_terminator(&process.terminator, &subject, emit),
        };

        for frame in frames.into_iter().rev() {
            match frame {
                InferenceFrame::Alias {
                    index,
                    span,
                    name,
                    annotation,
                    variable_span,
                    variable,
                    usage,
                } => {
                    typed_steps[index] = Some(Step::Let {
                        span,
                        name,
                        annotation,
                        typ: inferred_type.clone(),
                        value: Arc::new(Expression::Variable(
                            variable_span,
                            variable,
                            inferred_type.clone(),
                            usage,
                        )),
                    });
                }
                InferenceFrame::Noop {
                    index,
                    span,
                    name,
                    usage,
                } => {
                    typed_steps[index] = Some(Step::Do {
                        span,
                        name,
                        usage,
                        typ: inferred_type.clone(),
                        command: Command::Noop,
                    });
                }
                InferenceFrame::Send {
                    index,
                    span,
                    name,
                    usage,
                    argument,
                    argument_type,
                } => {
                    inferred_type = Type::Function(
                        span.clone(),
                        Box::new(argument_type),
                        Box::new(inferred_type),
                        vec![],
                    );
                    typed_steps[index] = Some(Step::Do {
                        span,
                        name,
                        usage,
                        typ: inferred_type.clone(),
                        command: Command::Send(argument),
                    });
                }
                InferenceFrame::Receive {
                    index,
                    span,
                    name,
                    usage,
                    parameter,
                    annotation,
                    parameter_type,
                    type_parameters,
                    failed,
                } => {
                    inferred_type = if failed {
                        Type::Fail(span.clone())
                    } else {
                        Type::Pair(
                            span.clone(),
                            Box::new(parameter_type.clone()),
                            Box::new(inferred_type),
                            type_parameters.clone(),
                        )
                    };
                    typed_steps[index] = Some(Step::Do {
                        span,
                        name,
                        usage,
                        typ: inferred_type.clone(),
                        command: Command::Receive(
                            parameter,
                            annotation,
                            parameter_type,
                            type_parameters,
                        ),
                    });
                }
                InferenceFrame::Signal {
                    index,
                    span,
                    name,
                    usage,
                    chosen,
                } => {
                    inferred_type = Type::Choice(
                        span.clone(),
                        BTreeMap::from([(chosen.clone(), inferred_type)]),
                    );
                    typed_steps[index] = Some(Step::Do {
                        span,
                        name,
                        usage,
                        typ: inferred_type.clone(),
                        command: Command::Signal(chosen),
                    });
                }
                InferenceFrame::ReceiveType {
                    index,
                    span,
                    name,
                    usage,
                    parameter,
                } => {
                    inferred_type =
                        Type::Exists(span.clone(), parameter.clone(), Box::new(inferred_type));
                    typed_steps[index] = Some(Step::Do {
                        span,
                        name,
                        usage,
                        typ: inferred_type.clone(),
                        command: Command::ReceiveType(parameter),
                    });
                }
                InferenceFrame::SendTypeFailure {
                    index,
                    span,
                    name,
                    usage,
                    argument,
                } => {
                    inferred_type = Type::Fail(span.clone());
                    typed_steps[index] = Some(Step::Do {
                        span,
                        name,
                        usage,
                        typ: inferred_type.clone(),
                        command: Command::SendType(argument),
                    });
                }
            }
        }

        let typed_steps = typed_steps
            .into_iter()
            .map(|step| step.expect("inference should type every process step"))
            .collect();
        (
            Arc::new(Process::new(typed_steps, terminator)),
            inferred_type,
        )
    }

    fn infer_terminator(
        &mut self,
        terminator: &Terminator<(), S>,
        inference_subject: &LocalName,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Terminator<Type<S>, S>, Type<S>) {
        match terminator {
            Terminator::Do {
                span,
                name,
                usage,
                typ: (),
                command,
            } if name == inference_subject => {
                let (command, typ) =
                    self.infer_terminal_command(span, inference_subject, command, emit);
                (
                    Terminator::Do {
                        span: span.clone(),
                        name: name.clone(),
                        usage: usage.clone(),
                        typ: typ.clone(),
                        command,
                    },
                    typ,
                )
            }
            Terminator::Do {
                span,
                name,
                usage,
                typ: (),
                command,
            } => {
                let typ = self
                    .get_variable_or_error(span, name)
                    .unwrap_or_else(|error| {
                        emit(error);
                        Type::Fail(span.clone())
                    });
                let (command, inferred) = self.check_terminal_command(
                    Some(inference_subject),
                    span,
                    name,
                    &typ,
                    command,
                    &ProcessAnalyzerMode::Infer(inference_subject.clone()),
                    emit,
                );
                let inferred = inferred.unwrap_or_else(|| {
                    emit(TypeError::TypeMustBeKnownAtThisPoint(
                        span.clone(),
                        inference_subject.clone(),
                    ));
                    Type::Fail(span.clone())
                });
                (
                    Terminator::Do {
                        span: span.clone(),
                        name: name.clone(),
                        usage: usage.clone(),
                        typ,
                        command,
                    },
                    inferred,
                )
            }
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
            } => self.infer_process_poll(
                span,
                kind,
                driver,
                point,
                clients,
                name,
                captures,
                then,
                else_,
                inference_subject,
                emit,
            ),
            Terminator::Submit {
                span,
                driver,
                point,
                values,
                captures,
            } => self.infer_process_submit(
                span,
                driver,
                point,
                values,
                captures,
                inference_subject,
                emit,
            ),
            Terminator::Unreachable(span) => self.infer_process_unreachable(span, emit),
            Terminator::Block(span, index, body, then) => {
                self.infer_process_block(span, *index, body, then, inference_subject, emit)
            }
            Terminator::Goto(span, index, captures) => {
                self.infer_process_goto(span, *index, captures, emit)
            }
        }
    }

    fn infer_terminal_command(
        &mut self,
        span: &Span,
        subject: &LocalName,
        command: &TerminalCommand<(), S>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (TerminalCommand<Type<S>, S>, Type<S>) {
        match command {
            TerminalCommand::Link(expression) => {
                let (expression, typ) = self.infer_expression(Some(subject), expression, emit);
                if let Err(error) = self.cannot_have_obligations(span) {
                    emit(error);
                }
                (TerminalCommand::Link(expression), typ.dual(Span::None))
            }
            TerminalCommand::Case(branches, processes, else_process) => {
                self.infer_command_case(span, subject, branches, processes, else_process, emit)
            }
            TerminalCommand::Break => {
                if let Err(error) = self.cannot_have_obligations(span) {
                    emit(error);
                }
                (TerminalCommand::Break, Type::Continue(span.clone()))
            }
            TerminalCommand::Begin { .. } => {
                emit(TypeError::TypeMustBeKnownAtThisPoint(
                    span.clone(),
                    subject.clone(),
                ));
                (TerminalCommand::Break, Type::Fail(span.clone()))
            }
            TerminalCommand::Loop(label, driver, captures) => {
                self.infer_command_loop(span, label, driver, captures, emit)
            }
        }
    }

    fn infer_process_poll(
        &mut self,
        span: &Span,
        kind: &PollKind,
        driver: &LocalName,
        point: &LocalName,
        clients: &[Arc<Expression<(), S>>],
        name: &LocalName,
        captures: &Captures,
        then: &Arc<Process<(), S>>,
        else_: &Arc<Process<(), S>>,
        inference_subject: &LocalName,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Terminator<Type<S>, S>, Type<S>) {
        let is_repoll = matches!(kind, PollKind::Repoll);

        let preserved_vars: IndexMap<_, _> = self
            .variables
            .iter()
            .filter(|&(n, _)| captures.names.contains_key(n))
            .map(|(n, t)| (n.clone(), t.clone()))
            .collect();

        let mut typed_clients = Vec::with_capacity(clients.len());

        let mut base;
        let mut then_ctx;
        let name_typ;

        if is_repoll {
            let (poll_driver, poll_pool_type, poll_points, poll_current_point) =
                match self.poll.as_ref() {
                    Some(poll) => (
                        poll.driver.clone(),
                        poll.pool_type.clone(),
                        poll.points.clone(),
                        poll.current_point.clone(),
                    ),
                    None => {
                        emit(TypeError::RepollOutsidePoll(span.clone()));
                        return (
                            Terminator::Unreachable(span.clone()),
                            Type::Fail(span.clone()),
                        );
                    }
                };
            if poll_driver != *driver {
                emit(TypeError::RepollOutsidePoll(span.clone()));
                return (
                    Terminator::Unreachable(span.clone()),
                    Type::Fail(span.clone()),
                );
            }

            if self.get_variable(driver).is_none() {
                emit(TypeError::RepollOutsidePoll(span.clone()));
                return (
                    Terminator::Unreachable(span.clone()),
                    Type::Fail(span.clone()),
                );
            }

            let mut point_client_type = poll_points
                .get(&poll_current_point)
                .expect("current poll-point missing from poll scope")
                .client_type
                .clone();

            for client in clients {
                let (typed, typ) = self.infer_expression(Some(inference_subject), client, emit);
                typed_clients.push(typed);
                let mut typ = typ;
                loop {
                    let next = typ.expand_definition(&self.type_defs).unwrap_or_else(|e| {
                        emit(e);
                        Type::Fail(span.clone())
                    });
                    if next == typ {
                        break;
                    }
                    typ = next;
                }
                let Type::Recursive { .. } = typ else {
                    emit(TypeError::PollClientMustBeRecursive(span.clone(), typ));
                    continue;
                };
                if !typ
                    .require_assignable_to(&poll_pool_type, &self.type_defs)
                    .unwrap_or(true)
                {
                    emit(TypeError::SubmittedClientNotAssignableToPoll(
                        span.clone(),
                        typ.clone(),
                        poll_pool_type.clone(),
                    ));
                }
                point_client_type = union_types(&self.type_defs, span, &point_client_type, &typ)
                    .unwrap_or_else(|e| {
                        emit(e);
                        Type::Fail(span.clone())
                    });
            }

            base = self.clone();

            let Type::Recursive {
                asc: point_asc,
                label: point_label,
                body: point_body,
                display_hint,
                ..
            } = point_client_type.clone()
            else {
                panic!("poll point client type must be recursive");
            };
            name_typ = Type::expand_recursive(
                &point_asc,
                &point_label,
                &point_body,
                display_hint.0.as_ref(),
            )
            .unwrap_or_else(|e| {
                emit(e);
                Type::Fail(span.clone())
            });

            let Some(base_poll) = base.poll.as_mut() else {
                panic!("repoll without a poll scope after validation");
            };
            if base_poll.driver != *driver {
                panic!("repoll driver does not match poll scope");
            }
            if base_poll
                .points
                .insert(
                    point.clone(),
                    PollPointScope {
                        client_type: point_client_type,
                        preserved: Arc::new(preserved_vars),
                    },
                )
                .is_some()
            {
                panic!("poll-point {} already registered", point);
            }
            base_poll.current_point = point.clone();

            then_ctx = base.clone();
        } else {
            if clients.is_empty() {
                emit(TypeError::PollMustHaveAtLeastOneClient(span.clone()));
                return (
                    Terminator::Unreachable(span.clone()),
                    Type::Fail(span.clone()),
                );
            }

            let mut client_type = None;
            for client in clients {
                let (client_expr, typ) =
                    self.infer_expression(Some(inference_subject), client, emit);
                typed_clients.push(client_expr);
                client_type = Some(match client_type {
                    None => typ,
                    Some(prev) => {
                        union_types(&self.type_defs, span, &prev, &typ).unwrap_or_else(|e| {
                            emit(e);
                            Type::Fail(span.clone())
                        })
                    }
                });
            }

            let mut client_type = client_type.expect("clients is not empty");
            loop {
                let next = client_type
                    .expand_definition(&self.type_defs)
                    .unwrap_or_else(|e| {
                        emit(e);
                        Type::Fail(span.clone())
                    });
                if next == client_type {
                    break;
                }
                client_type = next;
            }

            base = self.clone();

            let Type::Recursive {
                span: typ_span,
                asc,
                label,
                body,
                display_hint,
            } = client_type.clone()
            else {
                emit(TypeError::PollClientMustBeRecursive(
                    span.clone(),
                    client_type,
                ));
                return (
                    Terminator::Unreachable(span.clone()),
                    Type::Fail(span.clone()),
                );
            };

            let pool_type = client_type.clone();

            let mut asc = asc.clone();
            let loop_id = LoopId::new();
            asc.insert(loop_id);
            let point_client_type = Type::Recursive {
                span: typ_span.clone(),
                asc: asc.clone(),
                label: label.clone(),
                body: body.clone(),
                display_hint: display_hint.clone(),
            };

            name_typ = Type::expand_recursive(&asc, &label, &body, display_hint.0.as_ref())
                .unwrap_or_else(|e| {
                    emit(e);
                    Type::Fail(span.clone())
                });

            then_ctx = base.clone();
            let prev_poll = then_ctx.poll.take();
            if let Some(prev_poll) = &prev_poll {
                then_ctx.variables.shift_remove(&prev_poll.driver);
            }
            then_ctx.poll_stash.push(prev_poll);
            then_ctx.poll = Some(PollScope {
                driver: driver.clone(),
                pool_type,
                points: IndexMap::from([(
                    point.clone(),
                    PollPointScope {
                        client_type: point_client_type,
                        preserved: Arc::new(preserved_vars),
                    },
                )]),
                current_point: point.clone(),
                token_span: span.clone(),
            });
        }

        if let Err(e) = then_ctx.put(span, driver.clone(), Type::Continue(span.clone())) {
            emit(e);
        }
        if let Err(e) = then_ctx.put(span, name.clone(), name_typ.clone()) {
            emit(e);
        }
        let (typed_then, then_type) = then_ctx.infer_process(then, inference_subject, emit);

        base.blocks = then_ctx.blocks.clone();

        let mut else_ctx = base;
        if is_repoll {
            let current = else_ctx
                .poll
                .take()
                .expect("repoll else branch must have a poll scope");
            if current.driver != *driver {
                panic!("repoll else branch driver mismatch");
            }
            else_ctx.variables.shift_remove(&current.driver);
            let prev = else_ctx.poll_stash.pop().unwrap_or(None);
            if let Some(prev_poll) = &prev {
                if let Err(e) = else_ctx.put(
                    &prev_poll.token_span,
                    prev_poll.driver.clone(),
                    Type::Continue(prev_poll.token_span.clone()),
                ) {
                    emit(e);
                }
            }
            else_ctx.poll = prev;
        }

        let (typed_else, else_type) = else_ctx.infer_process(else_, inference_subject, emit);

        self.variables.clear();

        (
            Terminator::Poll {
                span: span.clone(),
                kind: kind.clone(),
                driver: driver.clone(),
                point: point.clone(),
                clients: typed_clients,
                name: name.clone(),
                name_typ,
                captures: captures.clone(),
                then: typed_then,
                else_: typed_else,
            },
            intersect_types(&self.type_defs, span, &then_type, &else_type).unwrap_or_else(|e| {
                emit(e);
                Type::Fail(span.clone())
            }),
        )
    }

    fn infer_process_submit(
        &mut self,
        span: &Span,
        driver: &LocalName,
        point: &LocalName,
        values: &[Arc<Expression<(), S>>],
        captures: &Captures,
        inference_subject: &LocalName,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Terminator<Type<S>, S>, Type<S>) {
        let (poll_pool_type, current_point_client_type, poll_point_client_type, preserved_vars) =
            match self.poll.as_ref() {
                Some(poll) => {
                    if &poll.driver != driver {
                        panic!("submit driver does not match poll scope");
                    }
                    let preserved = poll
                        .points
                        .get(point)
                        .cloned()
                        .unwrap_or_else(|| panic!("submit to unknown poll-point: {point}"));
                    let current_point_client_type = poll
                        .points
                        .get(&poll.current_point)
                        .expect("current poll-point missing from poll scope")
                        .client_type
                        .clone();
                    (
                        poll.pool_type.clone(),
                        current_point_client_type,
                        preserved.client_type.clone(),
                        preserved.preserved.clone(),
                    )
                }
                None => {
                    emit(TypeError::SubmitOutsidePoll(span.clone()));
                    return (
                        Terminator::Unreachable(span.clone()),
                        Type::Fail(span.clone()),
                    );
                }
            };

        if !current_point_client_type
            .require_assignable_to(&poll_point_client_type, &self.type_defs)
            .unwrap_or(true)
        {
            emit(TypeError::SubmitCannotTargetPollPoint(
                span.clone(),
                current_point_client_type,
                poll_point_client_type.clone(),
            ));
        }

        let mut typed_values = Vec::with_capacity(values.len());
        for value in values {
            let (typed, typ) = self.infer_expression(Some(inference_subject), value, emit);
            let mut typ = typ;
            loop {
                let next = typ.expand_definition(&self.type_defs).unwrap_or_else(|e| {
                    emit(e);
                    Type::Fail(span.clone())
                });
                if next == typ {
                    break;
                }
                typ = next;
            }
            if !typ
                .require_assignable_to(&poll_pool_type, &self.type_defs)
                .unwrap_or(true)
            {
                emit(TypeError::SubmittedClientNotAssignableToPoll(
                    span.clone(),
                    typ.clone(),
                    poll_pool_type.clone(),
                ));
            }
            if !typ
                .require_assignable_to(&poll_point_client_type, &self.type_defs)
                .unwrap_or(true)
            {
                emit(TypeError::SubmittedClientDoesNotDescend(span.clone()));
            }
            typed_values.push(typed);
        }

        for (var, type_at_poll) in preserved_vars.iter() {
            let Some(current_type) = self.get_variable(var) else {
                emit(TypeError::PollVariableNotPreserved(
                    span.clone(),
                    var.clone(),
                ));
                continue;
            };
            if !current_type
                .require_assignable_to(type_at_poll, &self.type_defs)
                .unwrap_or(true)
            {
                emit(TypeError::PollVariableChangedType(
                    span.clone(),
                    var.clone(),
                    current_type,
                    type_at_poll.clone(),
                ));
            }
        }

        if self.get_variable(driver).is_none() {
            emit(TypeError::SubmitOutsidePoll(span.clone()));
        }

        if let Err(e) = self.cannot_have_obligations(span) {
            emit(e);
        }
        self.variables.clear();

        (
            Terminator::Submit {
                span: span.clone(),
                driver: driver.clone(),
                point: point.clone(),
                values: typed_values,
                captures: captures.clone(),
            },
            Type::choice(vec![]),
        )
    }

    fn infer_process_unreachable(
        &mut self,
        span: &Span,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Terminator<Type<S>, S>, Type<S>) {
        let impossible = Type::either(vec![]);
        let mut exhaustive = false;
        for typ in self.variables.values() {
            match typ.is_definitely_assignable_to(&impossible, &self.type_defs) {
                Ok(true) => {
                    exhaustive = true;
                    break;
                }
                Ok(false) => {}
                Err(e) => {
                    emit(e);
                }
            }
        }
        if !exhaustive {
            emit(TypeError::NonExhaustiveIf(span.clone()));
        }
        self.variables.clear();
        (Terminator::Unreachable(span.clone()), Type::choice(vec![]))
    }

    fn infer_process_block(
        &mut self,
        span: &Span,
        index: usize,
        body: &Arc<Process<(), S>>,
        then: &Arc<Process<(), S>>,
        inference_subject: &LocalName,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Terminator<Type<S>, S>, Type<S>) {
        let target_type_vars = self.type_defs.vars.clone();
        if self
            .blocks
            .insert(
                index,
                BlockScope {
                    target_type_vars,
                    paths: Vec::new(),
                },
            )
            .is_some()
        {
            panic!("block {} already defined", index);
        }
        let (typed_then, then_type) = self.infer_process(then, inference_subject, emit);
        let scope = self
            .blocks
            .shift_remove(&index)
            .expect("block should have been registered");
        let mut target_type_defs = self.type_defs.clone();
        target_type_defs.vars = scope.target_type_vars;
        if scope.paths.is_empty() {
            self.type_defs = target_type_defs;
            // Ill-typed synthesized condition blocks can become unreachable during recovery.
            return (
                Terminator::Block(
                    span.clone(),
                    index,
                    Process::terminal(Terminator::Unreachable(span.clone())),
                    typed_then,
                ),
                then_type,
            );
        }
        let free = body.free_variables();
        let contexts = filter_block_path_contexts(&target_type_defs, span, scope.paths, emit)
            .into_iter()
            .map(|mut ctx| {
                ctx.shift_remove(inference_subject);
                ctx
            })
            .collect();
        let merged = merge_path_contexts(&target_type_defs, span, &contexts, &free, emit);

        let saved = self.variables.clone();
        self.variables = merged;
        self.type_defs = target_type_defs.clone();
        let (typed_body, body_type) = self.infer_process(body, inference_subject, emit);
        self.variables = saved;
        self.type_defs = target_type_defs.clone();

        let final_type = intersect_types(&target_type_defs, span, &then_type, &body_type)
            .unwrap_or_else(|e| {
                emit(e);
                Type::Fail(span.clone())
            });

        (
            Terminator::Block(span.clone(), index, typed_body, typed_then),
            final_type,
        )
    }

    fn infer_process_goto(
        &mut self,
        span: &Span,
        index: usize,
        caps: &Captures,
        _emit: &mut impl FnMut(TypeError<S>),
    ) -> (Terminator<Type<S>, S>, Type<S>) {
        let entry = self.blocks.get_mut(&index).unwrap();
        entry.paths.push(BlockPathContext {
            variables: self.variables.clone(),
            type_vars: self.type_defs.vars.clone(),
        });
        self.variables.clear();
        (
            Terminator::Goto(span.clone(), index, caps.clone()),
            Type::choice(vec![]),
        )
    }

    fn infer_command_case(
        &mut self,
        span: &Span,
        subject: &LocalName,
        branches: &Arc<[LocalName]>,
        processes: &Box<[Arc<Process<(), S>>]>,
        else_process: &Option<Arc<Process<(), S>>>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (TerminalCommand<Type<S>, S>, Type<S>) {
        if else_process.is_some() {
            emit(TypeError::TypeMustBeKnownAtThisPoint(
                span.clone(),
                subject.clone(),
            ));
            let mut original_context = self.clone();
            let mut typed_processes = Vec::new();
            for (_branch, process) in branches.iter().zip(processes.iter()) {
                *self = original_context.clone();
                let (process, _typ) = self.infer_process(process, subject, emit);
                typed_processes.push(process);
                original_context.blocks = self.blocks.clone();
            }
            let typed_else = else_process.as_ref().map(|p| {
                *self = original_context.clone();
                let (process, _) = self.infer_process(p, subject, emit);
                process
            });
            return (
                TerminalCommand::Case(Arc::clone(branches), Box::from(typed_processes), typed_else),
                Type::Fail(span.clone()),
            );
        }

        let mut original_context = self.clone();
        let mut typed_processes = Vec::new();
        let mut branch_types = BTreeMap::new();

        for (branch, process) in branches.iter().zip(processes.iter()) {
            *self = original_context.clone();
            let (process, typ) = self.infer_process(process, subject, emit);
            typed_processes.push(process);
            branch_types.insert(branch.clone(), typ);
            original_context.blocks = self.blocks.clone();
        }

        (
            TerminalCommand::Case(Arc::clone(branches), Box::from(typed_processes), None),
            Type::Either(span.clone(), branch_types),
        )
    }

    fn infer_command_loop(
        &mut self,
        span: &Span,
        label: &Option<LocalName>,
        driver: &LocalName,
        captures: &Captures,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (TerminalCommand<Type<S>, S>, Type<S>) {
        let Some((driver_type, variables)) = self.loop_points.get(label).cloned() else {
            emit(TypeError::NoSuchLoopPoint(span.clone(), label.clone()));
            return (TerminalCommand::Break, Type::Fail(span.clone()));
        };

        for (var, type_at_begin) in variables.as_ref() {
            let Some(current_type) = self.get_variable(var) else {
                emit(TypeError::LoopVariableNotPreserved(
                    span.clone(),
                    var.clone(),
                ));
                continue;
            };
            if !current_type
                .require_assignable_to(type_at_begin, &self.type_defs)
                .unwrap_or(true)
            {
                emit(TypeError::LoopVariableChangedType(
                    span.clone(),
                    var.clone(),
                    current_type,
                    type_at_begin.clone(),
                ));
            }
        }
        if let Err(e) = self.cannot_have_obligations(span) {
            emit(e);
        }

        (
            TerminalCommand::Loop(label.clone(), driver.clone(), captures.clone()),
            driver_type,
        )
    }

    pub(crate) fn check_expression(
        &mut self,
        inference_subject: Option<&LocalName>,
        expression: &Expression<(), S>,
        target_type: &Type<S>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> Arc<Expression<Type<S>, S>> {
        match expression {
            Expression::Global(span, name, ()) => {
                self.check_expression_global(span, name, target_type, emit)
            }
            Expression::Variable(span, name, (), usage) => self.check_expression_variable(
                span,
                name,
                usage,
                inference_subject,
                target_type,
                emit,
            ),
            Expression::Box(span, captures, expression, ()) => self.check_expression_box(
                span,
                captures,
                expression,
                inference_subject,
                target_type,
                emit,
            ),
            Expression::Chan {
                span,
                captures,
                chan_name: channel,
                chan_annotation: annotation,
                process,
                ..
            } => self.check_expression_chan(
                span,
                captures,
                channel,
                annotation,
                process,
                inference_subject,
                target_type,
                emit,
            ),
            Expression::Primitive(span, value, ()) => {
                self.check_expression_primitive(span, value, target_type, emit)
            }
            Expression::External(f, ()) => self.check_expression_external(f, target_type, emit),
        }
    }

    pub(crate) fn infer_expression(
        &mut self,
        inference_subject: Option<&LocalName>,
        expression: &Expression<(), S>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Arc<Expression<Type<S>, S>>, Type<S>) {
        match expression {
            Expression::Global(span, name, ()) => self.infer_expression_global(span, name, emit),
            Expression::Variable(span, name, (), usage) => {
                self.infer_expression_variable(span, name, usage, inference_subject, emit)
            }
            Expression::Box(span, captures, expression, ()) => {
                self.infer_expression_box(span, captures, expression, inference_subject, emit)
            }
            Expression::Chan {
                span,
                captures,
                chan_name: channel,
                chan_annotation: annotation,
                process,
                ..
            } => self.infer_expression_chan(
                span,
                captures,
                channel,
                annotation,
                process,
                inference_subject,
                emit,
            ),
            Expression::Primitive(span, value, ()) => {
                self.infer_expression_primitive(span, value, emit)
            }
            Expression::External(_f, ()) => self.infer_expression_external(emit),
        }
    }

    fn check_expression_global(
        &mut self,
        span: &Span,
        name: &super::super::language::GlobalName<S>,
        target_type: &Type<S>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> Arc<Expression<Type<S>, S>> {
        let typ = self.get_global(span, name, emit);
        if let Err(e) = typ.check_assignable(span, target_type, &self.type_defs) {
            emit(e);
        }
        Arc::new(Expression::Global(span.clone(), name.clone(), typ.clone()))
    }

    fn check_expression_variable(
        &mut self,
        span: &Span,
        name: &LocalName,
        usage: &VariableUsage,
        inference_subject: Option<&LocalName>,
        target_type: &Type<S>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> Arc<Expression<Type<S>, S>> {
        if Some(name) == inference_subject {
            emit(TypeError::TypeMustBeKnownAtThisPoint(
                span.clone(),
                name.clone(),
            ));
            return Arc::new(Expression::Variable(
                span.clone(),
                name.clone(),
                Type::Fail(span.clone()),
                usage.clone(),
            ));
        }

        let typ = self.get_variable_or_error(span, name).unwrap_or_else(|e| {
            emit(e);
            Type::Fail(span.clone())
        });
        if let Err(e) = typ.check_assignable(span, target_type, &self.type_defs) {
            emit(e);
        }
        if !typ.is_linear(&self.type_defs).unwrap_or(false) {
            if let Err(e) = self.put(span, name.clone(), typ.clone()) {
                emit(e);
            }
        }
        Arc::new(Expression::Variable(
            span.clone(),
            name.clone(),
            typ.clone(),
            usage.clone(),
        ))
    }

    fn check_expression_box(
        &mut self,
        span: &Span,
        captures: &Captures,
        expression: &Arc<Expression<(), S>>,
        inference_subject: Option<&LocalName>,
        target_type: &Type<S>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> Arc<Expression<Type<S>, S>> {
        if let Some(inference_subject) = inference_subject {
            if captures.names.contains_key(inference_subject) {
                emit(TypeError::TypeMustBeKnownAtThisPoint(
                    span.clone(),
                    inference_subject.clone(),
                ));
                return Arc::new(Expression::Box(
                    span.clone(),
                    captures.clone(),
                    Arc::new(Expression::Primitive(
                        span.clone(),
                        Primitive::Number(Number::Int(num_bigint::BigInt::ZERO)),
                        Type::Fail(span.clone()),
                    )),
                    target_type.clone(),
                ));
            }
        }
        let mut context = self.split();
        if let Err(e) = self.capture(inference_subject, captures, true, &mut context) {
            emit(e);
        }
        let mut target_inner_type = target_type.clone();
        loop {
            match target_inner_type
                .expand_definition(&self.type_defs)
                .unwrap_or_else(|e| {
                    emit(e);
                    Type::Fail(span.clone())
                }) {
                Type::Box(_, inner) => target_inner_type = *inner,
                Type::Recursive {
                    span: _,
                    asc,
                    label,
                    body,
                    display_hint,
                } => {
                    target_inner_type =
                        Type::expand_recursive(&asc, &label, &body, display_hint.0.as_ref())
                            .unwrap_or_else(|e| {
                                emit(e);
                                Type::Fail(span.clone())
                            });
                }
                Type::Iterative {
                    span: iter_span,
                    asc,
                    label,
                    body,
                    display_hint,
                } => {
                    target_inner_type = Type::expand_iterative(
                        &iter_span,
                        &asc,
                        &label,
                        &body,
                        display_hint.0.as_ref(),
                    )
                    .unwrap_or_else(|e| {
                        emit(e);
                        Type::Fail(span.clone())
                    });
                }
                _ => break,
            }
        }
        let expression =
            self.check_expression(inference_subject, expression, &target_inner_type, emit);
        Arc::new(Expression::Box(
            span.clone(),
            captures.clone(),
            expression,
            target_type.clone(),
        ))
    }

    fn check_expression_chan(
        &mut self,
        span: &Span,
        captures: &Captures,
        channel: &LocalName,
        annotation: &Option<Type<S>>,
        process: &Arc<Process<(), S>>,
        inference_subject: Option<&LocalName>,
        target_type: &Type<S>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> Arc<Expression<Type<S>, S>> {
        let target_dual = target_type.clone().dual(Span::None);
        let (chan_type, expr_type) = match annotation {
            Some(annotated_type) => {
                if let Err(e) = self.type_defs.validate_type(annotated_type) {
                    emit(e);
                }
                if let Err(e) = annotated_type.check_assignable(span, &target_dual, &self.type_defs)
                {
                    emit(e);
                }
                (annotated_type.clone(), target_type)
            }
            None => (target_dual, target_type),
        };
        let mut context = self.split();
        if let Err(e) = self.capture(inference_subject, captures, false, &mut context) {
            emit(e);
        }
        if let Err(e) = context.put(span, channel.clone(), chan_type.clone()) {
            emit(e);
        }
        let process = context.check_process(process, emit);
        Arc::new(Expression::Chan {
            span: span.clone(),
            captures: captures.clone(),
            chan_name: channel.clone(),
            chan_annotation: annotation.clone(),
            chan_type,
            expr_type: expr_type.clone(),
            process,
        })
    }

    fn check_expression_primitive(
        &mut self,
        span: &Span,
        value: &par_runtime::primitive::Primitive,
        target_type: &Type<S>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> Arc<Expression<Type<S>, S>> {
        let typ = get_primitive_type(value);
        if let Err(e) = typ.check_assignable(span, target_type, &self.type_defs) {
            emit(e);
        }
        Arc::new(Expression::Primitive(span.clone(), value.clone(), typ))
    }

    fn check_expression_external(
        &mut self,
        f: &par_runtime::linker::Unlinked,
        target_type: &Type<S>,
        _emit: &mut impl FnMut(TypeError<S>),
    ) -> Arc<Expression<Type<S>, S>> {
        Arc::new(Expression::External(f.clone(), target_type.clone()))
    }

    fn infer_expression_global(
        &mut self,
        span: &Span,
        name: &super::super::language::GlobalName<S>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Arc<Expression<Type<S>, S>>, Type<S>) {
        let typ = self.get_global(span, name, emit);
        (
            Arc::new(Expression::Global(span.clone(), name.clone(), typ.clone())),
            typ.clone(),
        )
    }

    fn infer_expression_variable(
        &mut self,
        span: &Span,
        name: &LocalName,
        usage: &VariableUsage,
        inference_subject: Option<&LocalName>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Arc<Expression<Type<S>, S>>, Type<S>) {
        if Some(name) == inference_subject {
            emit(TypeError::TypeMustBeKnownAtThisPoint(
                span.clone(),
                name.clone(),
            ));
            return (
                Arc::new(Expression::Variable(
                    span.clone(),
                    name.clone(),
                    Type::Fail(span.clone()),
                    usage.clone(),
                )),
                Type::Fail(span.clone()),
            );
        }
        let typ = self.get_variable_or_error(span, name).unwrap_or_else(|e| {
            emit(e);
            Type::Fail(span.clone())
        });
        if !typ.is_linear(&self.type_defs).unwrap_or(false) {
            if let Err(e) = self.put(span, name.clone(), typ.clone()) {
                emit(e);
            }
        }
        (
            Arc::new(Expression::Variable(
                span.clone(),
                name.clone(),
                typ.clone(),
                usage.clone(),
            )),
            typ,
        )
    }

    fn infer_expression_box(
        &mut self,
        span: &Span,
        captures: &Captures,
        expression: &Arc<Expression<(), S>>,
        inference_subject: Option<&LocalName>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Arc<Expression<Type<S>, S>>, Type<S>) {
        if let Some(inference_subject) = inference_subject {
            if captures.names.contains_key(inference_subject) {
                emit(TypeError::TypeMustBeKnownAtThisPoint(
                    span.clone(),
                    inference_subject.clone(),
                ));
                return (
                    Arc::new(Expression::Box(
                        span.clone(),
                        captures.clone(),
                        Arc::new(Expression::Primitive(
                            span.clone(),
                            Primitive::Number(Number::Int(num_bigint::BigInt::ZERO)),
                            Type::Fail(span.clone()),
                        )),
                        Type::Fail(span.clone()),
                    )),
                    Type::Fail(span.clone()),
                );
            }
        }
        let mut context = self.split();
        if let Err(e) = self.capture(inference_subject, captures, true, &mut context) {
            emit(e);
        }
        let (expression, typ) = self.infer_expression(inference_subject, expression, emit);
        let typ = Type::Box(span.clone(), Box::new(typ.clone()));
        (
            Arc::new(Expression::Box(
                span.clone(),
                captures.clone(),
                expression,
                typ.clone(),
            )),
            typ,
        )
    }

    fn infer_expression_chan(
        &mut self,
        span: &Span,
        captures: &Captures,
        channel: &LocalName,
        annotation: &Option<Type<S>>,
        process: &Arc<Process<(), S>>,
        inference_subject: Option<&LocalName>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Arc<Expression<Type<S>, S>>, Type<S>) {
        let mut context = self.split();
        if let Err(e) = self.capture(inference_subject, captures, false, &mut context) {
            emit(e);
        }
        let (process, typ) = match annotation {
            Some(typ) => {
                if let Err(e) = self.type_defs.validate_type(typ) {
                    emit(e);
                }
                if let Err(e) = context.put(span, channel.clone(), typ.clone()) {
                    emit(e);
                }
                (context.check_process(process, emit), typ.clone())
            }
            None => context.infer_process(process, channel, emit),
        };
        let dual = typ.clone().dual(Span::None);
        (
            Arc::new(Expression::Chan {
                span: span.clone(),
                captures: captures.clone(),
                chan_name: channel.clone(),
                chan_annotation: annotation.clone(),
                chan_type: typ,
                expr_type: dual.clone(),
                process,
            }),
            dual,
        )
    }

    fn infer_expression_primitive(
        &mut self,
        span: &Span,
        value: &par_runtime::primitive::Primitive,
        _emit: &mut impl FnMut(TypeError<S>),
    ) -> (Arc<Expression<Type<S>, S>>, Type<S>) {
        let typ = get_primitive_type(value);
        (
            Arc::new(Expression::Primitive(
                span.clone(),
                value.clone(),
                typ.clone(),
            )),
            typ,
        )
    }

    fn infer_expression_external(
        &mut self,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Arc<Expression<Type<S>, S>>, Type<S>) {
        emit(TypeError::TypeMustBeKnownAtThisPoint(
            Span::None,
            LocalName::error(),
        ));
        (
            Arc::new(Expression::Primitive(
                Span::None,
                Primitive::Number(Number::Int(num_bigint::BigInt::ZERO)),
                Type::Fail(Span::None),
            )),
            Type::Fail(Span::None),
        )
    }
}

fn free_type_vars<S>(typ: &Type<S>) -> IndexSet<LocalName> {
    fn inner<S>(typ: &Type<S>, bound: &mut Vec<LocalName>, out: &mut IndexSet<LocalName>) {
        match typ {
            Type::Var(_, name) | Type::DualVar(_, name) => {
                if !bound.iter().any(|bound| bound == name) {
                    out.insert(name.clone());
                }
            }
            Type::Name(_, _, args) | Type::DualName(_, _, args) => {
                for arg in args {
                    inner(arg, bound, out);
                }
            }
            Type::Box(_, body) | Type::DualBox(_, body) => inner(body, bound, out),
            Type::Pair(_, left, right, vars) | Type::Function(_, left, right, vars) => {
                for var in vars {
                    bound.push(var.name.clone());
                }
                inner(left, bound, out);
                inner(right, bound, out);
                for _ in vars {
                    bound.pop();
                }
            }
            Type::Either(_, branches) | Type::Choice(_, branches) => {
                for branch in branches.values() {
                    inner(branch, bound, out);
                }
            }
            Type::Recursive { body, .. } | Type::Iterative { body, .. } => {
                inner(body, bound, out);
            }
            Type::Exists(_, param, body) | Type::Forall(_, param, body) => {
                bound.push(param.name.clone());
                inner(body, bound, out);
                bound.pop();
            }
            Type::Primitive(..)
            | Type::DualPrimitive(..)
            | Type::Hole(..)
            | Type::DualHole(..)
            | Type::Break(..)
            | Type::Continue(..)
            | Type::Self_(..)
            | Type::DualSelf(..)
            | Type::Fail(..) => {}
        }
    }

    let mut out = IndexSet::new();
    inner(typ, &mut Vec::new(), &mut out);
    out
}

fn filter_block_path_contexts<S: Clone + Eq + std::hash::Hash>(
    target_type_defs: &TypeDefs<S>,
    span: &Span,
    paths: Vec<BlockPathContext<S>>,
    emit: &mut impl FnMut(TypeError<S>),
) -> Vec<IndexMap<LocalName, Type<S>>> {
    paths
        .into_iter()
        .map(|path| {
            let mut path_type_defs = target_type_defs.clone();
            path_type_defs.vars = path.type_vars;
            path.variables
                .into_iter()
                .filter_map(|(name, typ)| {
                    let escapes_type_scope = free_type_vars(&typ)
                        .iter()
                        .any(|var| !target_type_defs.vars.contains_key(var));

                    if !escapes_type_scope {
                        return Some((name, typ));
                    }

                    if typ.is_linear(&path_type_defs).unwrap_or(true) {
                        emit(TypeError::VariableEscapesTypeScope(
                            span.clone(),
                            name.clone(),
                        ));
                    }

                    None
                })
                .collect()
        })
        .collect()
}

fn merge_path_contexts<S: Clone + Eq + std::hash::Hash>(
    typedefs: &TypeDefs<S>,
    span: &Span,
    paths: &Vec<IndexMap<LocalName, Type<S>>>,
    free_vars: &IndexSet<LocalName>,
    emit: &mut impl FnMut(TypeError<S>),
) -> IndexMap<LocalName, Type<S>> {
    // Collect all variable names present in any path.
    let mut all_names: IndexSet<LocalName> = IndexSet::new();
    for map in paths {
        all_names.extend(map.keys().cloned());
    }

    let mut merged_variables = IndexMap::new();
    for name in all_names {
        let used = free_vars.contains(&name);
        let mut present_types: Vec<Type<S>> = Vec::new();
        let mut missing = false;
        for map in paths {
            if let Some(t) = map.get(&name) {
                present_types.push(t.clone());
            } else {
                missing = true;
            }
        }

        let is_linear = present_types
            .iter()
            .any(|t| t.is_linear(typedefs).unwrap_or(true));

        let is_absurd = present_types.iter().any(|t| {
            t.is_definitely_assignable_to(&Type::either(vec![]), typedefs)
                .unwrap_or(false)
        });

        // If any present type is Fail and the variable is missing from some paths,
        // its presence is unreliable due to error recovery — drop it to avoid
        // cascading errors.
        let is_fail = present_types.iter().any(|t| matches!(t, Type::Fail(_)));

        if (!used && !is_linear && !is_absurd) || (is_fail && missing) {
            // Drop it.
            continue;
        }

        // Variable used or linear: must be present everywhere.
        if missing {
            emit(TypeError::MergeVariableMissing(span.clone(), name.clone()));
            continue;
        }

        let mut acc = present_types
            .get(0)
            .cloned()
            .expect("at least one type when not missing");
        for next in present_types.iter().skip(1) {
            acc = match union_types(typedefs, span, &acc, next) {
                Ok(t) => t,
                Err(_) => {
                    emit(TypeError::MergeVariableTypesCannotBeUnified(
                        span.clone(),
                        name.clone(),
                        acc.clone(),
                        next.clone(),
                    ));
                    acc
                }
            };
        }
        merged_variables.insert(name.clone(), acc);
    }
    merged_variables
}

#[cfg(test)]
mod flat_ir_tests {
    use super::*;
    use crate::frontend_impl::language::Unresolved;

    #[test]
    fn checks_long_flat_process_with_a_loop() {
        const STEP_COUNT: usize = 20_000;
        let subject = LocalName::from(arcstr::literal!("subject"));
        let mut builder = crate::frontend_impl::process::ProcessBuilder::new();
        for _ in 0..STEP_COUNT {
            builder.push(Step::Do {
                span: Span::None,
                name: subject.clone(),
                usage: VariableUsage::Unknown,
                typ: (),
                command: Command::Noop,
            });
        }
        let process = builder.finish(Terminator::Do {
            span: Span::None,
            name: subject.clone(),
            usage: VariableUsage::Unknown,
            typ: (),
            command: TerminalCommand::Break,
        });

        let mut context =
            Context::<Unresolved>::new(TypeDefs::default(), IndexMap::new(), IndexMap::new());
        context
            .put(&Span::None, subject, Type::Continue(Span::None))
            .unwrap();
        let mut errors = Vec::new();
        let checked = context.check_process(&process, &mut |error| errors.push(error));
        assert!(errors.is_empty(), "{errors:#?}");
        assert_eq!(checked.steps.len(), STEP_COUNT);
    }

    #[test]
    fn inferred_continue_checks_the_remaining_suffix() {
        let subject = LocalName::from(arcstr::literal!("subject"));
        let other = LocalName::from(arcstr::literal!("other"));
        let process = Process::do_step(
            Span::None,
            subject.clone(),
            VariableUsage::Unknown,
            (),
            Command::Continue,
            Process::do_terminal(
                Span::None,
                other.clone(),
                VariableUsage::Unknown,
                (),
                TerminalCommand::Break,
            ),
        );
        let mut context =
            Context::<Unresolved>::new(TypeDefs::default(), IndexMap::new(), IndexMap::new());
        context
            .put(&Span::None, other, Type::Continue(Span::None))
            .unwrap();
        let mut errors = Vec::new();
        let (checked, inferred) =
            context.infer_process(&process, &subject, &mut |error| errors.push(error));
        assert!(errors.is_empty(), "{errors:#?}");
        assert!(matches!(inferred, Type::Break(_)));
        assert!(matches!(
            &checked.steps[0],
            Step::Do {
                typ: Type::Break(_),
                command: Command::Continue,
                ..
            }
        ));
    }
}
