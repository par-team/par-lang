use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt::{Debug, Display},
    sync::Arc,
};

use crate::frontend_impl::process::VariableUsage;
use crate::frontend_impl::program::DefinitionBody;
use crate::frontend_impl::types::core::get_primitive_type;
use crate::frontend_impl::{
    language::{GlobalName, LocalName, TypeParameter, Universal},
    process::{
        Captures, Command, Expression, PollKind, Process, Step, TerminalCommand, Terminator,
    },
    types::Type,
};
use crate::runtime_impl::tree::net::{Net, Tree};
use crate::{
    frontend_impl::{
        program::{CheckedModule, Definition},
        types::TypeDefs,
    },
    location::{Span, Spanning},
};
use arcstr::ArcStr;
use indexmap::{IndexMap, IndexSet};
use par_runtime::fan_behavior::FanBehavior;
use par_runtime::linker::Unlinked;
use par_runtime::poll::POLL_TOKEN;
use std::hash::Hash;

#[derive(Clone, Debug)]
pub enum Error {
    /// Error that is emitted when a variable that was never bound/captured is used
    UnboundVar(Span, #[allow(unused)] Var),
    /// Error that is emitted when it is unclear how a variable is used (move/copy)
    UnknownVariableUsage(Span),
    GlobalNotFound(GlobalName<Universal>),
    DependencyCycle {
        global: GlobalName<Universal>,
        dependents: IndexSet<GlobalName<Universal>>,
    },
    UnguardedLoop(Span, #[allow(unused)] Option<LocalName>),
}

impl Error {
    pub fn display(&self, _code: &str) -> String {
        "An INet compilation error was encountered. This is a BUG.\n\
        Please file an issue along with the code at\n\
        https://github.com/par-team/par-lang/issues/new"
            .to_string()
        //TODO: fix error messages
        /*match self {
            Error::UnboundVar(loc) => format!("Unbound variable\n{}", loc.display(code)),
            Error::UnusedVar(loc) => format!("Unused variable\n{}", loc.display(code)),
            Error::UnexpectedType(loc, ty) => {
                format!("Unexpected type: {:?}\n{}", ty, loc.display(code),)
            }
            Error::GlobalNotFound(name) => format!("Global not found: {:?}", name),
            Error::DependencyCycle { global, dependents } => format!(
                "Dependency cycle detected for global {:?} with dependents {:?}",
                global, dependents
            ),
            Error::UnguardedLoop(loc, name) => format!(
                "Unguarded loop with label {:?} at\n{}",
                name,
                loc.display(code)
            ),
        }*/
    }

    pub fn spans(&self) -> (Span, Vec<Span>) {
        match self {
            Error::UnboundVar(span, _)
            | Error::UnknownVariableUsage(span)
            | Error::UnguardedLoop(span, _) => (span.clone(), vec![]),

            Error::GlobalNotFound(name) => (name.span(), vec![]),

            Error::DependencyCycle { global, dependents } => (
                global.span(),
                dependents.iter().map(|name| name.span()).collect(),
            ),
        }
    }
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub(crate) struct TypedTree {
    pub tree: Tree<Unlinked>,
    pub ty: Type<Universal>,
}

impl Default for TypedTree {
    fn default() -> Self {
        Self {
            tree: Tree::Era,
            ty: Type::Break(Span::default()),
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum Var {
    Name(LocalName),
    Loop(Option<LocalName>),
}

impl From<LocalName> for Var {
    fn from(value: LocalName) -> Self {
        Var::Name(value)
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct LoopLabel(Option<LocalName>);

#[derive(Debug)]
pub(crate) struct Context {
    vars: BTreeMap<Var, TypedTree>,
    loop_points: BTreeMap<LoopLabel, BTreeSet<LoopLabel>>,
    unguarded_loop_labels: Vec<LoopLabel>,
}

pub(crate) struct PackData {
    names: Vec<Var>,
    types: Vec<Type<Universal>>,
    loop_points: BTreeMap<LoopLabel, BTreeSet<LoopLabel>>,
    unguarded_loop_labels: Vec<LoopLabel>,
}

impl Context {
    pub(crate) fn pack_template(
        &self,
        driver: Option<&LocalName>,
        captures: Option<&Captures>,
        labels_in_scope: Option<&BTreeSet<LoopLabel>>,
    ) -> PackData {
        let mut m_tys = vec![];
        let mut m_vars = vec![];
        for (name, tree) in self.vars.iter() {
            if let Some(captures) = captures {
                if let Var::Name(name) = name {
                    if !captures.names.contains_key(name) && Some(name) != driver {
                        continue;
                    }
                }
            }
            if let Some(labels_in_scope) = labels_in_scope {
                if let Var::Loop(label) = name {
                    if !labels_in_scope.contains(&LoopLabel(label.clone())) {
                        continue;
                    }
                }
            }
            m_vars.push(name.clone());
            m_tys.push(tree.ty.clone());
        }

        PackData {
            names: m_vars,
            types: m_tys,
            loop_points: self.loop_points.clone(),
            unguarded_loop_labels: self.unguarded_loop_labels.clone(),
        }
    }

    pub(crate) fn pack(
        &mut self,
        driver: Option<&LocalName>,
        captures: Option<&Captures>,
        labels_in_scope: Option<&BTreeSet<LoopLabel>>,
        net: &mut Net<Unlinked>,
    ) -> (Tree<Unlinked>, PackData) {
        let mut m_trees = vec![];
        let mut m_tys = vec![];
        let mut m_vars = vec![];
        for (name, tree) in core::mem::take(&mut self.vars) {
            if let Some(captures) = captures {
                if let Var::Name(name) = &name {
                    if !captures.names.contains_key(name) && Some(name) != driver {
                        net.link(tree.tree, Tree::Era);
                        continue;
                    }
                }
            }
            if let Some(labels_in_scope) = labels_in_scope {
                if let Var::Loop(label) = &name {
                    if !labels_in_scope.contains(&LoopLabel(label.clone())) {
                        net.link(tree.tree, Tree::Era);
                        continue;
                    }
                }
            }
            m_vars.push(name);
            m_trees.push(tree.tree);
            m_tys.push(tree.ty);
        }
        let context_in = multiplex_trees(m_trees);
        (
            context_in,
            PackData {
                names: m_vars,
                types: m_tys,
                loop_points: core::mem::take(&mut self.loop_points),
                unguarded_loop_labels: core::mem::take(&mut self.unguarded_loop_labels),
            },
        )
    }

    pub(crate) fn unpack(&mut self, packed: &PackData, net: &mut Net<Unlinked>) -> Tree<Unlinked> {
        let mut m_trees = vec![];
        for (name, ty) in packed.names.iter().zip(packed.types.iter()) {
            let (v0, v1) = net.create_wire();
            self.bind_variable(name.clone(), v0.with_type(ty.clone()));
            m_trees.push(v1);
        }
        self.loop_points = packed.loop_points.clone();
        self.unguarded_loop_labels = packed.unguarded_loop_labels.clone();
        let context_out = demultiplex_trees(m_trees);
        context_out
    }

    fn bind_variable(&mut self, var: Var, tree: TypedTree) {
        match self.vars.insert(var.clone(), tree) {
            Some(x) => panic!("{:?}", x),
            None => (),
        }
    }
}

pub(crate) struct Compiler {
    net: Net<Unlinked>,
    context: Context,
    type_defs: TypeDefs<Universal>,
    definitions: IndexMap<
        GlobalName<Universal>,
        (
            Definition<Arc<Expression<Type<Universal>, Universal>>, Universal>,
            Type<Universal>,
        ),
    >,
    global_name_to_id: IndexMap<GlobalName<Universal>, usize>,
    id_to_package: Vec<Net<Unlinked>>,
    lazy_redexes: Vec<(Tree<Unlinked>, Tree<Unlinked>)>,
    compile_global_stack: IndexSet<GlobalName<Universal>>,
    package_is_case_branch: IndexMap<usize, ArcStr>,
    blocks: IndexMap<usize, Arc<Process<Type<Universal>, Universal>>>,
    poll_packages: IndexMap<LocalName, PollInfo>,
    max_interactions: u32,
}

#[derive(Clone)]
struct PollInfo {
    package_id: usize,
    labels_in_scope: BTreeSet<LoopLabel>,
    driver: LocalName,
}

fn poll_token_tree() -> Tree<Unlinked> {
    Tree::External(POLL_TOKEN.into())
}

impl Tree<Unlinked> {
    pub(crate) fn with_type(self, ty: Type<Universal>) -> TypedTree {
        TypedTree { tree: self, ty }
    }
}

pub(crate) fn multiplex_trees(mut trees: Vec<Tree<Unlinked>>) -> Tree<Unlinked> {
    if trees.len() == 0 {
        Tree::Break
    } else if trees.len() == 1 {
        trees.pop().unwrap()
    } else {
        let new_trees = trees.split_off(trees.len() / 2);
        Tree::Times(
            Box::new(multiplex_trees(trees)),
            Box::new(multiplex_trees(new_trees)),
        )
    }
}
pub(crate) fn demultiplex_trees(mut trees: Vec<Tree<Unlinked>>) -> Tree<Unlinked> {
    if trees.len() == 0 {
        Tree::Continue
    } else if trees.len() == 1 {
        trees.pop().unwrap()
    } else {
        let new_trees = trees.split_off(trees.len() / 2);
        Tree::Par(
            Box::new(demultiplex_trees(trees)),
            Box::new(demultiplex_trees(new_trees)),
        )
    }
}

impl Compiler {
    fn build_submit_stream(&mut self, items: Vec<TypedTree>) -> Tree<Unlinked> {
        let mut stream = Tree::Signal(ArcStr::from("#end"), Box::new(Tree::Break));
        for item in items.into_iter().rev() {
            stream = Tree::Signal(
                ArcStr::from("#item"),
                Box::new(Tree::Times(Box::new(stream), Box::new(item.tree))),
            );
        }
        stream
    }

    fn compile_global(&mut self, name: &GlobalName<Universal>) -> Result<TypedTree> {
        if let Some(id) = self.global_name_to_id.get(name) {
            return Ok(TypedTree {
                tree: Tree::Package(*id, Box::new(Tree::Break), FanBehavior::Expand),
                ty: Type::Break(Span::None),
            });
        };
        if !self.compile_global_stack.insert(name.clone()) {
            return Err(Error::DependencyCycle {
                global: name.clone(),
                dependents: self.compile_global_stack.clone(),
            });
        }
        let global = match self.definitions.get(name).cloned() {
            Some((def, _typ)) => match def.body {
                DefinitionBody::Par(expr) => expr,
                DefinitionBody::External(_) => {
                    let def_ref = Unlinked {
                        package: name.module.package.clone(),
                        path: name.module.directories.clone(),
                        module: name.module.module.clone(),
                        name: name.primary.clone(),
                    };
                    Arc::new(Expression::External(def_ref, Type::Break(Span::None)))
                }
            },
            _ => return Err(Error::GlobalNotFound(name.clone())),
        };

        let (id, typ) = self.in_package(name.to_string(), |this, _| {
            let mut s = String::new();
            global.pretty(&mut s, 0).unwrap();
            Ok((
                this.compile_expression(global.as_ref())?,
                (Tree::Continue).with_type(Type::Continue(Span::None)),
            ))
        })?;
        self.global_name_to_id.insert(name.clone(), id);
        self.compile_global_stack.shift_remove(name);
        Ok(Tree::Package(id, Box::new(Tree::Break), FanBehavior::Expand).with_type(typ))
    }

    /// Optimize away erasure underneath auxiliary ports of DUP and CON nodes where it is safe to do so.
    ///
    /// Expects vars to be already have been substituted.
    fn apply_safe_rules(&mut self, tree: Tree<Unlinked>) -> Tree<Unlinked> {
        match tree {
            Tree::Dup(a, b) => {
                let a = self.apply_safe_rules(*a);
                let b = self.apply_safe_rules(*b);
                match (a, b) {
                    // This is unconditionally valid on the initial net because no "sup" nodes (dups with opposite polarity) are created in an initial net.
                    (Tree::Era, x) | (x, Tree::Era) => x,
                    (a, b) => Tree::Dup(Box::new(a), Box::new(b)),
                }
            }
            Tree::Times(a, b) => {
                let a = self.apply_safe_rules(*a);
                let b = self.apply_safe_rules(*b);
                match (a, b) {
                    (Tree::Era, Tree::Era) => {
                        // Eta reduction is always correct
                        Tree::Era
                    }
                    (a, b) => {
                        // TODO optimize `!` and `?`
                        Tree::Times(Box::new(a), Box::new(b))
                    }
                }
            }
            Tree::Package(id, cx, b) => {
                let cx = self.apply_safe_rules(*cx);
                Tree::Package(id, Box::new(cx), b)
            }
            Tree::Signal(signal, a) => {
                let a = self.apply_safe_rules(*a);
                Tree::Signal(signal, Box::new(a))
            }
            Tree::Choice(a, branches, else_branch) => {
                let a = self.apply_safe_rules(*a);
                Tree::Choice(Box::new(a), branches, else_branch)
            }
            tree => tree,
        }
    }

    /// Reduces the tree in ways that aren't regular interactions. This might be invalid after the net has been reduced with regular interactions such as after calling [`Self::normal()`].
    fn non_principal_interactions(&mut self, mut tree: Tree<Unlinked>) -> Tree<Unlinked> {
        self.net.substitute_tree(&mut tree);
        self.apply_safe_rules(tree)
    }

    fn in_package(
        &mut self,
        debug_name: String,
        f: impl FnOnce(&mut Self, usize) -> Result<(TypedTree, TypedTree)>,
    ) -> Result<(usize, Type<Universal>)> {
        let id = self.id_to_package.len();
        let old_net = core::mem::take(&mut self.net);
        let old_lazy_redexes = core::mem::take(&mut self.lazy_redexes);
        // Allocate package
        self.id_to_package.push(Default::default());
        let (mut root, captures) = self.with_captures(&Captures::default(), |this| f(this, id))?;

        // Non-principal interaction optimization pass
        root.tree = self.non_principal_interactions(root.tree);

        self.lazy_redexes = core::mem::take(&mut self.lazy_redexes)
            .into_iter()
            .map(|(tree, tree1)| {
                (
                    self.non_principal_interactions(tree),
                    self.non_principal_interactions(tree1),
                )
            })
            .collect();
        self.net.redexes = core::mem::take(&mut self.net.redexes)
            .into_iter()
            .map(|(tree, tree1)| {
                (
                    self.non_principal_interactions(tree),
                    self.non_principal_interactions(tree1),
                )
            })
            .collect();

        self.net.ports.push_back(root.tree);
        self.net.ports.push_back(captures.tree);

        self.net.packages = Arc::new(self.id_to_package.clone().into_iter().enumerate().collect());
        // self.net.assert_valid_with(
        //     self.lazy_redexes
        //         .iter()
        //         .map(|(a, b)| [a, b].into_iter())
        //         .flatten(),
        // );
        // This is less efficient, but at least panics will now contain the lazy redexes
        let mut net2 = self.net.clone();
        net2.redexes.append(&mut self.lazy_redexes.clone().into());
        net2.assert_valid();

        self.net.debug_name = debug_name;
        self.net.normal(self.max_interactions);
        self.net
            .redexes
            .append(&mut core::mem::take(&mut self.lazy_redexes).into());
        self.net.assert_valid();
        *self.id_to_package.get_mut(id).unwrap() = core::mem::take(&mut self.net);
        self.lazy_redexes = old_lazy_redexes;
        self.net = old_net;

        Ok((id, root.ty))
    }

    fn with_captures<T>(
        &mut self,
        captures: &Captures,
        f: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let mut vars = BTreeMap::new();
        for (name, (_span, usage)) in captures.names.iter() {
            let tree = self.use_variable(name, usage, false)?;
            vars.insert(Var::Name(name.clone()), tree);
        }
        for (label, _) in self.context.loop_points.clone().iter() {
            let tree = self.use_var(&Var::Loop(label.0.clone()), &VariableUsage::Copy, false)?;
            vars.insert(Var::Loop(label.0.clone()), tree);
        }
        let loop_points_before = self.context.loop_points.clone();
        core::mem::swap(&mut vars, &mut self.context.vars);
        let t = f(self);
        self.context.vars = vars;
        self.context.loop_points = loop_points_before;
        t
    }

    fn bind_variable(&mut self, var: impl Into<Var>, tree: TypedTree) -> Result<()> {
        let prev = self.context.vars.insert(var.into(), tree);
        match prev {
            Some(prev_tree) => {
                let disposer = if prev_tree.ty.is_linear(&self.type_defs).unwrap_or(true)
                    && prev_tree.ty.is_once(&self.type_defs).unwrap_or(false)
                {
                    Tree::Close
                } else {
                    Tree::Era
                };
                self.net.link(prev_tree.tree, disposer);
                Ok(())
            }
            None => Ok(()),
        }
    }

    fn use_var(&mut self, var: &Var, usage: &VariableUsage, in_command: bool) -> Result<TypedTree> {
        match self.context.vars.remove(var) {
            Some(tree) => {
                if in_command {
                    return Ok(tree);
                }
                match usage {
                    VariableUsage::Move => Ok(tree),
                    VariableUsage::Copy => {
                        let (w0, w1) = self.net.create_wire();
                        let (v0, v1) = self.net.create_wire();
                        self.net
                            .link(Tree::Dup(Box::new(v0), Box::new(w0)), tree.tree);
                        self.context.vars.insert(
                            var.clone(),
                            TypedTree {
                                tree: w1,
                                ty: tree.ty.clone(),
                            },
                        );
                        Ok(TypedTree {
                            tree: v1,
                            ty: tree.ty.clone(),
                        })
                    }
                    VariableUsage::Unknown => Err(Error::UnknownVariableUsage(Span::None)),
                }
            }
            _ => Err(Error::UnboundVar(Default::default(), var.clone())),
        }
    }

    fn use_global(&mut self, name: &GlobalName<Universal>) -> Result<TypedTree> {
        match self.compile_global(name) {
            Ok(value) => Ok(value),
            Err(Error::GlobalNotFound(_)) => Err(Error::GlobalNotFound(name.clone())),
            Err(e) => Err(e),
        }
    }

    fn use_variable(
        &mut self,
        name: &LocalName,
        usage: &VariableUsage,
        in_command: bool,
    ) -> Result<TypedTree> {
        self.use_var(&Var::Name(name.clone()), usage, in_command)
    }

    fn create_typed_wire(&mut self) -> (TypedTree, TypedTree) {
        let (v0, v1) = self.net.create_wire();
        (
            TypedTree {
                tree: v0,
                ty: Type::Break(Span::None),
            },
            TypedTree {
                tree: v1,
                ty: Type::Break(Span::None),
            },
        )
    }

    fn link_typed(&mut self, a: TypedTree, b: TypedTree) {
        self.net.link(a.tree, b.tree);
    }

    fn either_instance(&mut self, signal: ArcStr, tree: Tree<Unlinked>) -> Tree<Unlinked> {
        Tree::Signal(signal, Box::new(tree))
    }

    fn choice_instance(
        &mut self,
        ctx_out: Tree<Unlinked>,
        branches: HashMap<ArcStr, usize>,
        else_branch: Option<usize>,
    ) -> Tree<Unlinked> {
        Tree::Choice(Box::new(ctx_out), Arc::new(branches), else_branch)
    }

    fn compile_expression(
        &mut self,
        expr: &Expression<Type<Universal>, Universal>,
    ) -> Result<TypedTree> {
        match expr {
            Expression::Global(_, name, _) => self.use_global(name),
            Expression::Variable(_, name, _typ, usage) => {
                Ok(self.use_variable(name, usage, false)?)
            }

            Expression::Box(span, captures, expression, typ) => {
                self.with_captures(captures, |this| {
                    let (context_in, pack_data) =
                        this.context.pack(None, None, None, &mut this.net);
                    let (package_id, _) =
                        this.in_package(format!("Box at {span}"), |this, _| {
                            let context_out = this.context.unpack(&pack_data, &mut this.net);
                            let body = this.compile_expression(&expression)?;
                            this.end_context()?;
                            Ok((body, context_out.with_type(Type::Break(Span::default()))))
                        })?;
                    Ok(
                        Tree::Package(package_id, Box::new(context_in), FanBehavior::Propagate)
                            .with_type(typ.clone()),
                    )
                })
            }

            Expression::Chan {
                captures,
                chan_name,
                chan_type: _chan_type,
                process,
                ..
            } => self.with_captures(captures, |this| {
                let (v0, v1) = this.create_typed_wire();
                this.bind_variable(chan_name.clone(), v0)?;
                this.compile_process(process)?;
                Ok(v1)
            }),

            Expression::Primitive(_, value, _) => Ok(TypedTree {
                tree: Tree::Primitive(value.clone()),
                ty: get_primitive_type(value),
            }),

            Expression::External(f, typ) => Ok(TypedTree {
                tree: Tree::External(f.clone()),
                ty: typ.clone(),
            }),
        }
    }

    fn compile_process(&mut self, proc: &Process<Type<Universal>, Universal>) -> Result<()> {
        let mut received_parameters = Vec::new();
        for step in &proc.steps {
            match step {
                Step::Let { name, value, .. } => {
                    let value = self.compile_expression(value)?;
                    self.bind_variable(name.clone(), value)?;
                }
                Step::Do {
                    span,
                    name,
                    usage,
                    typ,
                    command,
                } => self.compile_command(
                    span,
                    name.clone(),
                    usage,
                    typ.clone(),
                    command,
                    &mut received_parameters,
                )?,
            }
        }

        match &proc.terminator {
            Terminator::Do {
                span,
                name,
                usage,
                typ,
                command,
            } => self.compile_terminal_command(span, name, usage, typ, command)?,

            Terminator::Poll {
                span: _span,
                kind,
                driver,
                point,
                clients,
                name,
                name_typ: _,
                captures,
                then,
                else_,
            } => self.compile_process_poll(
                proc.span(),
                kind,
                driver,
                point,
                clients,
                name,
                captures,
                then,
                else_,
            )?,

            Terminator::Submit {
                span: _span,
                driver,
                point,
                values,
                captures,
            } => self.compile_process_submit(driver, point, values, captures)?,

            Terminator::Block(_, index, body, process) => {
                self.compile_process_block(*index, body, process)?
            }

            Terminator::Goto(_, index, _) => self.compile_process_goto(*index)?,

            Terminator::Unreachable(_) => self.compile_process_unreachable()?,
        }

        for (parameter, was_empty_before) in received_parameters.into_iter().rev() {
            if was_empty_before {
                self.type_defs.vars.shift_remove(&parameter);
            }
        }
        Ok(())
    }

    fn compile_command(
        &mut self,
        _span: &Span,
        name: LocalName,
        usage: &VariableUsage,
        _ty: Type<Universal>,
        cmd: &Command<Type<Universal>, Universal>,
        received_parameters: &mut Vec<(LocalName, bool)>,
    ) -> Result<()> {
        match cmd {
            Command::Noop => {}
            Command::Close => self.compile_command_close(&name, usage)?,
            // types get erased.
            Command::SendType(_argument) => self.compile_command_send_type(name, usage)?,
            Command::ReceiveType(parameter) => {
                let was_empty_before = self.compile_command_receive_type(name, usage, parameter)?;
                received_parameters.push((parameter.name.clone(), was_empty_before));
            }
            Command::Send(expr) => self.compile_command_send(name, usage, expr)?,
            Command::Receive(target, _, _, _) => {
                self.compile_command_receive(name, usage, target)?
            }
            Command::Signal(chosen) => self.compile_command_signal(name, usage, chosen)?,
            Command::Continue => self.compile_command_continue(&name, usage)?,
        };
        Ok(())
    }

    fn compile_terminal_command(
        &mut self,
        span: &Span,
        name: &LocalName,
        usage: &VariableUsage,
        _ty: &Type<Universal>,
        cmd: &TerminalCommand<Type<Universal>, Universal>,
    ) -> Result<()> {
        match cmd {
            TerminalCommand::Link(expr) => self.compile_command_link(name, usage, expr)?,
            TerminalCommand::Case(names, processes, else_process) => self.compile_command_case(
                span,
                name.clone(),
                usage,
                names,
                processes,
                else_process,
            )?,
            TerminalCommand::Break => self.compile_command_break(name, usage)?,
            TerminalCommand::Begin {
                label,
                captures,
                body,
                ..
            } => self.compile_command_begin(span, name, label, captures, body)?,
            TerminalCommand::Loop(label, driver, captures) => {
                self.compile_command_loop(span, name, usage, label, driver, captures)?
            }
        };
        Ok(())
    }

    fn compile_process_poll(
        &mut self,
        span: Span,
        kind: &PollKind,
        driver: &LocalName,
        point: &LocalName,
        clients: &[Arc<Expression<Type<Universal>, Universal>>],
        name: &LocalName,
        captures: &Captures,
        then: &Arc<Process<Type<Universal>, Universal>>,
        else_: &Arc<Process<Type<Universal>, Universal>>,
    ) -> Result<()> {
        let labels_in_scope: BTreeSet<_> = self.context.loop_points.keys().cloned().collect();

        if matches!(kind, PollKind::Poll) {
            self.bind_variable(
                driver.clone(),
                poll_token_tree().with_type(Type::Break(Span::None)),
            )?;
        }

        if !clients.is_empty() {
            let mut initial_client_trees = Vec::with_capacity(clients.len());
            for client in clients {
                initial_client_trees.push(self.compile_expression(client)?);
            }

            let driver_tree = self.use_variable(driver, &VariableUsage::Move, false)?;
            let (next0, next1) = self.create_typed_wire();
            let stream = self.build_submit_stream(initial_client_trees);
            let payload = Tree::Times(Box::new(next1.tree), Box::new(stream));
            let request = Tree::Signal(ArcStr::from("#submit"), Box::new(payload));
            self.net.link(driver_tree.tree, request);
            self.bind_variable(driver.clone(), next0)?;
        }

        let captures = captures.clone();
        let driver = driver.clone();
        let point = point.clone();
        let name = name.clone();
        let then = then.clone();
        let else_ = else_.clone();

        let pack_template =
            self.context
                .pack_template(Some(&driver), Some(&captures), Some(&labels_in_scope));

        let (context_in, _pack_data) = self.context.pack(
            Some(&driver),
            Some(&captures),
            Some(&labels_in_scope),
            &mut self.net,
        );

        let (poll_package_id, _) =
            self.in_package(format!("poll body at {span:?}"), |this, package_id| {
                this.poll_packages.insert(
                    point.clone(),
                    PollInfo {
                        package_id,
                        labels_in_scope: labels_in_scope.clone(),
                        driver: driver.clone(),
                    },
                );

                let context_out = this.context.unpack(&pack_template, &mut this.net);
                let driver_tree = this.use_variable(&driver, &VariableUsage::Move, false)?;
                let (next0, next1) = this.create_typed_wire();
                let (result0, result1) = this.create_typed_wire();

                let payload = Tree::Times(Box::new(next1.tree), Box::new(result1.tree));
                let request = Tree::Signal(ArcStr::from("#poll"), Box::new(payload));
                this.net.link(driver_tree.tree, request);
                this.bind_variable(driver.clone(), next0)?;

                this.context.unguarded_loop_labels.clear();
                let (case_context_in, case_pack_data) =
                    this.context.pack(None, None, None, &mut this.net);

                let mut branches = HashMap::new();

                {
                    let (package_id, _) =
                        this.in_package("poll #client branch".to_string(), |this, _| {
                            let (w0, w1) = this.create_typed_wire();
                            this.bind_variable(name.clone(), w0)?;
                            let context_out = this.context.unpack(&case_pack_data, &mut this.net);
                            this.compile_process(&then)?;
                            Ok((w1, context_out.with_type(Type::Continue(Span::None))))
                        })?;
                    branches.insert(ArcStr::from("#client"), package_id);
                }

                {
                    let (package_id, _) =
                        this.in_package("poll #empty branch".to_string(), |this, _| {
                            let context_out = this.context.unpack(&case_pack_data, &mut this.net);
                            if let Ok(driver_tree) =
                                this.use_variable(&driver, &VariableUsage::Move, false)
                            {
                                this.net.link(
                                    driver_tree.tree,
                                    Tree::Signal(ArcStr::from("#close"), Box::new(Tree::Break)),
                                );
                            }
                            this.compile_process(&else_)?;
                            Ok((
                                Tree::Era.with_type(Type::Break(Span::None)),
                                context_out.with_type(Type::Continue(Span::None)),
                            ))
                        })?;
                    branches.insert(ArcStr::from("#empty"), package_id);
                }

                let (else_branch, _) =
                    this.in_package("poll (invalid branch)".to_string(), |this, _| {
                        let context_out = this.context.unpack(&case_pack_data, &mut this.net);
                        this.end_context()?;
                        Ok((
                            Tree::Era.with_type(Type::Break(Span::None)),
                            context_out.with_type(Type::Continue(Span::None)),
                        ))
                    })?;

                let choice = this.choice_instance(case_context_in, branches, Some(else_branch));
                this.net.link(result0.tree, choice);

                Ok((
                    Tree::Era.with_type(Type::Break(Span::None)),
                    context_out.with_type(Type::Continue(Span::None)),
                ))
            })?;

        self.lazy_redexes.push((
            Tree::Package(poll_package_id, Box::new(context_in), FanBehavior::Expand),
            Tree::Break,
        ));
        Ok(())
    }

    fn compile_process_submit(
        &mut self,
        driver: &LocalName,
        point: &LocalName,
        values: &[Arc<Expression<Type<Universal>, Universal>>],
        captures: &Captures,
    ) -> Result<()> {
        let Some(poll_info) = self.poll_packages.get(point).cloned() else {
            panic!("Submit to unknown poll-point during compilation: {point}");
        };
        if poll_info.driver != *driver {
            panic!("Submit poll-point driver mismatch: submit to {driver}, point {point}");
        }
        let poll_package_id = poll_info.package_id;

        let mut value_trees = Vec::with_capacity(values.len());
        for value in values {
            value_trees.push(self.compile_expression(value)?);
        }

        let driver_tree = self.use_variable(driver, &VariableUsage::Move, false)?;
        let (next0, next1) = self.create_typed_wire();
        let stream = self.build_submit_stream(value_trees);
        let payload = Tree::Times(Box::new(next1.tree), Box::new(stream));
        let request = Tree::Signal(ArcStr::from("#submit"), Box::new(payload));
        self.net.link(driver_tree.tree, request);
        self.bind_variable(driver.clone(), next0)?;

        let (context_in, _pack_data) = self.context.pack(
            Some(driver),
            Some(captures),
            Some(&poll_info.labels_in_scope),
            &mut self.net,
        );
        self.lazy_redexes.push((
            Tree::Package(poll_package_id, Box::new(context_in), FanBehavior::Expand),
            Tree::Break,
        ));
        Ok(())
    }

    fn compile_process_block(
        &mut self,
        index: usize,
        body: &Arc<Process<Type<Universal>, Universal>>,
        process: &Arc<Process<Type<Universal>, Universal>>,
    ) -> Result<()> {
        let prev = self.blocks.insert(index, body.clone());
        self.compile_process(process)?;
        match prev {
            Some(old) => {
                self.blocks.insert(index, old);
            }
            None => {
                self.blocks.shift_remove(&index);
            }
        }
        Ok(())
    }

    fn compile_process_goto(&mut self, index: usize) -> Result<()> {
        let body = self
            .blocks
            .get(&index)
            .expect("goto target missing during compilation")
            .clone();
        self.compile_process(&body)
    }

    fn compile_process_unreachable(&mut self) -> Result<()> {
        self.end_context()?;
        Ok(())
    }

    fn compile_command_link(
        &mut self,
        name: &LocalName,
        usage: &VariableUsage,
        expr: &Expression<Type<Universal>, Universal>,
    ) -> Result<()> {
        let subject = self.use_variable(name, usage, true)?;
        let value = self.compile_expression(expr)?;
        self.link_typed(subject, value);
        self.end_context()?;
        Ok(())
    }

    fn compile_command_send_type(&mut self, name: LocalName, usage: &VariableUsage) -> Result<()> {
        let subject = self.use_variable(&name, usage, true)?;
        self.bind_variable(name, subject.tree.with_type(Type::Break(Span::None)))?;
        Ok(())
    }

    fn compile_command_close(&mut self, name: &LocalName, usage: &VariableUsage) -> Result<()> {
        let subject = self.use_variable(name, usage, true)?;
        self.net.link(subject.tree, Tree::Close);
        Ok(())
    }

    fn compile_command_receive_type(
        &mut self,
        name: LocalName,
        usage: &VariableUsage,
        parameter: &TypeParameter,
    ) -> Result<bool> {
        let subject = self.use_variable(&name, usage, true)?;
        let was_empty_before = self
            .type_defs
            .vars
            .insert(parameter.name.clone(), parameter.constraint)
            .is_none();
        self.bind_variable(name, subject.tree.with_type(Type::Break(Span::None)))?;
        Ok(was_empty_before)
    }

    fn compile_command_send(
        &mut self,
        name: LocalName,
        usage: &VariableUsage,
        expr: &Expression<Type<Universal>, Universal>,
    ) -> Result<()> {
        let subject = self.use_variable(&name, usage, true)?;
        let expr = self.compile_expression(expr)?;
        let (v0, v1) = self.create_typed_wire();
        self.bind_variable(name, v0)?;
        self.net.link(
            Tree::Times(Box::new(v1.tree), Box::new(expr.tree)),
            subject.tree,
        );
        Ok(())
    }

    fn compile_command_receive(
        &mut self,
        name: LocalName,
        usage: &VariableUsage,
        target: &LocalName,
    ) -> Result<()> {
        let subject = self.use_variable(&name, usage, true)?;
        let (v0, v1) = self.create_typed_wire();
        let (w0, w1) = self.create_typed_wire();
        self.bind_variable(name, w0)?;
        self.bind_variable(target.clone(), v0)?;
        self.net.link(
            Tree::Par(Box::new(w1.tree), Box::new(v1.tree)),
            subject.tree,
        );
        Ok(())
    }

    fn compile_command_signal(
        &mut self,
        name: LocalName,
        usage: &VariableUsage,
        chosen: &LocalName,
    ) -> Result<()> {
        let subject = self.use_variable(&name, usage, true)?;
        let (v0, v1) = self.create_typed_wire();
        let choosing_tree = self.either_instance(ArcStr::from(&chosen.string), v1.tree);
        self.net.link(choosing_tree, subject.tree);
        self.bind_variable(name, v0)?;
        Ok(())
    }

    fn compile_command_case(
        &mut self,
        span: &Span,
        name: LocalName,
        usage: &VariableUsage,
        names: &[LocalName],
        processes: &[Arc<Process<Type<Universal>, Universal>>],
        else_process: &Option<Arc<Process<Type<Universal>, Universal>>>,
    ) -> Result<()> {
        self.context.unguarded_loop_labels.clear();
        let old_tree = self.use_variable(&name, usage, true)?;
        let (context_in, pack_data) = self.context.pack(None, None, None, &mut self.net);

        let mut branches = HashMap::new();
        let mut choice_and_process: Vec<_> = names.iter().zip(processes.iter()).collect();
        choice_and_process.sort_by_key(|k| k.0);

        for (branch_name, process) in choice_and_process {
            let branch_name = ArcStr::from(&branch_name.string);
            let (package_id, _) =
                self.in_package(format!("Branch {branch_name} at {span}"), |this, id| {
                    this.package_is_case_branch.insert(id, branch_name.clone());
                    let (w0, w1) = this.create_typed_wire();
                    this.bind_variable(name.clone(), w0)?;
                    let context_out = this.context.unpack(&pack_data, &mut this.net);
                    this.compile_process(process)?;
                    Ok((
                        w1,
                        context_out.with_type(Type::Continue(Default::default())),
                    ))
                })?;
            branches.insert(branch_name, package_id);
        }

        let else_branch = match else_process {
            Some(process) => {
                let (package_id, _) =
                    self.in_package(format!("Else branch at {span}"), |this, id| {
                        this.package_is_case_branch.insert(id, ArcStr::from(""));
                        let (w0, w1) = this.create_typed_wire();
                        this.bind_variable(name.clone(), w0)?;
                        let context_out = this.context.unpack(&pack_data, &mut this.net);
                        this.compile_process(process)?;
                        Ok((
                            w1,
                            context_out.with_type(Type::Continue(Default::default())),
                        ))
                    })?;
                Some(package_id)
            }
            None => None,
        };

        let t = self.choice_instance(context_in, branches, else_branch);
        self.net.link(old_tree.tree, t);
        Ok(())
    }

    fn compile_command_break(&mut self, name: &LocalName, usage: &VariableUsage) -> Result<()> {
        let a = self.use_variable(name, usage, true)?.tree;
        self.net.link(a, Tree::Break);
        self.end_context()?;
        Ok(())
    }

    fn compile_command_continue(&mut self, name: &LocalName, usage: &VariableUsage) -> Result<()> {
        let a = self.use_variable(name, usage, true)?.tree;
        self.net.link(a, Tree::Continue);
        Ok(())
    }

    fn compile_command_begin(
        &mut self,
        span: &Span,
        name: &LocalName,
        label: &Option<LocalName>,
        captures: &Captures,
        body: &Arc<Process<Type<Universal>, Universal>>,
    ) -> Result<()> {
        let label = LoopLabel(label.clone());

        let (def0, def1) = self.net.create_wire();
        let prev = self.context.vars.insert(
            Var::Loop(label.0.clone()),
            def0.with_type(Type::Break(Span::default())),
        );

        if let Some(prev_tree) = prev {
            self.net.link(prev_tree.tree, Tree::Era);
        }

        let mut labels_in_scope: BTreeSet<_> = self.context.loop_points.keys().cloned().collect();
        labels_in_scope.insert(label.clone());
        self.context
            .loop_points
            .insert(label.clone(), labels_in_scope);
        self.context.unguarded_loop_labels.push(label.clone());

        let (context_in, pack_data) =
            self.context
                .pack(Some(name), Some(captures), None, &mut self.net);
        let (id, _) = self.in_package(format!("Loop body at {span}"), |this, _| {
            let context_out = this.context.unpack(&pack_data, &mut this.net);
            this.compile_process(body)?;
            Ok((
                context_out.with_type(Type::Break(Span::default())),
                (Tree::Continue).with_type(Type::Continue(Span::default())),
            ))
        })?;
        self.net.link(
            def1,
            Tree::Package(id, Box::new(Tree::Break), FanBehavior::Propagate),
        );
        self.net.link(
            context_in,
            Tree::Package(id, Box::new(Tree::Break), FanBehavior::Propagate),
        );
        Ok(())
    }

    fn compile_command_loop(
        &mut self,
        span: &Span,
        name: &LocalName,
        usage: &VariableUsage,
        label: &Option<LocalName>,
        driver: &LocalName,
        captures: &Captures,
    ) -> Result<()> {
        let label = LoopLabel(label.clone());
        if self.context.unguarded_loop_labels.contains(&label) {
            return Err(Error::UnguardedLoop(span.clone(), label.clone().0));
        }
        let tree = self.use_var(&Var::Loop(label.0.clone()), &VariableUsage::Copy, false)?;
        let driver_tree = self.use_variable(name, usage, true)?;
        self.bind_variable(driver.clone(), driver_tree)?;
        let labels_in_scope = self.context.loop_points.get(&label).unwrap().clone();
        let (context_in, _) = self.context.pack(
            Some(driver),
            Some(captures),
            Some(&labels_in_scope),
            &mut self.net,
        );
        self.net.redexes.push_back((tree.tree, context_in));
        Ok(())
    }

    fn end_context(&mut self) -> Result<()> {
        // drop all replicables
        for (_, value) in core::mem::take(&mut self.context.vars).into_iter() {
            self.net.link(value.tree, Tree::Era);
        }
        self.context.loop_points = Default::default();
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct IcCompiled {
    pub(crate) id_to_package: Arc<IndexMap<usize, Net<Unlinked>>>,
    pub(crate) name_to_id: IndexMap<GlobalName<Universal>, usize>,
    package_is_case_branch: IndexMap<usize, ArcStr>,
}

impl Display for IcCompiled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (k, v) in self.id_to_package.iter() {
            // check if it has a name
            for (name, id) in self.name_to_id.iter() {
                if id == k {
                    f.write_fmt(format_args!("// {} \n", name))?;
                }
            }
            f.write_fmt(format_args!("@{} = {}\n", k, v.show()))?;
        }
        Ok(())
    }
}

impl IcCompiled {
    pub fn get_with_name(&self, name: &GlobalName<Universal>) -> Option<Net<Unlinked>> {
        let id = self.name_to_id.get(name)?;
        self.id_to_package.get(id).cloned()
    }

    pub fn get_case_branch_name(&self, id: usize) -> Option<ArcStr> {
        self.package_is_case_branch.get(&id).cloned()
    }

    pub fn create_net(&self) -> Net<Unlinked> {
        let mut net = Net::default();
        net.packages = self.id_to_package.clone();
        net
    }

    pub fn compile_file(
        program: &CheckedModule<Universal>,
        max_interactions: u32,
    ) -> Result<IcCompiled> {
        let mut compiler = Compiler {
            net: Net::default(),
            context: Context {
                vars: BTreeMap::default(),
                loop_points: BTreeMap::default(),
                unguarded_loop_labels: Default::default(),
            },
            type_defs: program.type_defs.clone(),
            definitions: program.definitions.clone(),
            global_name_to_id: Default::default(),
            id_to_package: Default::default(),
            compile_global_stack: Default::default(),
            lazy_redexes: vec![],
            package_is_case_branch: Default::default(),
            blocks: IndexMap::new(),
            poll_packages: Default::default(),
            max_interactions: max_interactions,
        };

        for name in compiler.definitions.keys().cloned().collect::<Vec<_>>() {
            compiler.compile_global(&name)?;
        }

        Ok(IcCompiled {
            id_to_package: Arc::new(compiler.id_to_package.into_iter().enumerate().collect()),
            name_to_id: compiler.global_name_to_id,
            package_is_case_branch: compiler.package_is_case_branch,
        })
    }
}
