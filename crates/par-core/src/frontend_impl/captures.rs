use super::{
    language::{LocalName, Unresolved},
    process::{Command, Expression, Process, Step, TerminalCommand, Terminator},
};
use crate::location::Span;
use indexmap::IndexMap;
use std::{collections::VecDeque, sync::Arc};

#[derive(Clone, Debug)]
pub enum VariableUsage {
    Unknown,
    Copy,
    Move,
}

#[derive(Clone, Debug)]
pub struct Captures {
    pub names: IndexMap<LocalName, (Span, VariableUsage)>,
}

impl Default for Captures {
    fn default() -> Self {
        Self {
            names: IndexMap::new(),
        }
    }
}

impl Captures {
    pub fn new() -> Self {
        Self {
            names: IndexMap::new(),
        }
    }

    pub fn single(name: LocalName, span: Span, usage: VariableUsage) -> Self {
        let mut caps = Self::new();
        caps.add(name, span, usage);
        caps
    }

    pub fn extend(&mut self, other: Self) {
        for (name, span) in other.names {
            self.names.insert(name, span);
        }
    }

    pub fn add(&mut self, name: LocalName, span: Span, usage: VariableUsage) {
        self.names.insert(name, (span, usage));
    }

    pub fn remove(&mut self, name: &LocalName) -> Option<(Span, VariableUsage)> {
        self.names.shift_remove(name)
    }

    pub fn contains(&self, name: &LocalName) -> bool {
        self.names.contains_key(name)
    }

    fn merge_missing(&mut self, other: &Captures) -> bool {
        let mut changed = false;
        for (name, (span, usage)) in &other.names {
            if !self.names.contains_key(name) {
                self.names
                    .insert(name.clone(), (span.clone(), usage.clone()));
                changed = true;
            }
        }
        changed
    }
}

type BeginId = Span;

#[derive(Clone, Debug, Default)]
struct LoopEnv {
    labels: IndexMap<Option<LocalName>, BeginId>,
}

impl LoopEnv {
    fn with_begin(&self, label: &Option<LocalName>, id: BeginId) -> Self {
        let mut env = self.clone();
        env.labels.insert(label.clone(), id);
        env
    }

    fn resolve(&self, label: &Option<LocalName>) -> Option<BeginId> {
        self.labels.get(label).cloned()
    }

    fn intersect_in_place(&mut self, other: &LoopEnv) -> bool {
        let mut changed = false;
        self.labels.retain(|label, id| {
            let keep = other.labels.get(label) == Some(id);
            if !keep {
                changed = true;
            }
            keep
        });
        changed
    }
}

#[derive(Clone, Debug)]
struct CaptureAnalysis {
    begin_drivers: IndexMap<BeginId, LocalName>,
    block_envs: IndexMap<usize, LoopEnv>,
    begin_caps: IndexMap<BeginId, Captures>,
    block_caps: IndexMap<usize, Captures>,
    poll_caps: IndexMap<LocalName, Captures>,
}

impl CaptureAnalysis {
    fn from_process(process: &Process<(), Unresolved>) -> Self {
        let (block_envs, begin_drivers) = BlockEnvAnalyzer::analyze_process(process);
        let (begin_caps, block_caps, poll_caps) =
            compute_captures_from_process(process, &block_envs);
        CaptureAnalysis {
            begin_drivers,
            block_envs,
            begin_caps,
            block_caps,
            poll_caps,
        }
    }

    fn from_expression(expression: &Expression<(), Unresolved>) -> Self {
        let (block_envs, begin_drivers) = BlockEnvAnalyzer::analyze_expression(expression);
        let (begin_caps, block_caps, poll_caps) =
            compute_captures_from_expression(expression, &block_envs);
        CaptureAnalysis {
            begin_drivers,
            block_envs,
            begin_caps,
            block_caps,
            poll_caps,
        }
    }

    fn fix_process(
        &self,
        process: &Process<(), Unresolved>,
        env: &LoopEnv,
    ) -> (Arc<Process<(), Unresolved>>, Captures) {
        let (terminator, mut caps) = self.fix_terminator(&process.terminator, env);
        let mut steps = Vec::with_capacity(process.steps.len());

        for step in process.steps.iter().rev() {
            match step {
                Step::Let {
                    span,
                    name,
                    annotation,
                    typ,
                    value,
                } => {
                    caps.remove(name);
                    let (value, value_caps) = self.fix_expression(value, env, &caps);
                    caps.extend(value_caps);
                    steps.push(Step::Let {
                        span: span.clone(),
                        name: name.clone(),
                        annotation: annotation.clone(),
                        typ: *typ,
                        value,
                    });
                }
                Step::Do {
                    span,
                    name,
                    typ,
                    command,
                    ..
                } => {
                    let (command, mut command_caps) = self.fix_command(command, env, caps);
                    let usage = if command_caps.contains(name) {
                        VariableUsage::Copy
                    } else {
                        VariableUsage::Move
                    };
                    command_caps.add(name.clone(), span.clone(), VariableUsage::Unknown);
                    caps = command_caps;
                    steps.push(Step::Do {
                        span: span.clone(),
                        name: name.clone(),
                        usage,
                        typ: *typ,
                        command,
                    });
                }
            }
        }
        steps.reverse();
        (Arc::new(Process::new(steps, terminator)), caps)
    }

    fn fix_terminator(
        &self,
        terminator: &Terminator<(), Unresolved>,
        env: &LoopEnv,
    ) -> (Terminator<(), Unresolved>, Captures) {
        match terminator {
            Terminator::Do {
                span,
                name,
                typ,
                command,
                ..
            } => {
                let (command, mut caps) = self.fix_terminal_command(command, span, env);
                let usage = if caps.contains(name) {
                    VariableUsage::Copy
                } else {
                    VariableUsage::Move
                };
                caps.add(name.clone(), span.clone(), VariableUsage::Unknown);
                (
                    Terminator::Do {
                        span: span.clone(),
                        name: name.clone(),
                        usage,
                        typ: *typ,
                        command,
                    },
                    caps,
                )
            }
            Terminator::Poll {
                span,
                kind,
                driver,
                point,
                clients,
                name,
                name_typ,
                captures: _,
                then,
                else_,
            } => {
                let (then, mut then_caps) = self.fix_process(then, env);
                then_caps.remove(name);
                then_caps.remove(driver);

                let (else_, mut else_caps) = self.fix_process(else_, env);
                else_caps.remove(driver);

                let mut later_caps = then_caps.clone();
                later_caps.extend(else_caps.clone());

                let mut fixed_clients = Vec::with_capacity(clients.len());
                let mut caps = then_caps;
                caps.extend(else_caps);
                for client in clients {
                    let (client, caps1) = self.fix_expression(client, env, &later_caps);
                    fixed_clients.push(client);
                    caps.extend(caps1);
                }

                let poll_caps = self.poll_caps.get(point).cloned().unwrap_or_default();
                (
                    Terminator::Poll {
                        span: span.clone(),
                        kind: kind.clone(),
                        driver: driver.clone(),
                        point: point.clone(),
                        clients: fixed_clients,
                        name: name.clone(),
                        name_typ: name_typ.clone(),
                        captures: poll_caps,
                        then,
                        else_,
                    },
                    caps,
                )
            }
            Terminator::Submit {
                span,
                driver,
                point,
                values,
                captures: _,
            } => {
                let poll_caps = self.poll_caps.get(point).cloned().unwrap_or_default();
                let mut caps = poll_caps.clone();
                caps.add(driver.clone(), span.clone(), VariableUsage::Unknown);

                let mut fixed_values = Vec::with_capacity(values.len());
                for value in values {
                    let (value, caps1) = self.fix_expression(value, env, &poll_caps);
                    fixed_values.push(value);
                    caps.extend(caps1);
                }

                (
                    Terminator::Submit {
                        span: span.clone(),
                        driver: driver.clone(),
                        point: point.clone(),
                        values: fixed_values,
                        captures: poll_caps,
                    },
                    caps,
                )
            }
            Terminator::Block(span, index, body, process) => {
                let (process, caps) = self.fix_process(process, env);
                let body_env = self.block_envs.get(index).cloned().unwrap_or_default();
                let (body, _body_caps) = self.fix_process(body, &body_env);
                (Terminator::Block(span.clone(), *index, body, process), caps)
            }
            Terminator::Goto(span, index, _) => {
                let caps = self.block_caps.get(index).cloned().unwrap_or_default();
                (Terminator::Goto(span.clone(), *index, caps.clone()), caps)
            }
            Terminator::Unreachable(span) => {
                (Terminator::Unreachable(span.clone()), Captures::new())
            }
            Terminator::ToDo(span) => (Terminator::ToDo(span.clone()), Captures::new()),
        }
    }

    fn fix_command(
        &self,
        command: &Command<(), Unresolved>,
        env: &LoopEnv,
        mut caps: Captures,
    ) -> (Command<(), Unresolved>, Captures) {
        match command {
            Command::Noop => (Command::Noop, caps),
            Command::Send(argument) => {
                let (argument, caps1) = self.fix_expression(argument, env, &caps);
                caps.extend(caps1);
                (Command::Send(argument), caps)
            }
            Command::Receive(parameter, annotation, typ, vars) => {
                caps.remove(parameter);
                (
                    Command::Receive(parameter.clone(), annotation.clone(), *typ, vars.clone()),
                    caps,
                )
            }
            Command::Signal(chosen) => (Command::Signal(chosen.clone()), caps),
            Command::Continue => (Command::Continue, caps),
            Command::SendType(argument) => (Command::SendType(argument.clone()), caps),
            Command::ReceiveType(parameter) => (Command::ReceiveType(parameter.clone()), caps),
        }
    }

    fn fix_terminal_command(
        &self,
        command: &TerminalCommand<(), Unresolved>,
        span: &Span,
        env: &LoopEnv,
    ) -> (TerminalCommand<(), Unresolved>, Captures) {
        match command {
            TerminalCommand::Link(expression) => {
                let (expression, caps) = self.fix_expression(expression, env, &Captures::new());
                (TerminalCommand::Link(expression), caps)
            }
            TerminalCommand::Case(branches, processes, else_process) => {
                let mut fixed_processes = Vec::new();
                let mut caps = Captures::new();
                for process in processes {
                    let (process, caps1) = self.fix_process(process, env);
                    fixed_processes.push(process);
                    caps.extend(caps1);
                }
                let fixed_else = else_process.clone().map(|process| {
                    let (process, caps1) = self.fix_process(&process, env);
                    caps.extend(caps1);
                    process
                });
                (
                    TerminalCommand::Case(
                        branches.clone(),
                        fixed_processes.into_boxed_slice(),
                        fixed_else,
                    ),
                    caps,
                )
            }
            TerminalCommand::Break => (TerminalCommand::Break, Captures::new()),
            TerminalCommand::Begin {
                unfounded,
                label,
                captures: _,
                body,
            } => {
                let begin_id = span.clone();
                let env = env.with_begin(label, begin_id.clone());
                let (process, caps) = self.fix_process(body, &env);
                let loop_caps = self.begin_caps.get(&begin_id).cloned().unwrap_or_default();
                (
                    TerminalCommand::Begin {
                        unfounded: *unfounded,
                        label: label.clone(),
                        captures: loop_caps,
                        body: process,
                    },
                    caps,
                )
            }
            TerminalCommand::Loop(label, _, _) => match env.resolve(label) {
                Some(begin_id) => {
                    let driver = self
                        .begin_drivers
                        .get(&begin_id)
                        .cloned()
                        .unwrap_or_else(LocalName::invalid);
                    let loop_caps = self.begin_caps.get(&begin_id).cloned().unwrap_or_default();
                    (
                        TerminalCommand::Loop(label.clone(), driver, loop_caps.clone()),
                        loop_caps,
                    )
                }
                _ => (
                    TerminalCommand::Loop(label.clone(), LocalName::invalid(), Captures::new()),
                    Captures::new(),
                ),
            },
        }
    }

    fn fix_expression(
        &self,
        expression: &Expression<(), Unresolved>,
        env: &LoopEnv,
        later_captures: &Captures,
    ) -> (Arc<Expression<(), Unresolved>>, Captures) {
        match expression {
            Expression::Global(span, name, typ) => (
                Arc::new(Expression::Global(span.clone(), name.clone(), typ.clone())),
                Captures::new(),
            ),
            Expression::Variable(span, name, typ, _usage) => {
                let usage = if later_captures.contains(name) {
                    VariableUsage::Copy
                } else {
                    VariableUsage::Move
                };
                (
                    Arc::new(Expression::Variable(
                        span.clone(),
                        name.clone(),
                        typ.clone(),
                        usage,
                    )),
                    Captures::single(name.clone(), span.clone(), VariableUsage::Unknown),
                )
            }
            Expression::Box(span, _caps, expression, typ) => {
                let (expression, mut caps) = self.fix_expression(expression, env, later_captures);
                for (name, (_span, usage)) in caps.names.iter_mut() {
                    if later_captures.contains(name) {
                        *usage = VariableUsage::Copy;
                    } else {
                        *usage = VariableUsage::Move;
                    }
                }
                (
                    Arc::new(Expression::Box(
                        span.clone(),
                        caps.clone(),
                        expression,
                        typ.clone(),
                    )),
                    caps,
                )
            }
            Expression::Chan {
                span,
                chan_name,
                chan_annotation,
                chan_type,
                expr_type,
                process,
                ..
            } => {
                let (process, mut caps) = self.fix_process(process, env);
                caps.remove(chan_name);
                for (name, (_span, usage)) in caps.names.iter_mut() {
                    if later_captures.contains(name) {
                        *usage = VariableUsage::Copy;
                    } else {
                        *usage = VariableUsage::Move;
                    }
                }
                (
                    Arc::new(Expression::Chan {
                        span: span.clone(),
                        captures: caps.clone(),
                        chan_name: chan_name.clone(),
                        chan_annotation: chan_annotation.clone(),
                        chan_type: chan_type.clone(),
                        expr_type: expr_type.clone(),
                        process,
                    }),
                    caps,
                )
            }
            Expression::Primitive(span, value, typ) => (
                Arc::new(Expression::Primitive(
                    span.clone(),
                    value.clone(),
                    typ.clone(),
                )),
                Captures::new(),
            ),
            Expression::External(f, typ) => (
                Arc::new(Expression::External(f.clone(), typ.clone())),
                Captures::new(),
            ),
            Expression::ToDo(span, typ) => (
                Arc::new(Expression::ToDo(span.clone(), typ.clone())),
                Captures::new(),
            ),
        }
    }
}

struct BlockEnvAnalyzer {
    blocks: IndexMap<usize, Arc<Process<(), Unresolved>>>,
    begin_drivers: IndexMap<BeginId, LocalName>,
    block_envs: IndexMap<usize, LoopEnv>,
    queue: VecDeque<(Arc<Process<(), Unresolved>>, LoopEnv)>,
}

impl BlockEnvAnalyzer {
    fn new() -> Self {
        Self {
            blocks: IndexMap::new(),
            begin_drivers: IndexMap::new(),
            block_envs: IndexMap::new(),
            queue: VecDeque::new(),
        }
    }

    fn analyze_process(
        process: &Process<(), Unresolved>,
    ) -> (IndexMap<usize, LoopEnv>, IndexMap<BeginId, LocalName>) {
        let mut analyzer = Self::new();
        analyzer.visit_process(process, &LoopEnv::default());
        analyzer.run();
        (analyzer.block_envs, analyzer.begin_drivers)
    }

    fn analyze_expression(
        expression: &Expression<(), Unresolved>,
    ) -> (IndexMap<usize, LoopEnv>, IndexMap<BeginId, LocalName>) {
        let mut analyzer = Self::new();
        analyzer.visit_expression(expression, &LoopEnv::default());
        analyzer.run();
        (analyzer.block_envs, analyzer.begin_drivers)
    }

    fn run(&mut self) {
        while let Some((process, env)) = self.queue.pop_front() {
            self.visit_process(&process, &env);
        }
    }

    fn visit_expression(&mut self, expression: &Expression<(), Unresolved>, env: &LoopEnv) {
        match expression {
            Expression::Box(_, _, expr, _) => self.visit_expression(expr, env),
            Expression::Chan { process, .. } => self.visit_process(process, env),
            Expression::Global(_, _, _)
            | Expression::Variable(_, _, _, _)
            | Expression::Primitive(_, _, _)
            | Expression::External(_, _)
            | Expression::ToDo(_, _) => {}
        }
    }

    fn visit_process(&mut self, process: &Process<(), Unresolved>, env: &LoopEnv) {
        for step in &process.steps {
            match step {
                Step::Let { value, .. } => self.visit_expression(value, env),
                Step::Do { command, .. } => self.visit_command(command, env),
            }
        }

        match &process.terminator {
            Terminator::Do {
                span,
                name,
                command,
                ..
            } => self.visit_terminal_command(command, span, name, env),
            Terminator::Poll {
                clients,
                then,
                else_,
                ..
            } => {
                for client in clients {
                    self.visit_expression(client, env);
                }
                self.visit_process(then, env);
                self.visit_process(else_, env);
            }
            Terminator::Submit { values, .. } => {
                for value in values {
                    self.visit_expression(value, env);
                }
            }
            Terminator::Block(_, index, body, process) => {
                self.blocks
                    .entry(*index)
                    .or_insert_with(|| Arc::clone(body));
                self.visit_process(process, env);
            }
            Terminator::Goto(_, index, _) => {
                self.schedule_block(*index, env);
            }
            Terminator::Unreachable(_) | Terminator::ToDo(_) => {}
        }
    }

    fn visit_command(&mut self, command: &Command<(), Unresolved>, env: &LoopEnv) {
        match command {
            Command::Send(argument) => self.visit_expression(argument, env),
            Command::Noop
            | Command::Receive(..)
            | Command::Signal(_)
            | Command::Continue
            | Command::SendType(_)
            | Command::ReceiveType(_) => {}
        }
    }

    fn visit_terminal_command(
        &mut self,
        command: &TerminalCommand<(), Unresolved>,
        span: &Span,
        subject: &LocalName,
        env: &LoopEnv,
    ) {
        match command {
            TerminalCommand::Link(expression) => self.visit_expression(expression, env),
            TerminalCommand::Case(_, processes, else_process) => {
                for process in processes {
                    self.visit_process(process, env);
                }
                if let Some(process) = else_process {
                    self.visit_process(process, env);
                }
            }
            TerminalCommand::Break | TerminalCommand::Loop(..) => {}
            TerminalCommand::Begin { label, body, .. } => {
                let begin_id = span.clone();
                self.begin_drivers
                    .entry(begin_id.clone())
                    .or_insert_with(|| subject.clone());
                let env = env.with_begin(label, begin_id);
                self.visit_process(body, &env);
            }
        }
    }

    fn schedule_block(&mut self, index: usize, env: &LoopEnv) {
        let changed = match self.block_envs.get_mut(&index) {
            None => {
                self.block_envs.insert(index, env.clone());
                true
            }
            Some(existing) => existing.intersect_in_place(env),
        };
        if changed {
            if let Some(body) = self.blocks.get(&index) {
                let env = self.block_envs.get(&index).cloned().unwrap_or_default();
                self.queue.push_back((Arc::clone(body), env));
            }
        }
    }
}

fn compute_captures_from_process(
    process: &Process<(), Unresolved>,
    block_envs: &IndexMap<usize, LoopEnv>,
) -> (
    IndexMap<BeginId, Captures>,
    IndexMap<usize, Captures>,
    IndexMap<LocalName, Captures>,
) {
    compute_captures(
        |collector| {
            collector.process_captures(process, &LoopEnv::default());
        },
        block_envs,
    )
}

fn compute_captures_from_expression(
    expression: &Expression<(), Unresolved>,
    block_envs: &IndexMap<usize, LoopEnv>,
) -> (
    IndexMap<BeginId, Captures>,
    IndexMap<usize, Captures>,
    IndexMap<LocalName, Captures>,
) {
    compute_captures(
        |collector| {
            collector.expression_captures(expression, &LoopEnv::default());
        },
        block_envs,
    )
}

fn compute_captures(
    f: impl Fn(&mut CaptureCollector<'_>),
    block_envs: &IndexMap<usize, LoopEnv>,
) -> (
    IndexMap<BeginId, Captures>,
    IndexMap<usize, Captures>,
    IndexMap<LocalName, Captures>,
) {
    let mut begin_caps = IndexMap::new();
    let mut block_caps = IndexMap::new();
    let mut poll_caps = IndexMap::new();
    loop {
        let mut next_begin_caps = begin_caps.clone();
        let mut next_block_caps = block_caps.clone();
        let mut next_poll_caps = poll_caps.clone();
        let mut changed = false;
        {
            let mut collector = CaptureCollector {
                block_envs,
                old_begin_caps: &begin_caps,
                old_block_caps: &block_caps,
                old_poll_caps: &poll_caps,
                next_begin_caps: &mut next_begin_caps,
                next_block_caps: &mut next_block_caps,
                next_poll_caps: &mut next_poll_caps,
                changed: &mut changed,
            };
            f(&mut collector);
        }
        if !changed {
            return (next_begin_caps, next_block_caps, next_poll_caps);
        }
        begin_caps = next_begin_caps;
        block_caps = next_block_caps;
        poll_caps = next_poll_caps;
    }
}

struct CaptureCollector<'a> {
    block_envs: &'a IndexMap<usize, LoopEnv>,
    old_begin_caps: &'a IndexMap<BeginId, Captures>,
    old_block_caps: &'a IndexMap<usize, Captures>,
    old_poll_caps: &'a IndexMap<LocalName, Captures>,
    next_begin_caps: &'a mut IndexMap<BeginId, Captures>,
    next_block_caps: &'a mut IndexMap<usize, Captures>,
    next_poll_caps: &'a mut IndexMap<LocalName, Captures>,
    changed: &'a mut bool,
}

impl<'a> CaptureCollector<'a> {
    fn expression_captures(
        &mut self,
        expression: &Expression<(), Unresolved>,
        env: &LoopEnv,
    ) -> Captures {
        match expression {
            Expression::Global(_, _, _) => Captures::new(),
            Expression::Variable(span, name, _, _) => {
                Captures::single(name.clone(), span.clone(), VariableUsage::Unknown)
            }
            Expression::Box(_, _, expression, _) => self.expression_captures(expression, env),
            Expression::Chan {
                chan_name, process, ..
            } => {
                let mut caps = self.process_captures(process, env);
                caps.remove(chan_name);
                caps
            }
            Expression::Primitive(_, _, _) => Captures::new(),
            Expression::External(_, _) => Captures::new(),
            Expression::ToDo(_, _) => Captures::new(),
        }
    }

    fn process_captures(&mut self, process: &Process<(), Unresolved>, env: &LoopEnv) -> Captures {
        let mut caps = self.terminator_captures(&process.terminator, env);
        for step in process.steps.iter().rev() {
            match step {
                Step::Let { name, value, .. } => {
                    caps.remove(name);
                    let expr_caps = self.expression_captures(value, env);
                    caps.merge_missing(&expr_caps);
                }
                Step::Do {
                    span,
                    name,
                    command,
                    ..
                } => {
                    caps = self.command_captures(command, env, caps);
                    caps.add(name.clone(), span.clone(), VariableUsage::Unknown);
                }
            }
        }
        caps
    }

    fn terminator_captures(
        &mut self,
        terminator: &Terminator<(), Unresolved>,
        env: &LoopEnv,
    ) -> Captures {
        match terminator {
            Terminator::Do {
                span,
                name,
                command,
                ..
            } => {
                let mut caps = self.terminal_command_captures(command, span, name, env);
                caps.add(name.clone(), span.clone(), VariableUsage::Unknown);
                caps
            }
            Terminator::Poll {
                driver,
                point,
                clients,
                name,
                then,
                else_,
                ..
            } => {
                let mut poll_caps = self.process_captures(then, env);
                poll_caps.remove(name);
                poll_caps.remove(driver);
                let mut else_caps = self.process_captures(else_, env);
                else_caps.remove(driver);
                poll_caps.merge_missing(&else_caps);
                self.update_poll_caps(point, &poll_caps);

                for client in clients {
                    let client_caps = self.expression_captures(client, env);
                    poll_caps.merge_missing(&client_caps);
                }

                poll_caps
            }
            Terminator::Submit {
                span,
                driver,
                point,
                values,
                ..
            } => {
                let mut caps = self.old_poll_caps.get(point).cloned().unwrap_or_default();
                caps.add(driver.clone(), span.clone(), VariableUsage::Unknown);
                for value in values {
                    let value_caps = self.expression_captures(value, env);
                    caps.merge_missing(&value_caps);
                }
                caps
            }
            Terminator::Block(_span, index, body, process) => {
                let body_env = self.block_envs.get(index).cloned().unwrap_or_default();
                let body_caps = self.process_captures(body, &body_env);
                self.update_block_caps(*index, &body_caps);
                self.process_captures(process, env)
            }
            Terminator::Goto(_, index, _) => {
                self.old_block_caps.get(index).cloned().unwrap_or_default()
            }
            Terminator::Unreachable(_) | Terminator::ToDo(_) => Captures::new(),
        }
    }

    fn command_captures(
        &mut self,
        command: &Command<(), Unresolved>,
        env: &LoopEnv,
        mut caps: Captures,
    ) -> Captures {
        match command {
            Command::Noop
            | Command::Signal(_)
            | Command::Continue
            | Command::SendType(_)
            | Command::ReceiveType(_) => caps,
            Command::Send(argument) => {
                let arg_caps = self.expression_captures(argument, env);
                caps.merge_missing(&arg_caps);
                caps
            }
            Command::Receive(parameter, _annotation, _typ, _vars) => {
                caps.remove(parameter);
                caps
            }
        }
    }

    fn terminal_command_captures(
        &mut self,
        command: &TerminalCommand<(), Unresolved>,
        span: &Span,
        subject: &LocalName,
        env: &LoopEnv,
    ) -> Captures {
        match command {
            TerminalCommand::Link(expression) => self.expression_captures(expression, env),
            TerminalCommand::Case(_, processes, else_process) => {
                let mut caps = Captures::new();
                for process in processes {
                    let branch_caps = self.process_captures(process, env);
                    caps.merge_missing(&branch_caps);
                }
                if let Some(process) = else_process {
                    let else_caps = self.process_captures(process, env);
                    caps.merge_missing(&else_caps);
                }
                caps
            }
            TerminalCommand::Break => Captures::new(),
            TerminalCommand::Begin { label, body, .. } => {
                let begin_id = span.clone();
                let env = env.with_begin(label, begin_id.clone());
                let body_caps = self.process_captures(body, &env);
                let mut loop_caps = body_caps.clone();
                loop_caps.remove(subject);
                self.update_begin_caps(begin_id, &loop_caps);
                body_caps
            }
            TerminalCommand::Loop(label, _, _) => env
                .resolve(label)
                .and_then(|id| self.old_begin_caps.get(&id).cloned())
                .unwrap_or_default(),
        }
    }

    fn update_begin_caps(&mut self, id: BeginId, caps: &Captures) {
        let entry = self.next_begin_caps.entry(id).or_insert_with(Captures::new);
        if entry.merge_missing(caps) {
            *self.changed = true;
        }
    }

    fn update_block_caps(&mut self, index: usize, caps: &Captures) {
        let entry = self
            .next_block_caps
            .entry(index)
            .or_insert_with(Captures::new);
        if entry.merge_missing(caps) {
            *self.changed = true;
        }
    }

    fn update_poll_caps(&mut self, point: &LocalName, caps: &Captures) {
        let entry = self
            .next_poll_caps
            .entry(point.clone())
            .or_insert_with(Captures::new);
        if entry.merge_missing(caps) {
            *self.changed = true;
        }
    }
}

impl Process<(), Unresolved> {
    pub fn fix_captures(&self) -> (Arc<Self>, Captures) {
        let analysis = CaptureAnalysis::from_process(self);
        analysis.fix_process(self, &LoopEnv::default())
    }
}

impl Expression<(), Unresolved> {
    pub fn fix_captures(&self) -> (Arc<Self>, Captures) {
        let analysis = CaptureAnalysis::from_expression(self);
        analysis.fix_expression(self, &LoopEnv::default(), &Captures::new())
    }
}
