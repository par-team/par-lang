use super::super::language::{LocalName, TypeConstraint, TypeParameter};
use super::super::process::{Captures, Command, Expression, PollKind, Process, VariableUsage};
use super::context::{BlockPathContext, BlockScope, PollPointScope, PollScope};
use super::core::{LoopId, Operation, Type, get_primitive_type};
use super::error::TypeError;
use super::lattice::union_types;
use super::{Context, TypeDefs};
use crate::frontend::TypeError::TypeMustBeKnownAtThisPoint;
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
        match process {
            Process::Let {
                span,
                name,
                annotation,
                typ: (),
                value: expression,
                then: process,
            } => match annotation {
                Some(annotated_type) => self.check_process_let_annotated(
                    span,
                    name,
                    annotation,
                    annotated_type,
                    expression,
                    process,
                    emit,
                ),
                None => self
                    .check_process_let_inferred(span, name, annotation, expression, process, emit),
            },

            Process::Do {
                span,
                name: object,
                usage,
                typ: (),
                command,
            } => self.check_process_do(span, object, usage, command, emit),

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
            } => self.check_process_poll(
                span, kind, driver, point, clients, name, captures, then, else_, emit,
            ),

            Process::Submit {
                span,
                driver,
                point,
                values,
                captures,
            } => self.check_process_submit(span, driver, point, values, captures, emit),

            Process::Unreachable(span) => self.check_process_unreachable(span, emit),

            Process::Block(span, index, body, then) => {
                self.check_process_block(span, *index, body, then, emit)
            }

            Process::Goto(span, index, caps) => self.check_process_goto(span, *index, caps, emit),
        }
    }

    fn check_process_let_annotated(
        &mut self,
        span: &Span,
        name: &LocalName,
        annotation: &Option<Type<S>>,
        annotated_type: &Type<S>,
        expression: &Expression<(), S>,
        process: &Process<(), S>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> Arc<Process<Type<S>, S>> {
        if let Err(e) = self.type_defs.validate_type(annotated_type) {
            emit(e);
        }
        let expression = self.check_expression(None, expression, annotated_type, emit);
        let typ = annotated_type.clone();
        if let Err(e) = self.put(span, name.clone(), typ.clone()) {
            emit(e);
        }
        let process = self.check_process(process, emit);
        Arc::new(Process::Let {
            span: span.clone(),
            name: name.clone(),
            annotation: annotation.clone(),
            typ,
            value: expression,
            then: process,
        })
    }

    fn check_process_let_inferred(
        &mut self,
        span: &Span,
        name: &LocalName,
        annotation: &Option<Type<S>>,
        expression: &Expression<(), S>,
        process: &Process<(), S>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> Arc<Process<Type<S>, S>> {
        let (expression, typ) = self.infer_expression(None, expression, emit);
        if let Err(e) = self.put(span, name.clone(), typ.clone()) {
            emit(e);
        }
        let process = self.check_process(process, emit);
        Arc::new(Process::Let {
            span: span.clone(),
            name: name.clone(),
            annotation: annotation.clone(),
            typ,
            value: expression,
            then: process,
        })
    }

    fn check_process_do(
        &mut self,
        span: &Span,
        object: &LocalName,
        usage: &VariableUsage,
        command: &Command<(), S>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> Arc<Process<Type<S>, S>> {
        let typ = self
            .get_variable_or_error(span, object)
            .unwrap_or_else(|e| {
                emit(e);
                Type::Fail(span.clone())
            });
        let (command, _) = self.check_command(
            None,
            span,
            object,
            &typ,
            command,
            &ProcessAnalyzerMode::Check,
            emit,
        );

        Arc::new(Process::Do {
            span: span.clone(),
            name: object.clone(),
            usage: usage.clone(),
            typ,
            command,
        })
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
    ) -> Arc<Process<Type<S>, S>> {
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
                        return Arc::new(Process::Unreachable(span.clone()));
                    }
                };
            if poll_driver != *driver {
                emit(TypeError::RepollOutsidePoll(span.clone()));
                return Arc::new(Process::Unreachable(span.clone()));
            }
            if self.get_variable(driver).is_none() {
                emit(TypeError::RepollOutsidePoll(span.clone()));
                return Arc::new(Process::Unreachable(span.clone()));
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
                return Arc::new(Process::Unreachable(span.clone()));
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
                return Arc::new(Process::Unreachable(span.clone()));
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

        Arc::new(Process::Poll {
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
        })
    }

    fn check_process_submit(
        &mut self,
        span: &Span,
        driver: &LocalName,
        point: &LocalName,
        values: &[Arc<Expression<(), S>>],
        captures: &Captures,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> Arc<Process<Type<S>, S>> {
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
                    return Arc::new(Process::Unreachable(span.clone()));
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

        Arc::new(Process::Submit {
            span: span.clone(),
            driver: driver.clone(),
            point: point.clone(),
            values: typed_values,
            captures: captures.clone(),
        })
    }

    fn check_process_unreachable(
        &mut self,
        span: &Span,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> Arc<Process<Type<S>, S>> {
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
        Arc::new(Process::Unreachable(span.clone()))
    }

    fn check_process_block(
        &mut self,
        span: &Span,
        index: usize,
        body: &Process<(), S>,
        then: &Process<(), S>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> Arc<Process<Type<S>, S>> {
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
            return Arc::new(Process::Block(
                span.clone(),
                index,
                Arc::new(Process::Unreachable(span.clone())),
                typed_then,
            ));
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

        Arc::new(Process::Block(span.clone(), index, typed_body, typed_then))
    }

    fn check_process_goto(
        &mut self,
        span: &Span,
        index: usize,
        caps: &Captures,
        _emit: &mut impl FnMut(TypeError<S>),
    ) -> Arc<Process<Type<S>, S>> {
        let entry = self.blocks.get_mut(&index).unwrap();
        entry.paths.push(BlockPathContext {
            variables: self.variables.clone(),
            type_vars: self.type_defs.vars.clone(),
        });
        self.variables.clear();
        Arc::new(Process::Goto(span.clone(), index, caps.clone()))
    }

    fn check_command(
        &mut self,
        inference_subject: Option<&LocalName>,
        span: &Span,
        object: &LocalName,
        typ: &Type<S>,
        command: &Command<(), S>,
        mode: &ProcessAnalyzerMode,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Option<Type<S>>) {
        if let Type::Name(_, name, args) = typ {
            let expanded = self.type_defs.get(span, name, args).unwrap_or_else(|e| {
                emit(e);
                Type::Fail(span.clone())
            });
            return self.check_command(
                inference_subject,
                span,
                object,
                &expanded,
                command,
                mode,
                emit,
            );
        }
        if let Type::DualName(_, name, args) = typ {
            let expanded = self
                .type_defs
                .get_dual(span, name, args)
                .unwrap_or_else(|e| {
                    emit(e);
                    Type::Fail(span.clone())
                });
            return self.check_command(
                inference_subject,
                span,
                object,
                &expanded,
                command,
                mode,
                emit,
            );
        }
        if let Type::Box(_, inner) = typ {
            return self.check_command(inference_subject, span, object, inner, command, mode, emit);
        }
        if let Type::DualBox(_, inner) = typ {
            if inner
                .satisfies_constraint(TypeConstraint::Box, &self.type_defs)
                .unwrap_or(false)
            {
                return self.check_command(
                    inference_subject,
                    span,
                    object,
                    &inner.clone().dual(Span::None),
                    command,
                    mode,
                    emit,
                );
            }
        }
        if !matches!(command, Command::Link(_)) {
            if let Type::Iterative {
                asc: top_asc,
                label: top_label,
                body,
                display_hint,
                ..
            } = typ
            {
                let expanded =
                    Type::expand_iterative(span, top_asc, top_label, body, display_hint.0.as_ref())
                        .unwrap_or_else(|e| {
                            emit(e);
                            Type::Fail(span.clone())
                        });
                return self.check_command(
                    inference_subject,
                    span,
                    object,
                    &expanded,
                    command,
                    mode,
                    emit,
                );
            }
        }
        if !matches!(command, Command::Begin { .. } | Command::Loop(_, _, _)) {
            if let Type::Recursive {
                asc: top_asc,
                label: top_label,
                body,
                display_hint,
                ..
            } = typ
            {
                let expanded =
                    Type::expand_recursive(top_asc, top_label, body, display_hint.0.as_ref())
                        .unwrap_or_else(|e| {
                            emit(e);
                            Type::Fail(span.clone())
                        });
                return self.check_command(
                    inference_subject,
                    span,
                    object,
                    &expanded,
                    command,
                    mode,
                    emit,
                );
            }
        }

        self.check_command_normalized(inference_subject, span, object, typ, command, mode, emit)
    }

    fn check_command_normalized(
        &mut self,
        inference_subject: Option<&LocalName>,
        span: &Span,
        object: &LocalName,
        typ: &Type<S>,
        command: &Command<(), S>,
        mode: &ProcessAnalyzerMode,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Option<Type<S>>) {
        match typ {
            Type::Hole(_span, _name, hole) => {
                if let Some(inference_subject) = inference_subject {
                    emit(TypeMustBeKnownAtThisPoint(
                        span.clone(),
                        inference_subject.clone(),
                    ));
                    let fail = Type::Fail(span.clone());
                    self.put(span, object.clone(), fail.clone()).ok();
                    let (cmd, typ) = self.infer_command(span, object, command, emit);
                    hole.add_upper_bound(typ);
                    return (cmd, Some(fail));
                }
                let (cmd, typ) = self.infer_command(span, object, command, emit);
                hole.add_upper_bound(typ);
                return (cmd, None);
            }
            Type::DualHole(_span, _name, hole) => {
                if let Some(inference_subject) = inference_subject {
                    emit(TypeMustBeKnownAtThisPoint(
                        span.clone(),
                        inference_subject.clone(),
                    ));
                    let fail = Type::Fail(span.clone());
                    self.put(span, object.clone(), fail.clone()).ok();
                    let (cmd, typ) = self.infer_command(span, object, command, emit);
                    hole.add_lower_bound(typ.dual(Span::None));
                    return (cmd, None);
                }
                let (cmd, typ) = self.infer_command(span, object, command, emit);
                hole.add_lower_bound(typ.dual(Span::None));
                return (cmd, None);
            }
            _ => {}
        };
        match command {
            Command::Noop(process) => {
                self.put(span, object.clone(), typ.clone()).ok();
                let (process, inferred) = self.analyze_process(process, mode, emit);
                (Command::Noop(process), inferred)
            }
            Command::Link(expression) => self.check_command_link(span, typ, expression, emit),
            Command::Send(argument, process) => {
                self.check_command_send(span, object, typ, argument, process, mode, emit)
            }
            Command::Receive(parameter, annotation, (), process, type_parameters) => self
                .check_command_receive(
                    span,
                    object,
                    typ,
                    parameter,
                    annotation,
                    process,
                    type_parameters,
                    mode,
                    emit,
                ),
            Command::Signal(chosen, process) => {
                self.check_command_signal(span, object, typ, chosen, process, mode, emit)
            }
            Command::Case(branches, processes, else_process) => self.check_command_case(
                span,
                object,
                typ,
                branches,
                processes,
                else_process,
                mode,
                emit,
            ),
            Command::Break => {
                let Type::Continue(_) = typ else {
                    if !matches!(typ, Type::Fail(_)) {
                        emit(TypeError::InvalidOperation(
                            span.clone(),
                            Operation::Break,
                            typ.clone(),
                        ));
                    }
                    return (Command::Break, None);
                };
                if let Err(e) = self.cannot_have_obligations(span) {
                    emit(e);
                }
                (Command::Break, None)
            }
            Command::Continue(process) => {
                self.check_command_continue(span, typ, process, mode, emit)
            }
            Command::Begin {
                unfounded,
                label,
                captures,
                body: process,
            } => self.check_command_begin(
                inference_subject,
                span,
                object,
                typ,
                *unfounded,
                label,
                captures,
                process,
                mode,
                emit,
            ),
            Command::Loop(label, driver, captures) => self.check_command_loop(
                inference_subject,
                span,
                object,
                typ,
                label,
                driver,
                captures,
                emit,
            ),
            Command::SendType(argument, process) => {
                self.check_command_send_type(span, object, typ, argument, process, mode, emit)
            }
            Command::ReceiveType(parameter, process) => {
                self.check_command_receive_type(span, object, typ, parameter, process, mode, emit)
            }
        }
    }

    fn check_command_link(
        &mut self,
        span: &Span,
        typ: &Type<S>,
        expression: &Arc<Expression<(), S>>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Option<Type<S>>) {
        let expression =
            self.check_expression(None, expression, &typ.clone().dual(Span::None), emit);
        if let Err(e) = self.cannot_have_obligations(span) {
            emit(e);
        }
        (Command::Link(expression), None)
    }

    fn check_command_send(
        &mut self,
        span: &Span,
        object: &LocalName,
        typ: &Type<S>,
        argument: &Arc<Expression<(), S>>,
        process: &Arc<Process<(), S>>,
        mode: &ProcessAnalyzerMode,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Option<Type<S>>) {
        let Type::Function(_, argument_type, then_type, vars) = typ else {
            if !matches!(typ, Type::Fail(_)) {
                emit(TypeError::InvalidOperation(
                    span.clone(),
                    Operation::Send,
                    typ.clone(),
                ));
            }
            let fail = Type::Fail(span.clone());
            let argument = self.check_expression(None, argument, &fail, emit);
            self.put(span, object.clone(), fail.clone()).ok();
            let (process, inferred) = self.analyze_process(process, mode, emit);
            return (Command::Send(argument, process), inferred);
        };
        if vars.is_empty() {
            self.check_command_send_plain(
                span,
                object,
                argument,
                process,
                argument_type,
                then_type,
                mode,
                emit,
            )
        } else {
            self.check_command_send_generic(
                span,
                object,
                argument,
                process,
                argument_type,
                then_type,
                vars,
                mode,
                emit,
            )
        }
    }

    fn check_command_send_generic(
        &mut self,
        span: &Span,
        object: &LocalName,
        argument: &Arc<Expression<(), S>>,
        process: &Arc<Process<(), S>>,
        argument_type: &Type<S>,
        then_type: &Type<S>,
        vars: &[TypeParameter],
        mode: &ProcessAnalyzerMode,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Option<Type<S>>) {
        let (argument_type, holes_map) =
            substitute_holes(argument_type, vars).unwrap_or_else(|e| {
                emit(e);
                (Type::Fail(span.clone()), HashMap::new())
            });
        let argument = self.check_expression(None, argument, &argument_type, emit);
        let inferred_holes =
            resolve_holes(span, vars, &self.type_defs, holes_map).unwrap_or_else(|e| {
                emit(e);
                vars.iter()
                    .map(|var| (var.name.clone(), Type::Fail(span.clone())))
                    .collect()
            });
        let argument =
            argument.map_types(&mut |typ| typ.substitute_inferred_holes(&inferred_holes));
        let then_type = then_type
            .clone()
            .substitute(inferred_holes.iter().map(|(k, v)| (k, v)).collect())
            .unwrap_or_else(|e: TypeError<S>| {
                emit(e);
                Type::Fail(span.clone())
            });
        self.finish_check_command_send(span, object, argument, process, then_type, mode, emit)
    }

    fn check_command_send_plain(
        &mut self,
        span: &Span,
        object: &LocalName,
        argument: &Arc<Expression<(), S>>,
        process: &Arc<Process<(), S>>,
        argument_type: &Type<S>,
        then_type: &Type<S>,
        mode: &ProcessAnalyzerMode,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Option<Type<S>>) {
        let argument = self.check_expression(None, argument, argument_type, emit);
        self.finish_check_command_send(
            span,
            object,
            argument,
            process,
            then_type.clone(),
            mode,
            emit,
        )
    }

    fn finish_check_command_send(
        &mut self,
        span: &Span,
        object: &LocalName,
        argument: Arc<Expression<Type<S>, S>>,
        process: &Arc<Process<(), S>>,
        then_type: Type<S>,
        mode: &ProcessAnalyzerMode,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Option<Type<S>>) {
        if let Err(e) = self.put(span, object.clone(), then_type) {
            emit(e);
        }
        let (process, inferred_types) = self.analyze_process(process, mode, emit);
        (Command::Send(argument, process), inferred_types)
    }

    fn check_command_receive(
        &mut self,
        span: &Span,
        object: &LocalName,
        typ: &Type<S>,
        parameter: &LocalName,
        annotation: &Option<Type<S>>,
        process: &Arc<Process<(), S>>,
        type_parameters: &[TypeParameter],
        mode: &ProcessAnalyzerMode,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Option<Type<S>>) {
        let Type::Pair(_, param_type, then_type, type_names) = typ else {
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
            self.put(span, object.clone(), fail.clone()).ok();
            self.put(span, parameter.clone(), fail.clone()).ok();
            let (process, inferred) = self.analyze_process(process, mode, emit);
            return (
                Command::Receive(
                    parameter.clone(),
                    annotation.clone(),
                    fail,
                    process,
                    type_parameters.to_vec(),
                ),
                inferred,
            );
        };

        if type_parameters.len() != type_names.len() {
            emit(TypeError::InvalidOperation(
                span.clone(),
                Operation::Receive {
                    generics: type_parameters.len(),
                },
                typ.clone(),
            ));
            let fail = Type::Fail(span.clone());
            self.put(span, object.clone(), fail.clone()).ok();
            self.put(span, parameter.clone(), fail.clone()).ok();
            let (process, inferred) = self.analyze_process(process, mode, emit);
            return (
                Command::Receive(
                    parameter.clone(),
                    annotation.clone(),
                    fail,
                    process,
                    type_parameters.to_vec(),
                ),
                inferred,
            );
        }

        if type_names.is_empty() {
            self.check_command_receive_plain(
                span,
                object,
                parameter,
                annotation,
                process,
                type_names,
                param_type,
                then_type,
                type_parameters,
                mode,
                emit,
            )
        } else {
            self.check_command_receive_generic(
                span,
                object,
                parameter,
                annotation,
                process,
                type_names,
                param_type,
                then_type,
                type_parameters,
                mode,
                emit,
            )
        }
    }

    fn check_command_receive_generic(
        &mut self,
        span: &Span,
        object: &LocalName,
        parameter: &LocalName,
        annotation: &Option<Type<S>>,
        process: &Arc<Process<(), S>>,
        type_names: &[TypeParameter],
        param_type: &Type<S>,
        then_type: &Type<S>,
        type_parameters: &[TypeParameter],
        mode: &ProcessAnalyzerMode,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Option<Type<S>>) {
        let type_parameters = self.resolve_type_parameters(type_parameters, type_names, emit);
        let type_vars: Vec<Type<S>> = type_parameters
            .iter()
            .map(|v| Type::Var(Span::None, v.name.clone()))
            .collect();
        let then_type = then_type
            .clone()
            .substitute(
                type_names
                    .iter()
                    .map(|name| &name.name)
                    .zip(type_vars.iter())
                    .collect(),
            )
            .unwrap_or_else(|e| {
                emit(e);
                Type::Fail(span.clone())
            });
        let param_type = param_type
            .clone()
            .substitute(
                type_names
                    .iter()
                    .map(|name| &name.name)
                    .zip(type_vars.iter())
                    .collect(),
            )
            .unwrap_or_else(|e| {
                emit(e);
                Type::Fail(span.clone())
            });
        self.finish_check_command_receive(
            span,
            object,
            parameter,
            annotation,
            process,
            &type_parameters,
            param_type,
            then_type,
            mode,
            emit,
        )
    }

    fn check_command_receive_plain(
        &mut self,
        span: &Span,
        object: &LocalName,
        parameter: &LocalName,
        annotation: &Option<Type<S>>,
        process: &Arc<Process<(), S>>,
        type_names: &[TypeParameter],
        param_type: &Type<S>,
        then_type: &Type<S>,
        type_parameters: &[TypeParameter],
        mode: &ProcessAnalyzerMode,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Option<Type<S>>) {
        let type_parameters = self.resolve_type_parameters(type_parameters, type_names, emit);
        self.finish_check_command_receive(
            span,
            object,
            parameter,
            annotation,
            process,
            &type_parameters,
            param_type.clone(),
            then_type.clone(),
            mode,
            emit,
        )
    }

    fn finish_check_command_receive(
        &mut self,
        span: &Span,
        object: &LocalName,
        parameter: &LocalName,
        annotation: &Option<Type<S>>,
        process: &Arc<Process<(), S>>,
        type_parameters: &[TypeParameter],
        param_type: Type<S>,
        then_type: Type<S>,
        mode: &ProcessAnalyzerMode,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Option<Type<S>>) {
        self.type_defs.extend_vars(type_parameters.iter().cloned());

        if let Some(annotated_type) = annotation {
            if let Err(e) = self.type_defs.validate_type(annotated_type) {
                emit(e);
            }
            if let Err(e) = param_type.check_assignable(span, annotated_type, &self.type_defs) {
                emit(e);
            }
        }
        if let Err(e) = self.put(span, parameter.clone(), param_type.clone()) {
            emit(e);
        }
        if let Err(e) = self.put(span, object.clone(), then_type) {
            emit(e);
        }
        let (process, inferred_types) = self.analyze_process(process, mode, emit);
        (
            Command::Receive(
                parameter.clone(),
                annotation.clone(),
                param_type,
                process,
                type_parameters.to_vec(),
            ),
            inferred_types,
        )
    }

    fn check_command_signal(
        &mut self,
        span: &Span,
        object: &LocalName,
        typ: &Type<S>,
        chosen: &LocalName,
        process: &Arc<Process<(), S>>,
        mode: &ProcessAnalyzerMode,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Option<Type<S>>) {
        let Type::Choice(_, branches) = typ else {
            if !matches!(typ, Type::Fail(_)) {
                emit(TypeError::InvalidOperation(
                    span.clone(),
                    Operation::Signal,
                    typ.clone(),
                ));
            }
            let fail = Type::Fail(span.clone());
            self.put(span, object.clone(), fail.clone()).ok();
            let (process, inferred) = self.analyze_process(process, mode, emit);
            return (Command::Signal(chosen.clone(), process), inferred);
        };
        let Some(branch_type) = branches.get(chosen) else {
            if !matches!(typ, Type::Fail(_)) {
                emit(TypeError::InvalidBranch(
                    span.clone(),
                    chosen.clone(),
                    typ.clone(),
                ));
            }
            let fail = Type::Fail(span.clone());
            self.put(span, object.clone(), fail.clone()).ok();
            let (process, inferred) = self.analyze_process(process, mode, emit);
            return (Command::Signal(chosen.clone(), process), inferred);
        };
        if let Err(e) = self.put(span, object.clone(), branch_type.clone()) {
            emit(e);
        }
        let (process, inferred_types) = self.analyze_process(process, mode, emit);
        (Command::Signal(chosen.clone(), process), inferred_types)
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
    ) -> (Command<Type<S>, S>, Option<Type<S>>) {
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
                Command::Case(
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
            Command::Case(
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

    fn check_command_continue(
        &mut self,
        span: &Span,
        typ: &Type<S>,
        process: &Arc<Process<(), S>>,
        mode: &ProcessAnalyzerMode,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Option<Type<S>>) {
        let Type::Break(_) = typ else {
            if !matches!(typ, Type::Fail(_)) {
                emit(TypeError::InvalidOperation(
                    span.clone(),
                    Operation::Continue,
                    typ.clone(),
                ));
            }
            let (process, inferred) = self.analyze_process(process, mode, emit);
            return (Command::Continue(process), inferred);
        };
        let (process, inferred_types) = self.analyze_process(process, mode, emit);
        (Command::Continue(process), inferred_types)
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
    ) -> (Command<Type<S>, S>, Option<Type<S>>) {
        if let Some(inference_subject) = inference_subject {
            emit(TypeError::TypeMustBeKnownAtThisPoint(
                span.clone(),
                inference_subject.clone(),
            ));
            let fail = Type::Fail(span.clone());
            self.put(span, object.clone(), fail.clone()).ok();
            let (process, inferred) = self.analyze_process(process, mode, emit);
            return (
                Command::Begin {
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
                Command::Begin {
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
            Command::Begin {
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
    ) -> (Command<Type<S>, S>, Option<Type<S>>) {
        if !matches!(typ, Type::Recursive { .. }) {
            if !matches!(typ, Type::Fail(_)) {
                emit(TypeError::InvalidOperation(
                    span.clone(),
                    Operation::Loop,
                    typ.clone(),
                ));
            }
            return (
                Command::Loop(label.clone(), driver.clone(), captures.clone()),
                None,
            );
        }
        let Some((driver_type, variables)) = self.loop_points.get(label).cloned() else {
            emit(TypeError::NoSuchLoopPoint(span.clone(), label.clone()));
            return (Command::Break, None);
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
            Command::Loop(label.clone(), driver.clone(), captures.clone()),
            inferred_loop.or(Some(Type::Self_(span.clone(), label.clone()))),
        )
    }

    fn check_command_send_type(
        &mut self,
        span: &Span,
        object: &LocalName,
        typ: &Type<S>,
        argument: &Type<S>,
        process: &Arc<Process<(), S>>,
        mode: &ProcessAnalyzerMode,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Option<Type<S>>) {
        let Type::Forall(_, type_name, then_type) = typ else {
            if !matches!(typ, Type::Fail(_)) {
                emit(TypeError::InvalidOperation(
                    span.clone(),
                    Operation::SendType,
                    typ.clone(),
                ));
            }
            let fail = Type::Fail(span.clone());
            self.put(span, object.clone(), fail.clone()).ok();
            let (process, inferred) = self.analyze_process(process, mode, emit);
            return (Command::SendType(argument.clone(), process), inferred);
        };
        self.check_type_constraint(span, type_name, argument, emit);
        let then_type = then_type
            .clone()
            .substitute(BTreeMap::from([(&type_name.name, argument)]))
            .unwrap_or_else(|e| {
                emit(e);
                Type::Fail(span.clone())
            });
        if let Err(e) = self.put(span, object.clone(), then_type) {
            emit(e);
        }
        let (process, inferred_types) = self.analyze_process(process, mode, emit);
        (Command::SendType(argument.clone(), process), inferred_types)
    }

    fn check_command_receive_type(
        &mut self,
        span: &Span,
        object: &LocalName,
        typ: &Type<S>,
        parameter: &TypeParameter,
        process: &Arc<Process<(), S>>,
        mode: &ProcessAnalyzerMode,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Option<Type<S>>) {
        let Type::Exists(_, type_name, then_type) = typ else {
            if !matches!(typ, Type::Fail(_)) {
                emit(TypeError::InvalidOperation(
                    span.clone(),
                    Operation::ReceiveType,
                    typ.clone(),
                ));
            }
            let fail = Type::Fail(span.clone());
            self.put(span, object.clone(), fail.clone()).ok();
            let (process, inferred) = self.analyze_process(process, mode, emit);
            return (Command::ReceiveType(parameter.clone(), process), inferred);
        };
        let parameter = self.resolve_type_parameter(parameter, type_name, emit);
        let then_type = then_type
            .clone()
            .substitute(BTreeMap::from([(
                &type_name.name,
                &Type::Var(span.clone(), parameter.name.clone()),
            )]))
            .unwrap_or_else(|e| {
                emit(e);
                Type::Fail(span.clone())
            });
        self.type_defs.insert_var(parameter.clone());
        if let Err(e) = self.put(span, object.clone(), then_type) {
            emit(e);
        }
        let (process, inferred_types) = self.analyze_process(process, mode, emit);
        (
            Command::ReceiveType(parameter.clone(), process),
            inferred_types,
        )
    }

    pub(crate) fn infer_process(
        &mut self,
        process: &Process<(), S>,
        inference_subject: &LocalName,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Arc<Process<Type<S>, S>>, Type<S>) {
        match process {
            Process::Let {
                span,
                name,
                annotation,
                typ: (),
                value: expression,
                then: process,
            } => match annotation {
                Some(annotated_type) => self.infer_process_let_annotated(
                    span,
                    name,
                    annotation,
                    annotated_type,
                    expression,
                    process,
                    inference_subject,
                    emit,
                ),
                None => self.infer_process_let_inferred(
                    span,
                    name,
                    annotation,
                    expression,
                    process,
                    inference_subject,
                    emit,
                ),
            },

            Process::Do {
                span,
                name: object,
                usage,
                typ: (),
                command,
            } => self.infer_process_do(span, object, usage, command, inference_subject, emit),

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

            Process::Submit {
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

            Process::Unreachable(span) => self.infer_process_unreachable(span, emit),

            Process::Block(span, index, body, then) => {
                self.infer_process_block(span, *index, body, then, inference_subject, emit)
            }

            Process::Goto(span, index, caps) => self.infer_process_goto(span, *index, caps, emit),
        }
    }

    fn infer_process_let_annotated(
        &mut self,
        span: &Span,
        name: &LocalName,
        annotation: &Option<Type<S>>,
        annotated_type: &Type<S>,
        expression: &Arc<Expression<(), S>>,
        process: &Arc<Process<(), S>>,
        inference_subject: &LocalName,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Arc<Process<Type<S>, S>>, Type<S>) {
        let expression =
            self.check_expression(Some(inference_subject), expression, annotated_type, emit);
        self.finish_infer_process_let(
            span,
            name,
            annotation,
            annotated_type.clone(),
            expression,
            process,
            inference_subject,
            emit,
        )
    }

    fn infer_process_let_inferred(
        &mut self,
        span: &Span,
        name: &LocalName,
        annotation: &Option<Type<S>>,
        expression: &Arc<Expression<(), S>>,
        process: &Arc<Process<(), S>>,
        inference_subject: &LocalName,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Arc<Process<Type<S>, S>>, Type<S>) {
        if let Expression::Variable(variable_span, variable, (), usage) = expression.as_ref()
            && variable == inference_subject
            && matches!(usage, VariableUsage::Move)
        {
            let (process, typ) = self.infer_process(process, name, emit);
            return (
                Arc::new(Process::Let {
                    span: span.clone(),
                    name: name.clone(),
                    annotation: annotation.clone(),
                    typ: typ.clone(),
                    value: Arc::new(Expression::Variable(
                        variable_span.clone(),
                        variable.clone(),
                        typ.clone(),
                        usage.clone(),
                    )),
                    then: process,
                }),
                typ,
            );
        }
        let (expression, typ) = self.infer_expression(Some(inference_subject), expression, emit);
        self.finish_infer_process_let(
            span,
            name,
            annotation,
            typ,
            expression,
            process,
            inference_subject,
            emit,
        )
    }

    fn finish_infer_process_let(
        &mut self,
        span: &Span,
        name: &LocalName,
        annotation: &Option<Type<S>>,
        typ: Type<S>,
        expression: Arc<Expression<Type<S>, S>>,
        process: &Arc<Process<(), S>>,
        inference_subject: &LocalName,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Arc<Process<Type<S>, S>>, Type<S>) {
        if let Err(e) = self.put(span, name.clone(), typ.clone()) {
            emit(e);
        }
        let (process, subject_type) = self.infer_process(process, inference_subject, emit);
        (
            Arc::new(Process::Let {
                span: span.clone(),
                name: name.clone(),
                annotation: annotation.clone(),
                typ,
                value: expression,
                then: process,
            }),
            subject_type,
        )
    }

    fn infer_process_do(
        &mut self,
        span: &Span,
        object: &LocalName,
        usage: &VariableUsage,
        command: &Command<(), S>,
        inference_subject: &LocalName,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Arc<Process<Type<S>, S>>, Type<S>) {
        if object == inference_subject {
            let (command, typ) = self.infer_command(span, inference_subject, command, emit);
            return (
                Arc::new(Process::Do {
                    span: span.clone(),
                    name: object.clone(),
                    usage: usage.clone(),
                    typ: typ.clone(),
                    command,
                }),
                typ,
            );
        }
        let typ = self
            .get_variable_or_error(span, object)
            .unwrap_or_else(|e| {
                emit(e);
                Type::Fail(span.clone())
            });

        let (command, inferred_type) = self.check_command(
            Some(inference_subject),
            span,
            object,
            &typ,
            command,
            &ProcessAnalyzerMode::Infer(inference_subject.clone()),
            emit,
        );

        let Some(inferred_type) = inferred_type else {
            emit(TypeError::TypeMustBeKnownAtThisPoint(
                span.clone(),
                inference_subject.clone(),
            ));
            return (
                Arc::new(Process::Do {
                    span: span.clone(),
                    name: object.clone(),
                    usage: usage.clone(),
                    typ,
                    command,
                }),
                Type::Fail(span.clone()),
            );
        };

        (
            Arc::new(Process::Do {
                span: span.clone(),
                name: object.clone(),
                usage: usage.clone(),
                typ,
                command,
            }),
            inferred_type,
        )
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
    ) -> (Arc<Process<Type<S>, S>>, Type<S>) {
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
                            Arc::new(Process::Unreachable(span.clone())),
                            Type::Fail(span.clone()),
                        );
                    }
                };
            if poll_driver != *driver {
                emit(TypeError::RepollOutsidePoll(span.clone()));
                return (
                    Arc::new(Process::Unreachable(span.clone())),
                    Type::Fail(span.clone()),
                );
            }

            if self.get_variable(driver).is_none() {
                emit(TypeError::RepollOutsidePoll(span.clone()));
                return (
                    Arc::new(Process::Unreachable(span.clone())),
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
                    Arc::new(Process::Unreachable(span.clone())),
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
                    Arc::new(Process::Unreachable(span.clone())),
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
            Arc::new(Process::Poll {
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
            }),
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
    ) -> (Arc<Process<Type<S>, S>>, Type<S>) {
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
                        Arc::new(Process::Unreachable(span.clone())),
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
            Arc::new(Process::Submit {
                span: span.clone(),
                driver: driver.clone(),
                point: point.clone(),
                values: typed_values,
                captures: captures.clone(),
            }),
            Type::choice(vec![]),
        )
    }

    fn infer_process_unreachable(
        &mut self,
        span: &Span,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Arc<Process<Type<S>, S>>, Type<S>) {
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
        (
            Arc::new(Process::Unreachable(span.clone())),
            Type::choice(vec![]),
        )
    }

    fn infer_process_block(
        &mut self,
        span: &Span,
        index: usize,
        body: &Arc<Process<(), S>>,
        then: &Arc<Process<(), S>>,
        inference_subject: &LocalName,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Arc<Process<Type<S>, S>>, Type<S>) {
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
                Arc::new(Process::Block(
                    span.clone(),
                    index,
                    Arc::new(Process::Unreachable(span.clone())),
                    typed_then,
                )),
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
            Arc::new(Process::Block(span.clone(), index, typed_body, typed_then)),
            final_type,
        )
    }

    fn infer_process_goto(
        &mut self,
        span: &Span,
        index: usize,
        caps: &Captures,
        _emit: &mut impl FnMut(TypeError<S>),
    ) -> (Arc<Process<Type<S>, S>>, Type<S>) {
        let entry = self.blocks.get_mut(&index).unwrap();
        entry.paths.push(BlockPathContext {
            variables: self.variables.clone(),
            type_vars: self.type_defs.vars.clone(),
        });
        self.variables.clear();
        (
            Arc::new(Process::Goto(span.clone(), index, caps.clone())),
            Type::choice(vec![]),
        )
    }

    pub(crate) fn infer_command(
        &mut self,
        span: &Span,
        subject: &LocalName,
        command: &Command<(), S>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Type<S>) {
        match command {
            Command::Noop(process) => {
                let (process, typ) = self.infer_process(process, subject, emit);
                (Command::Noop(process), typ)
            }
            Command::Link(expression) => self.infer_command_link(span, subject, expression, emit),
            Command::Send(argument, process) => {
                self.infer_command_send(span, subject, argument, process, emit)
            }
            Command::Receive(parameter, annotation, (), process, vars) => self
                .infer_command_receive(span, subject, parameter, annotation, process, vars, emit),
            Command::Signal(chosen, process) => {
                self.infer_command_signal(span, subject, chosen, process, emit)
            }
            Command::Case(branches, processes, else_process) => {
                self.infer_command_case(span, subject, branches, processes, else_process, emit)
            }
            Command::Break => {
                if let Err(e) = self.cannot_have_obligations(span) {
                    emit(e);
                }
                (Command::Break, Type::Continue(span.clone()))
            }
            Command::Continue(process) => self.infer_command_continue(span, process, emit),
            Command::Begin { .. } => {
                emit(TypeError::TypeMustBeKnownAtThisPoint(
                    span.clone(),
                    subject.clone(),
                ));
                (Command::Break, Type::Fail(span.clone()))
            }
            Command::Loop(label, driver, captures) => {
                self.infer_command_loop(span, label, driver, captures, emit)
            }
            Command::SendType(_, _) => {
                emit(TypeError::TypeMustBeKnownAtThisPoint(
                    span.clone(),
                    subject.clone(),
                ));
                (Command::Break, Type::Fail(span.clone()))
            }
            Command::ReceiveType(parameter, process) => {
                self.infer_command_receive_type(span, subject, parameter, process, emit)
            }
        }
    }

    fn infer_command_link(
        &mut self,
        span: &Span,
        subject: &LocalName,
        expression: &Arc<Expression<(), S>>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Type<S>) {
        let (expression, typ) = self.infer_expression(Some(subject), expression, emit);
        if let Err(e) = self.cannot_have_obligations(span) {
            emit(e);
        }
        (Command::Link(expression), typ.dual(Span::None))
    }

    fn infer_command_send(
        &mut self,
        span: &Span,
        subject: &LocalName,
        argument: &Arc<Expression<(), S>>,
        process: &Arc<Process<(), S>>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Type<S>) {
        let (argument, arg_type) = self.infer_expression(Some(subject), argument, emit);
        let (process, then_type) = self.infer_process(process, subject, emit);
        (
            Command::Send(argument, process),
            Type::Function(
                span.clone(),
                Box::new(arg_type),
                Box::new(then_type),
                vec![],
            ),
        )
    }

    fn infer_command_receive(
        &mut self,
        span: &Span,
        subject: &LocalName,
        parameter: &LocalName,
        annotation: &Option<Type<S>>,
        process: &Arc<Process<(), S>>,
        vars: &[TypeParameter],
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Type<S>) {
        self.type_defs.extend_vars(vars.iter().cloned());
        let Some(param_type) = annotation else {
            emit(TypeError::ParameterTypeMustBeKnown(
                span.clone(),
                parameter.clone(),
            ));
            let fail = Type::Fail(span.clone());
            if let Err(e) = self.put(span, parameter.clone(), fail.clone()) {
                emit(e);
            }
            let (process, _then_type) = self.infer_process(process, subject, emit);
            return (
                Command::Receive(
                    parameter.clone(),
                    annotation.clone(),
                    fail,
                    process,
                    vars.to_vec(),
                ),
                Type::Fail(span.clone()),
            );
        };
        if let Err(e) = self.put(span, parameter.clone(), param_type.clone()) {
            emit(e);
        }
        let (process, then_type) = self.infer_process(process, subject, emit);
        (
            Command::Receive(
                parameter.clone(),
                annotation.clone(),
                param_type.clone(),
                process,
                vars.to_vec(),
            ),
            Type::Pair(
                span.clone(),
                Box::new(param_type.clone()),
                Box::new(then_type),
                vars.to_vec(),
            ),
        )
    }

    fn infer_command_signal(
        &mut self,
        span: &Span,
        subject: &LocalName,
        chosen: &LocalName,
        process: &Arc<Process<(), S>>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Type<S>) {
        let (process, then_type) = self.infer_process(process, subject, emit);
        (
            Command::Signal(chosen.clone(), process),
            Type::Choice(span.clone(), BTreeMap::from([(chosen.clone(), then_type)])),
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
    ) -> (Command<Type<S>, S>, Type<S>) {
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
                Command::Case(Arc::clone(branches), Box::from(typed_processes), typed_else),
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
            Command::Case(Arc::clone(branches), Box::from(typed_processes), None),
            Type::Either(span.clone(), branch_types),
        )
    }

    fn infer_command_continue(
        &mut self,
        span: &Span,
        process: &Arc<Process<(), S>>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Type<S>) {
        let process = self.check_process(process, emit);
        (Command::Continue(process), Type::Break(span.clone()))
    }

    fn infer_command_loop(
        &mut self,
        span: &Span,
        label: &Option<LocalName>,
        driver: &LocalName,
        captures: &Captures,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Type<S>) {
        let Some((driver_type, variables)) = self.loop_points.get(label).cloned() else {
            emit(TypeError::NoSuchLoopPoint(span.clone(), label.clone()));
            return (Command::Break, Type::Fail(span.clone()));
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
            Command::Loop(label.clone(), driver.clone(), captures.clone()),
            driver_type,
        )
    }

    fn infer_command_receive_type(
        &mut self,
        span: &Span,
        subject: &LocalName,
        parameter: &TypeParameter,
        process: &Arc<Process<(), S>>,
        emit: &mut impl FnMut(TypeError<S>),
    ) -> (Command<Type<S>, S>, Type<S>) {
        self.type_defs.insert_var(parameter.clone());
        let (process, then_type) = self.infer_process(process, subject, emit);
        (
            Command::ReceiveType(parameter.clone(), process),
            Type::Exists(span.clone(), parameter.clone(), Box::new(then_type)),
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
