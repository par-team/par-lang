// why not rename this file to ast.rs?

use std::{
    collections::{BTreeMap, HashMap},
    fmt::Display,
    hash::Hash,
    sync::Arc,
};

use super::{
    process::{self, Captures},
    types::Type,
};
use crate::frontend_impl::process::VariableUsage;
use crate::{
    frontend_impl::types::error::labels_from_span,
    location::{Span, Spanning},
};
use arcstr::{ArcStr, literal};
use par_runtime::pkgid::PackageId;
use par_runtime::primitive::{ParString, Primitive};

#[derive(Clone, Debug)]
pub struct LocalName {
    pub span: Span,
    pub string: ArcStr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum TypeConstraint {
    #[default]
    Any,
    Box,
    Data,
    Number,
    Signed,
}

impl TypeConstraint {
    pub fn is_broader_or_equal_than(self, other: Self) -> bool {
        self.rank() <= other.rank()
    }

    pub fn broader(self, other: Self) -> Self {
        if self.is_broader_or_equal_than(other) {
            self
        } else {
            other
        }
    }

    pub fn narrower(self, other: Self) -> Self {
        if self.is_broader_or_equal_than(other) {
            other
        } else {
            self
        }
    }

    fn rank(self) -> u8 {
        match self {
            TypeConstraint::Any => 0,
            TypeConstraint::Box => 1,
            TypeConstraint::Data => 2,
            TypeConstraint::Number => 3,
            TypeConstraint::Signed => 4,
        }
    }
}

impl Display for TypeConstraint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeConstraint::Any => Ok(()),
            TypeConstraint::Box => write!(f, "box"),
            TypeConstraint::Data => write!(f, "data"),
            TypeConstraint::Number => write!(f, "number"),
            TypeConstraint::Signed => write!(f, "signed"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeParameter {
    pub name: LocalName,
    pub constraint: TypeConstraint,
}

impl TypeParameter {
    pub fn any(name: LocalName) -> Self {
        Self {
            name,
            constraint: TypeConstraint::Any,
        }
    }

    pub fn span(&self) -> Span {
        self.name.span()
    }
}

impl Display for TypeParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)?;
        if self.constraint != TypeConstraint::Any {
            write!(f, ": {}", self.constraint)?;
        }
        Ok(())
    }
}

impl Spanning for TypeParameter {
    fn span(&self) -> Span {
        self.name.span()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BuiltinOperatorModule {
    Data,
    Number,
    String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Unresolved {
    Path { qualifier: Option<String> },
    BuiltinOperator(BuiltinOperatorModule),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResolvedPackageRef {
    Local,
    Dependency(String),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Resolved {
    Path {
        package: ResolvedPackageRef,
        directories: Vec<String>,
        module: String,
    },
    BuiltinOperator(BuiltinOperatorModule),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Universal {
    pub package: PackageId,
    pub directories: Vec<String>,
    pub module: String,
}

#[derive(Clone, Debug)]
pub struct GlobalName<S> {
    pub span: Span,
    pub module: S,
    pub primary: String,
}

impl GlobalName<Unresolved> {
    pub fn external(module: Option<&'static str>, primary: &'static str) -> Self {
        Self::new(
            Default::default(),
            Unresolved::Path {
                qualifier: module.map(String::from),
            },
            String::from(primary),
        )
    }

    pub fn builtin_operator(
        span: Span,
        module: BuiltinOperatorModule,
        primary: impl Into<String>,
    ) -> Self {
        Self::new(span, Unresolved::BuiltinOperator(module), primary.into())
    }
}

impl GlobalName<Universal> {
    pub fn is_primary_export(&self) -> bool {
        self.primary == self.module.module
    }
}

impl<S> GlobalName<S> {
    pub fn new(span: Span, module: S, primary: String) -> Self {
        Self {
            span,
            module,
            primary,
        }
    }

    pub fn map_module_path<T>(self, mut map: impl FnMut(S) -> T) -> GlobalName<T> {
        GlobalName {
            span: self.span,
            module: map(self.module),
            primary: self.primary,
        }
    }
}

impl From<ArcStr> for LocalName {
    fn from(value: ArcStr) -> Self {
        LocalName {
            span: Span::None,
            string: value,
        }
    }
}

impl Spanning for LocalName {
    fn span(&self) -> Span {
        self.span.clone()
    }
}

impl<S> Spanning for GlobalName<S> {
    fn span(&self) -> Span {
        self.span.clone()
    }
}

impl LocalName {
    pub fn result() -> Self {
        Self {
            span: Span::None,
            string: literal!("#result"),
        }
    }

    pub fn object() -> Self {
        Self {
            span: Span::None,
            string: literal!("#object"),
        }
    }

    pub fn subject() -> Self {
        Self {
            span: Span::None,
            string: literal!("#subject"),
        }
    }

    pub fn error() -> Self {
        Self {
            span: Span::None,
            string: literal!("#error"),
        }
    }

    pub fn match_(level: usize) -> Self {
        Self {
            span: Span::None,
            string: arcstr::format!("#match{}", level),
        }
    }

    pub fn temp() -> Self {
        Self {
            span: Span::None,
            string: literal!("#temp"),
        }
    }

    pub fn invalid() -> Self {
        Self {
            span: Span::None,
            string: literal!("#invalid"),
        }
    }

    /// Check if this is an internal pattern matching variable.
    pub fn is_match(&self) -> bool {
        self.string.starts_with("#match")
    }
}

#[derive(Clone, Debug)]
pub enum Pattern<S> {
    Name(Span, LocalName, Option<Type<S>>),
    Receive(Span, Box<Self>, Box<Self>, Vec<TypeParameter>),
    Continue(Span),
    ReceiveType(Span, TypeParameter, Box<Self>),
    Try(Span, Option<LocalName>, Box<Self>),
    Default(Span, Box<Expression<S>>, Box<Self>),
}

#[derive(Clone, Debug)]
pub enum Condition<S> {
    Bool(Span, Box<Expression<S>>),
    Is {
        span: Span,
        value: Expression<S>,
        variant: LocalName,
        pattern: Pattern<S>,
    },
    And(Span, Box<Self>, Box<Self>),
    Or(Span, Box<Self>, Box<Self>),
    Not(Span, Box<Self>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArithmeticOperator {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComparisonOperator {
    Less,
    Greater,
    LessOrEqual,
    GreaterOrEqual,
    Equal,
    NotEqual,
}

#[derive(Clone, Debug)]
pub struct ComparisonStep<S> {
    pub op_span: Span,
    pub op: ComparisonOperator,
    pub expr: Expression<S>,
}

#[derive(Clone, Debug)]
pub enum TemplatePart<S> {
    Literal(ArcStr),
    StringExpr(Expression<S>),
    DataExpr(Expression<S>),
}

impl<S> Condition<S> {
    pub fn span(&self) -> Span {
        match self {
            Self::Bool(span, _)
            | Self::Is { span, .. }
            | Self::And(span, _, _)
            | Self::Or(span, _, _)
            | Self::Not(span, _) => span.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Expression<S> {
    Primitive(Span, Primitive),
    Template {
        span: Span,
        parts: Vec<TemplatePart<S>>,
    },
    List(Span, Vec<Self>),
    Global(Span, GlobalName<S>),
    Variable(Span, LocalName),
    Poll {
        span: Span,
        label: Option<LocalName>,
        clients: Vec<Expression<S>>,
        name: LocalName,
        then: Box<Expression<S>>,
        else_: Box<Expression<S>>,
    },
    Repoll {
        span: Span,
        label: Option<LocalName>,
        clients: Vec<Expression<S>>,
        name: LocalName,
        then: Box<Expression<S>>,
        else_: Box<Expression<S>>,
    },
    Submit {
        span: Span,
        label: Option<LocalName>,
        values: Vec<Expression<S>>,
    },
    Condition(Span, Box<Condition<S>>),
    Grouped(Span, Box<Self>),
    TypeIn {
        span: Span,
        typ: Type<S>,
        expr: Box<Self>,
    },
    Let {
        span: Span,
        pattern: Pattern<S>,
        expression: Box<Self>,
        then: Box<Self>,
    },
    Catch {
        span: Span,
        label: Option<LocalName>,
        pattern: Pattern<S>,
        block: Box<Self>,
        then: Box<Self>,
    },
    Throw(Span, Option<LocalName>, Box<Self>),
    If {
        span: Span,
        branches: Vec<(Condition<S>, Expression<S>)>,
        else_: Option<Box<Self>>,
    },
    Do {
        span: Span,
        process: Box<Process<S>>,
        then: Box<Self>,
    },
    Box(Span, Box<Self>),
    Chan {
        span: Span,
        pattern: Pattern<S>,
        process: Box<Process<S>>,
    },
    Arithmetic {
        span: Span,
        op_span: Span,
        op: ArithmeticOperator,
        left: Box<Self>,
        right: Box<Self>,
    },
    Neg {
        span: Span,
        op_span: Span,
        expr: Box<Self>,
    },
    ComparisonChain {
        span: Span,
        first: Box<Self>,
        rest: Vec<ComparisonStep<S>>,
    },
    Construction(Span, Construct<S>),
    Application(Span, Box<Self>, Apply<S>),
}

#[derive(Clone, Debug)]
pub enum Construct<S> {
    /// wraps an expression
    Then(Box<Expression<S>>),
    Send(Span, Box<Expression<S>>, Box<Self>),
    Receive(Span, Pattern<S>, Box<Self>, Vec<TypeParameter>),
    /// constructs an either type
    Signal(Span, LocalName, Box<Self>),
    /// constructs a choice type
    Case(Span, ConstructBranches<S>, Option<Box<ConstructBranch<S>>>),
    /// ! (unit)
    Break(Span),
    Begin {
        span: Span,
        unfounded: bool,
        label: Option<LocalName>,
        then: Box<Self>,
    },
    Loop(Span, Option<LocalName>),
    SendType(Span, Type<S>, Box<Self>),
    ReceiveType(Span, TypeParameter, Box<Self>),
}

#[derive(Clone, Debug)]
pub struct ConstructBranches<S>(pub BTreeMap<LocalName, ConstructBranch<S>>);

#[derive(Clone, Debug)]
pub enum ConstructBranch<S> {
    Then(Span, Expression<S>),
    Receive(Span, Pattern<S>, Box<Self>, Vec<TypeParameter>),
    ReceiveType(Span, TypeParameter, Box<Self>),
}

#[derive(Clone, Debug)]
pub enum Apply<S> {
    Noop(Span),
    Send(Span, Box<Expression<S>>, Box<Self>),
    Signal(Span, LocalName, Box<Self>),
    Case(Span, ApplyBranches<S>, Option<Box<ApplyBranch<S>>>),
    Begin {
        span: Span,
        unfounded: bool,
        label: Option<LocalName>,
        then: Box<Self>,
    },
    Loop(Span, Option<LocalName>),
    SendType(Span, Type<S>, Box<Self>),
    Try(Span, Option<LocalName>, Box<Self>),
    Default(Span, Box<Expression<S>>, Box<Self>),
    Pipe(Span, Box<Expression<S>>, Box<Self>),
}

#[derive(Clone, Debug)]
pub struct ApplyBranches<S>(pub BTreeMap<LocalName, ApplyBranch<S>>);

#[derive(Clone, Debug)]
pub enum ApplyBranch<S> {
    Then(Span, LocalName, Expression<S>),
    Receive(Span, Pattern<S>, Box<Self>, Vec<TypeParameter>),
    Continue(Span, Expression<S>),
    ReceiveType(Span, TypeParameter, Box<Self>),
    Try(Span, Option<LocalName>, Box<Self>),
    Default(Span, Box<Expression<S>>, Box<Self>),
}

// span doesn't include the "then" process
#[derive(Clone, Debug)]
pub enum Process<S> {
    Let {
        span: Span,
        pattern: Pattern<S>,
        value: Box<Expression<S>>,
        then: Box<Self>,
    },
    Poll {
        span: Span,
        label: Option<LocalName>,
        clients: Vec<Expression<S>>,
        name: LocalName,
        then: Box<Process<S>>,
        else_: Box<Process<S>>,
    },
    Repoll {
        span: Span,
        label: Option<LocalName>,
        clients: Vec<Expression<S>>,
        name: LocalName,
        then: Box<Process<S>>,
        else_: Box<Process<S>>,
    },
    Submit {
        span: Span,
        label: Option<LocalName>,
        values: Vec<Expression<S>>,
    },
    Catch {
        span: Span,
        label: Option<LocalName>,
        pattern: Pattern<S>,
        block: Box<Self>,
        then: Box<Self>,
    },
    Throw(Span, Option<LocalName>, Box<Expression<S>>),
    If {
        span: Span,
        branches: Vec<(Condition<S>, Process<S>)>,
        else_: Option<Box<Process<S>>>,
        then: Option<Box<Process<S>>>,
    },
    GlobalCommand(Span, GlobalName<S>, Command<S>),
    Command(Span, LocalName, Command<S>),
    Fallthrough(Span),
}

#[derive(Clone, Debug)]
pub enum Command<S> {
    Then(Box<Process<S>>),
    Link(Span, Box<Expression<S>>),
    Send(Span, Expression<S>, Box<Self>),
    Receive(Span, Pattern<S>, Box<Self>, Vec<TypeParameter>),
    Signal(Span, LocalName, Box<Self>),
    Case(
        Span,
        CommandBranches<S>,
        Option<Box<CommandBranch<S>>>,
        Option<Box<Process<S>>>,
    ),
    Break(Span),
    Continue(Span, Box<Process<S>>),
    Begin {
        span: Span,
        unfounded: bool,
        label: Option<LocalName>,
        then: Box<Self>,
    },
    Loop(Span, Option<LocalName>),
    SendType(Span, Type<S>, Box<Self>),
    ReceiveType(Span, TypeParameter, Box<Self>),
    Try(Span, Option<LocalName>, Box<Self>),
    Default(Span, Box<Expression<S>>, Box<Self>),
    Pipe(Span, Box<Expression<S>>, Box<Self>),
}

#[derive(Clone, Debug)]
pub struct CommandBranches<S>(pub BTreeMap<LocalName, CommandBranch<S>>);

#[derive(Clone, Debug)]
pub enum CommandBranch<S> {
    Then(Span, Process<S>),
    BindThen(Span, LocalName, Process<S>),
    Receive(Span, Pattern<S>, Box<Self>, Vec<TypeParameter>),
    Continue(Span, Process<S>),
    ReceiveType(Span, TypeParameter, Box<Self>),
    Try(Span, Option<LocalName>, Box<Self>),
    Default(Span, Box<Expression<S>>, Box<Self>),
}

impl Hash for LocalName {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.string.hash(state);
    }
}
impl PartialEq for LocalName {
    fn eq(&self, other: &Self) -> bool {
        self.string == other.string
    }
}
impl Eq for LocalName {}
impl PartialOrd for LocalName {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.string.partial_cmp(&other.string)
    }
}
impl Ord for LocalName {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.string.cmp(&other.string)
    }
}
impl Display for LocalName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.string)
    }
}

impl Display for Resolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Resolved::Path {
                package,
                directories,
                module,
            } => {
                let package = match package {
                    ResolvedPackageRef::Local => None,
                    ResolvedPackageRef::Dependency(name) => Some(format!("@{name}")),
                };
                match (&package, directories.is_empty()) {
                    (None, true) => write!(f, "{module}"),
                    (None, false) => write!(f, "{}/{}", directories.join("/"), module),
                    (Some(package), true) => write!(f, "{package}/{module}"),
                    (Some(package), false) => {
                        write!(f, "{package}/{}/{}", directories.join("/"), module)
                    }
                }
            }
            Resolved::BuiltinOperator(BuiltinOperatorModule::Data) => write!(f, "<builtin-data>"),
            Resolved::BuiltinOperator(BuiltinOperatorModule::Number) => {
                write!(f, "<builtin-number>")
            }
            Resolved::BuiltinOperator(BuiltinOperatorModule::String) => {
                write!(f, "<builtin-string>")
            }
        }
    }
}

impl Display for Unresolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unresolved::Path { qualifier } => match qualifier {
                Some(qualifier) => write!(f, "{qualifier}"),
                None => Ok(()),
            },
            Unresolved::BuiltinOperator(BuiltinOperatorModule::Data) => {
                write!(f, "<builtin-data>")
            }
            Unresolved::BuiltinOperator(BuiltinOperatorModule::Number) => {
                write!(f, "<builtin-number>")
            }
            Unresolved::BuiltinOperator(BuiltinOperatorModule::String) => {
                write!(f, "<builtin-string>")
            }
        }
    }
}

impl Display for Universal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.package.is_regular() {
            write!(f, "\"{}\"", self.package.name())?;
        } else {
            write!(f, "@{}", self.package.name())?;
        }

        write!(f, "/")?;
        if self.directories.is_empty() {
            write!(f, "{}", self.module)
        } else {
            write!(f, "{}/{}", self.directories.join("/"), self.module)
        }
    }
}

impl<S: Display> Display for GlobalName<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let module = self.module.to_string();
        if module.is_empty() {
            write!(f, "{}", self.primary)
        } else {
            write!(f, "{module}.{}", self.primary)
        }
    }
}

impl<S: Hash> Hash for GlobalName<S> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.module.hash(state);
        self.primary.hash(state);
    }
}
impl<S: PartialEq> PartialEq for GlobalName<S> {
    fn eq(&self, other: &Self) -> bool {
        (&self.module, &self.primary) == (&other.module, &other.primary)
    }
}
impl<S: Eq> Eq for GlobalName<S> {}
impl<S: PartialOrd> PartialOrd for GlobalName<S> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        (&self.module, &self.primary).partial_cmp(&(&other.module, &other.primary))
    }
}
impl<S: Ord> Ord for GlobalName<S> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (&self.module, &self.primary).cmp(&(&other.module, &other.primary))
    }
}
#[derive(Clone, Debug)]
pub enum CompileError {
    MustEndProcess(Span),
    UnreachableCode(Span),
    NoMatchingCatch(Span),
    MatchingCatchDisabled(Span, CatchDisabledReason),
    NoSuchPollPoint(Span, Option<LocalName>),
}

#[derive(Clone, Debug)]
pub enum CatchDisabledReason {
    DifferentProcess,
    ValuePartiallyConstructed,
}

impl Spanning for CompileError {
    fn span(&self) -> Span {
        match self {
            Self::MustEndProcess(span) => span.clone(),
            Self::UnreachableCode(span) => span.clone(),
            Self::NoMatchingCatch(span) => span.clone(),
            Self::MatchingCatchDisabled(span, _) => span.clone(),
            Self::NoSuchPollPoint(span, _) => span.clone(),
        }
    }
}

impl CompileError {
    pub fn to_report(&self, source_code: Arc<str>) -> miette::Report {
        let mk_report = |span: &Span, msg: &'static str| {
            let labels = labels_from_span(&source_code, span);
            let code: Arc<str> = if labels.is_empty() {
                "<UI>".into()
            } else {
                Arc::clone(&source_code)
            };
            miette::miette! { labels = labels, "{}", msg }.with_source_code(code)
        };
        let mk_report_owned = |span: &Span, msg: String| {
            let labels = labels_from_span(&source_code, span);
            let code: Arc<str> = if labels.is_empty() {
                "<UI>".into()
            } else {
                Arc::clone(&source_code)
            };
            miette::miette! { labels = labels, "{}", msg }.with_source_code(code)
        };

        match self {
            Self::MustEndProcess(span) => mk_report(span, "This process must end."),
            Self::UnreachableCode(span) => mk_report(span, "Unreachable code."),
            Self::NoMatchingCatch(span) => mk_report(span, "No matching `catch` block defined."),
            Self::MatchingCatchDisabled(span, CatchDisabledReason::DifferentProcess) => {
                mk_report(span, "Matching `catch` is in a different process.")
            }
            Self::MatchingCatchDisabled(span, CatchDisabledReason::ValuePartiallyConstructed) => {
                mk_report(
                    span,
                    "The expression the matching `catch` would return from has its result already partially constructed.",
                )
            }
            Self::NoSuchPollPoint(span, None) => {
                mk_report(span, "No unlabeled `poll`/`repoll` point is in scope here.")
            }
            Self::NoSuchPollPoint(span, Some(label)) => mk_report_owned(
                span,
                format!("No such `poll@...`/`repoll@...` label `@{label}` is in scope here."),
            ),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Context {
    passes: Passes,
    original_object_name: Option<LocalName>,
}

#[derive(Clone, Debug)]
pub(crate) struct Passes {
    next_block_index: usize,
    next_poll_index: usize,
    fallthrough: Option<Pass>,
    fallthrough_stash: Vec<Option<Pass>>,
    catch: HashMap<Option<LocalName>, Pass>,
    catch_stash: HashMap<Option<LocalName>, Vec<Option<Pass>>>,
    poll: Option<PollScope>,
    poll_stash: Vec<Option<PollScope>>,
}

#[derive(Clone, Debug)]
struct PollPoint {
    label: Option<ArcStr>,
    point: LocalName,
}

#[derive(Clone, Debug)]
struct PollScope {
    driver: LocalName,
    points: Vec<PollPoint>,
}

#[derive(Clone, Debug)]
struct Pass {
    block_index: usize,
    used: bool,
    // Stack of reasons that currently disable this catch (LIFO).
    disabled_reasons: Vec<CatchDisabledReason>,
}

enum CommandLoweringFrame {
    Step(process::Step<(), Unresolved>),
    Receive {
        span: Span,
        pattern: Pattern<Unresolved>,
        vars: Vec<TypeParameter>,
        original_object_name: Option<LocalName>,
    },
    Try {
        span: Span,
        catch_block: Arc<process::Process<(), Unresolved>>,
    },
    Default {
        span: Span,
        expression: Arc<process::Expression<(), Unresolved>>,
    },
    Pipe {
        span: Span,
        function: Arc<process::Expression<(), Unresolved>>,
    },
}

impl Context {
    pub(crate) fn new() -> Self {
        Self {
            passes: Passes::new(),
            original_object_name: None,
        }
    }

    pub(crate) fn restore_object_name(
        &mut self,
        name: Option<LocalName>,
        process: Arc<process::Process<(), Unresolved>>,
    ) -> Arc<process::Process<(), Unresolved>> {
        match name {
            None => process,
            Some(original) => process::Process::let_step(
                original.span.clone(),
                original.clone(),
                None,
                (),
                Arc::new(process::Expression::Variable(
                    original.span.clone(),
                    LocalName::subject(),
                    (),
                    VariableUsage::Unknown,
                )),
                process,
            ),
        }
    }

    fn get_block_index(&mut self) -> usize {
        self.passes.get_block_index()
    }

    fn get_poll_index(&mut self) -> usize {
        self.passes.get_poll_index()
    }

    fn fresh_infix_temp(&mut self, span: Span) -> LocalName {
        LocalName {
            span,
            string: arcstr::format!("#infix{}", self.get_block_index()),
        }
    }

    fn operator_global(
        span: &Span,
        module: BuiltinOperatorModule,
        primary: &'static str,
    ) -> Expression<Unresolved> {
        Expression::Global(
            span.clone(),
            GlobalName::builtin_operator(span.clone(), module, primary),
        )
    }

    fn operator_local_name(span: &Span, name: &'static str) -> LocalName {
        LocalName {
            span: span.clone(),
            string: ArcStr::from(name),
        }
    }

    fn expression_to_condition(expr: Expression<Unresolved>) -> Condition<Unresolved> {
        match expr {
            Expression::Condition(_, condition) => *condition,
            Expression::Grouped(_, inner) => Self::expression_to_condition(*inner),
            other => {
                let span = other.span();
                Condition::Bool(span, Box::new(other))
            }
        }
    }

    fn wrap_condition_expression(condition: Condition<Unresolved>) -> Expression<Unresolved> {
        let span = condition.span();
        Expression::Condition(span, Box::new(condition))
    }

    fn apply_expression(
        span: &Span,
        function: Expression<Unresolved>,
        argument: Expression<Unresolved>,
    ) -> Expression<Unresolved> {
        Expression::Application(
            span.clone(),
            Box::new(function),
            Apply::Send(
                span.clone(),
                Box::new(argument),
                Box::new(Apply::Noop(span.clone())),
            ),
        )
    }

    fn pair_expression(
        span: &Span,
        left: Expression<Unresolved>,
        right: Expression<Unresolved>,
    ) -> Expression<Unresolved> {
        Expression::Construction(
            span.clone(),
            Construct::Send(
                span.clone(),
                Box::new(left),
                Box::new(Construct::Then(Box::new(right))),
            ),
        )
    }

    fn and_expression(
        left: Expression<Unresolved>,
        right: Expression<Unresolved>,
    ) -> Expression<Unresolved> {
        let left_span = left.span();
        let right_span = right.span();
        Self::wrap_condition_expression(Condition::And(
            left_span.join(right_span),
            Box::new(Self::expression_to_condition(left)),
            Box::new(Self::expression_to_condition(right)),
        ))
    }

    fn desugar_neg_expression(
        op_span: &Span,
        expr: Expression<Unresolved>,
    ) -> Expression<Unresolved> {
        Self::apply_expression(
            op_span,
            Self::operator_global(op_span, BuiltinOperatorModule::Number, "Neg"),
            expr,
        )
    }

    fn desugar_arithmetic_expression(
        op_span: &Span,
        op: ArithmeticOperator,
        left: Expression<Unresolved>,
        right: Expression<Unresolved>,
    ) -> Expression<Unresolved> {
        let builtin = match op {
            ArithmeticOperator::Add => "Add",
            ArithmeticOperator::Sub => "Sub",
            ArithmeticOperator::Mul => "Mul",
            ArithmeticOperator::Div => "Div",
        };
        Self::apply_expression(
            op_span,
            Self::operator_global(op_span, BuiltinOperatorModule::Number, builtin),
            Self::pair_expression(op_span, left, right),
        )
    }

    fn desugar_comparison_expression(
        op_span: &Span,
        op: ComparisonOperator,
        left: Expression<Unresolved>,
        right: Expression<Unresolved>,
    ) -> Expression<Unresolved> {
        let (variant, negate) = match op {
            ComparisonOperator::Less => ("less", false),
            ComparisonOperator::Greater => ("greater", false),
            ComparisonOperator::LessOrEqual => ("greater", true),
            ComparisonOperator::GreaterOrEqual => ("less", true),
            ComparisonOperator::Equal => ("equal", false),
            ComparisonOperator::NotEqual => ("equal", true),
        };

        let compare = Self::apply_expression(
            op_span,
            Self::operator_global(op_span, BuiltinOperatorModule::Data, "Compare"),
            Self::pair_expression(op_span, left, right),
        );
        let condition = Condition::Is {
            span: Span::None,
            value: compare,
            variant: Self::operator_local_name(&Span::None, variant),
            pattern: Pattern::Continue(Span::None),
        };

        if negate {
            Self::wrap_condition_expression(Condition::Not(Span::None, Box::new(condition)))
        } else {
            Self::wrap_condition_expression(condition)
        }
    }

    fn desugar_comparison_chain_expression(
        &mut self,
        first: Expression<Unresolved>,
        rest: &[ComparisonStep<Unresolved>],
    ) -> Expression<Unresolved> {
        match rest {
            [] => first,
            [step] => Self::desugar_comparison_expression(
                &step.op_span,
                step.op,
                first,
                step.expr.clone(),
            ),
            _ => {
                let temporaries = rest
                    .iter()
                    .take(rest.len() - 1)
                    .map(|step| {
                        let span = step.expr.span();
                        (self.fresh_infix_temp(span), step.expr.clone())
                    })
                    .collect::<Vec<_>>();

                let mut left = first;
                let mut comparisons = Vec::with_capacity(rest.len());

                for (index, step) in rest.iter().enumerate() {
                    let right = if let Some((name, _binding)) = temporaries.get(index) {
                        Expression::Variable(name.span.clone(), name.clone())
                    } else {
                        step.expr.clone()
                    };
                    let next_left = right.clone();
                    comparisons.push(Self::desugar_comparison_expression(
                        &step.op_span,
                        step.op,
                        left,
                        right,
                    ));
                    left = next_left;
                }

                let mut combined = comparisons
                    .into_iter()
                    .reduce(Self::and_expression)
                    .expect("comparison chains always contain at least one comparison");

                for (name, binding) in temporaries.into_iter().rev() {
                    let binding_span = binding.span();
                    combined = Expression::Let {
                        span: binding_span.join(combined.span()),
                        pattern: Pattern::Name(binding_span.clone(), name, None),
                        expression: Box::new(binding),
                        then: Box::new(combined),
                    };
                }

                combined
            }
        }
    }

    fn desugar_template_expression(parts: &[TemplatePart<Unresolved>]) -> Expression<Unresolved> {
        let mut items = Vec::new();
        let mut has_interpolation = false;

        for part in parts {
            match part {
                TemplatePart::Literal(value) if value.is_empty() => {}
                TemplatePart::Literal(value) => {
                    items.push(Expression::Primitive(
                        Span::None,
                        Primitive::String(ParString::from_owner(value.clone())),
                    ));
                }
                TemplatePart::StringExpr(expr) => {
                    has_interpolation = true;
                    items.push(expr.clone());
                }
                TemplatePart::DataExpr(expr) => {
                    has_interpolation = true;
                    items.push(Self::apply_expression(
                        &Span::None,
                        Self::operator_global(&Span::None, BuiltinOperatorModule::Data, "ToString"),
                        expr.clone(),
                    ));
                }
            }
        }

        match items.as_slice() {
            [] => Expression::Primitive(Span::None, Primitive::String(ParString::default())),
            [only] if !has_interpolation => only.clone(),
            _ => Self::apply_expression(
                &Span::None,
                Self::operator_global(&Span::None, BuiltinOperatorModule::String, "Concat"),
                Expression::List(Span::None, items),
            ),
        }
    }

    fn without_fallthrough(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<Arc<process::Process<(), Unresolved>>, CompileError>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        self.passes
            .fallthrough_stash
            .push(self.passes.fallthrough.take());
        let result = f(self);
        self.passes.fallthrough = self.passes.fallthrough_stash.pop().unwrap();
        result
    }

    fn with_fallthrough(
        &mut self,
        body: Arc<process::Process<(), Unresolved>>,
        f: impl FnOnce(&mut Self) -> Result<Arc<process::Process<(), Unresolved>>, CompileError>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        self.passes
            .fallthrough_stash
            .push(self.passes.fallthrough.take());

        let block_index = self.get_block_index();
        self.passes.fallthrough = Some(Pass::new(block_index));
        let process = f(self)?;
        if !self.passes.fallthrough.take().unwrap().used {
            return Err(CompileError::UnreachableCode(body.span()));
        }
        let result = process::Process::terminal(process::Terminator::Block(
            body.span(),
            block_index,
            body,
            process,
        ));

        self.passes.fallthrough = self.passes.fallthrough_stash.pop().unwrap();

        Ok(result)
    }

    fn expr_without_fallthrough(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<Arc<process::Expression<(), Unresolved>>, CompileError>,
    ) -> Result<Arc<process::Expression<(), Unresolved>>, CompileError> {
        self.passes
            .fallthrough_stash
            .push(self.passes.fallthrough.take());
        let result = f(self);
        self.passes.fallthrough = self.passes.fallthrough_stash.pop().unwrap();
        result
    }

    fn expr_with_fallthrough(
        &mut self,
        span: &Span,
        body: Arc<process::Process<(), Unresolved>>,
        f: impl FnOnce(&mut Self) -> Result<Arc<process::Expression<(), Unresolved>>, CompileError>,
    ) -> Result<Arc<process::Expression<(), Unresolved>>, CompileError> {
        Ok(Arc::new(process::Expression::Chan {
            span: span.clone(),
            captures: Captures::new(),
            chan_name: LocalName::result(),
            chan_annotation: None,
            chan_type: (),
            expr_type: (),
            process: self.with_fallthrough(body, |pass| {
                Ok(process::Process::do_terminal(
                    span.clone(),
                    LocalName::result(),
                    VariableUsage::Unknown,
                    (),
                    process::TerminalCommand::Link(f(pass)?),
                ))
            })?,
        }))
    }

    fn use_fallthrough(&mut self, span: &Span) -> Option<Arc<process::Process<(), Unresolved>>> {
        self.passes
            .fallthrough
            .as_mut()
            .map(|pass| pass.use_at(span))
    }

    fn with_poll<T>(
        &mut self,
        driver: LocalName,
        point: LocalName,
        label: &Option<LocalName>,
        f: impl FnOnce(&mut Self) -> Result<T, CompileError>,
    ) -> Result<T, CompileError> {
        self.passes.poll_stash.push(self.passes.poll.take());
        self.passes.poll = Some(PollScope {
            driver,
            points: vec![PollPoint {
                label: label.as_ref().map(|l| l.string.clone()),
                point,
            }],
        });
        let result = f(self);
        self.passes.poll = self.passes.poll_stash.pop().unwrap();
        result
    }

    fn with_repoll<T>(
        &mut self,
        point: LocalName,
        label: &Option<LocalName>,
        f: impl FnOnce(&mut Self) -> Result<T, CompileError>,
    ) -> Result<T, CompileError> {
        let Some(mut scope) = self.passes.poll.clone() else {
            return f(self);
        };
        scope.points.push(PollPoint {
            label: label.as_ref().map(|l| l.string.clone()),
            point,
        });
        self.passes.poll_stash.push(self.passes.poll.take());
        self.passes.poll = Some(scope);
        let result = f(self);
        self.passes.poll = self.passes.poll_stash.pop().unwrap();
        result
    }

    fn without_poll<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, CompileError>,
    ) -> Result<T, CompileError> {
        let outer = self.passes.poll_stash.last().cloned().unwrap_or(None);
        self.passes.poll_stash.push(self.passes.poll.take());
        self.passes.poll = outer;
        let result = f(self);
        self.passes.poll = self.passes.poll_stash.pop().unwrap();
        result
    }

    fn current_poll_driver(&self) -> Option<&LocalName> {
        self.passes.poll.as_ref().map(|p| &p.driver)
    }

    fn resolve_poll_point_by_label(&self, label: &LocalName) -> Option<&LocalName> {
        let label_str = &label.string;
        self.passes.poll.as_ref().and_then(|p| {
            p.points
                .iter()
                .rev()
                .find(|pp| pp.label.as_ref() == Some(label_str))
                .map(|pp| &pp.point)
        })
    }

    fn resolve_poll_point(&self, label: &Option<LocalName>) -> Option<&LocalName> {
        match label.as_ref() {
            Some(label) => self.resolve_poll_point_by_label(label),
            None => self.passes.poll.as_ref().and_then(|p| {
                p.points
                    .iter()
                    .rev()
                    .find(|pp| pp.label.is_none())
                    .map(|pp| &pp.point)
            }),
        }
    }

    fn make_poll_process(
        &mut self,
        span: &Span,
        kind: process::PollKind,
        label: &Option<LocalName>,
        clients: Vec<Arc<process::Expression<(), Unresolved>>>,
        name: LocalName,
        then: impl FnOnce(&mut Self) -> Result<Arc<process::Process<(), Unresolved>>, CompileError>,
        else_: impl FnOnce(&mut Self) -> Result<Arc<process::Process<(), Unresolved>>, CompileError>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        let id = self.get_poll_index();
        let point = LocalName {
            span: span.clone(),
            string: ArcStr::from(format!("@{id}")),
        };

        let driver = match kind {
            process::PollKind::Poll => LocalName {
                span: span.clone(),
                string: ArcStr::from(format!("#pool{id}")),
            },
            process::PollKind::Repoll => self
                .current_poll_driver()
                .cloned()
                .unwrap_or_else(LocalName::invalid),
        };

        let build = |pass: &mut Self| {
            let then = then(pass)?;
            let else_ = pass.without_poll(|pass| else_(pass))?;
            Ok(process::Process::terminal(process::Terminator::Poll {
                span: span.clone(),
                kind: kind.clone(),
                driver: driver.clone(),
                point: point.clone(),
                clients: clients,
                name: name,
                name_typ: (),
                captures: Captures::new(),
                then,
                else_,
            }))
        };

        match kind {
            process::PollKind::Poll => {
                self.with_poll(driver.clone(), point.clone(), label, |pass| build(pass))
            }
            process::PollKind::Repoll => self.with_repoll(point.clone(), label, |pass| build(pass)),
        }
    }

    fn make_submit_process(
        &mut self,
        span: &Span,
        label: &Option<LocalName>,
        values: Vec<Arc<process::Expression<(), Unresolved>>>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        let driver = self
            .current_poll_driver()
            .cloned()
            .unwrap_or_else(LocalName::invalid);
        let point = match self.passes.poll.as_ref() {
            None => LocalName::invalid(),
            Some(_) => self.resolve_poll_point(label).cloned().ok_or_else(|| {
                let err_span = label
                    .as_ref()
                    .map(|l| l.span())
                    .unwrap_or_else(|| span.clone());
                CompileError::NoSuchPollPoint(err_span, label.clone())
            })?,
        };

        Ok(process::Process::terminal(process::Terminator::Submit {
            span: span.clone(),
            driver,
            point,
            values,
            captures: Captures::new(),
        }))
    }

    fn with_catch(
        &mut self,
        label: Option<LocalName>,
        body: Arc<process::Process<(), Unresolved>>,
        f: impl FnOnce(&mut Self) -> Result<Arc<process::Process<(), Unresolved>>, CompileError>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        self.passes
            .catch_stash
            .entry(label.clone())
            .or_default()
            .push(self.passes.catch.remove(&label));
        let block_index = self.get_block_index();
        self.passes
            .catch
            .insert(label.clone(), Pass::new(block_index));

        let result = f(self);

        let current = self.passes.catch.remove(&label);
        let stashed = self
            .passes
            .catch_stash
            .entry(label.clone())
            .or_default()
            .pop()
            .unwrap();
        if let Some(stashed) = stashed {
            self.passes.catch.insert(label, stashed);
        }
        let process = result?;
        if !current.unwrap().used {
            return Err(CompileError::UnreachableCode(body.span()));
        }
        Ok(process::Process::terminal(process::Terminator::Block(
            body.span(),
            block_index,
            body,
            process,
        )))
    }

    fn expr_with_catch(
        &mut self,
        span: &Span,
        label: Option<LocalName>,
        body: Arc<process::Process<(), Unresolved>>,
        f: impl FnOnce(&mut Self) -> Result<Arc<process::Expression<(), Unresolved>>, CompileError>,
    ) -> Result<Arc<process::Expression<(), Unresolved>>, CompileError> {
        Ok(Arc::new(process::Expression::Chan {
            span: span.clone(),
            captures: Captures::new(),
            chan_name: LocalName::result(),
            chan_annotation: None,
            chan_type: (),
            expr_type: (),
            process: self.with_catch(label, body, |pass| {
                Ok(process::Process::do_terminal(
                    span.clone(),
                    LocalName::result(),
                    VariableUsage::Unknown,
                    (),
                    process::TerminalCommand::Link(f(pass)?),
                ))
            })?,
        }))
    }

    fn use_catch(
        &mut self,
        span: &Span,
        label: &Option<LocalName>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        match self.passes.catch.get_mut(label) {
            Some(pass) => {
                if let Some(reason) = pass.disabled_reasons.last().cloned() {
                    return Err(CompileError::MatchingCatchDisabled(span.clone(), reason));
                }
                Ok(pass.use_at(span))
            }
            None => Err(CompileError::NoMatchingCatch(span.clone())),
        }
    }

    fn disable_catches(&mut self, reason: CatchDisabledReason) -> &mut Self {
        for pass in self.passes.catch.values_mut() {
            pass.disabled_reasons.push(reason.clone());
        }
        self
    }

    fn enable_catches(&mut self) -> &mut Self {
        for pass in self.passes.catch.values_mut() {
            if !pass.disabled_reasons.is_empty() {
                pass.disabled_reasons.pop().unwrap();
            }
        }
        self
    }

    pub(crate) fn compile_pattern_let(
        &mut self,
        pattern: &Pattern<Unresolved>,
        span: &Span,
        expression: Arc<process::Expression<(), Unresolved>>,
        process: Arc<process::Process<(), Unresolved>>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        if let Pattern::Name(_, name, annotation) = pattern {
            return Ok(process::Process::let_step(
                span.clone(),
                name.clone(),
                annotation.clone(),
                (),
                expression,
                process,
            ));
        }
        let then = self.compile_pattern_helper(pattern, 0, process)?;
        Ok(process::Process::let_step(
            span.clone(),
            LocalName::match_(0),
            pattern.annotation(),
            (),
            expression,
            then,
        ))
    }

    pub(crate) fn compile_pattern_chan(
        &mut self,
        pattern: &Pattern<Unresolved>,
        span: &Span,
        process: Arc<process::Process<(), Unresolved>>,
    ) -> Result<Arc<process::Expression<(), Unresolved>>, CompileError> {
        if let Pattern::Name(_, name, annotation) = pattern {
            return Ok(Arc::new(process::Expression::Chan {
                span: span.clone(),
                captures: Captures::new(),
                chan_name: name.clone(),
                chan_annotation: annotation.clone(),
                chan_type: (),
                expr_type: (),
                process,
            }));
        }
        Ok(Arc::new(process::Expression::Chan {
            span: span.clone(),
            captures: Captures::new(),
            chan_name: LocalName::match_(0),
            chan_annotation: None,
            chan_type: (),
            expr_type: (),
            process: self.compile_pattern_helper(pattern, 0, process)?,
        }))
    }

    pub(crate) fn compile_pattern_catch_block(
        &mut self,
        pattern: &Pattern<Unresolved>,
        span: &Span,
        block: Arc<process::Process<(), Unresolved>>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        if let Pattern::Name(_, name, annotation) = pattern {
            return Ok(process::Process::let_step(
                span.clone(),
                name.clone(),
                annotation.clone(),
                (),
                Arc::new(process::Expression::Variable(
                    span.clone(),
                    LocalName::error(),
                    (),
                    VariableUsage::Unknown,
                )),
                block,
            ));
        }
        let then = self.compile_pattern_helper(pattern, 0, block)?;
        Ok(process::Process::let_step(
            span.clone(),
            LocalName::match_(0),
            None,
            (),
            Arc::new(process::Expression::Variable(
                span.clone(),
                LocalName::error(),
                (),
                VariableUsage::Unknown,
            )),
            then,
        ))
    }

    pub(crate) fn compile_pattern_receive(
        &mut self,
        pattern: &Pattern<Unresolved>,
        level: usize,
        span: &Span,
        subject: &LocalName,
        process: Arc<process::Process<(), Unresolved>>,
        vars: Vec<TypeParameter>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        if let Pattern::Name(_, name, annotation) = pattern {
            return Ok(process::Process::do_step(
                span.clone(),
                subject.clone(),
                VariableUsage::Unknown,
                (),
                process::Command::Receive(name.clone(), annotation.clone(), (), vars),
                process,
            ));
        }
        let then = self.compile_pattern_helper(pattern, level, process)?;
        Ok(process::Process::do_step(
            span.clone(),
            subject.clone(),
            VariableUsage::Unknown,
            (),
            process::Command::Receive(LocalName::match_(level), pattern.annotation(), (), vars),
            then,
        ))
    }

    fn compile_pattern_helper(
        &mut self,
        pattern: &Pattern<Unresolved>,
        level: usize,
        process: Arc<process::Process<(), Unresolved>>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        match pattern {
            Pattern::Name(span, name, annotation) => Ok(process::Process::let_step(
                span.clone(),
                name.clone(),
                annotation.clone(),
                (),
                Arc::new(process::Expression::Variable(
                    span.clone(),
                    LocalName::match_(level),
                    (),
                    VariableUsage::Unknown,
                )),
                process,
            )),

            Pattern::Receive(span, first, rest, vars) => {
                let then_process = self.compile_pattern_helper(rest, level, process)?;
                self.compile_pattern_receive(
                    first,
                    level + 1,
                    span,
                    &LocalName::match_(level),
                    then_process,
                    vars.clone(),
                )
            }

            Pattern::Continue(span) => Ok(process::Process::do_step(
                span.clone(),
                LocalName::match_(level),
                VariableUsage::Unknown,
                (),
                process::Command::Continue,
                process,
            )),

            Pattern::ReceiveType(span, parameter, rest) => {
                let then = self.compile_pattern_helper(rest, level, process)?;
                Ok(process::Process::do_step(
                    span.clone(),
                    LocalName::match_(level),
                    VariableUsage::Unknown,
                    (),
                    process::Command::ReceiveType(parameter.clone()),
                    then,
                ))
            }

            Pattern::Try(span, label, rest) => {
                let catch_block = self.use_catch(span, label)?;
                let catch_block = if let Some(original) = &self.original_object_name {
                    process::Process::let_step(
                        original.span.clone(),
                        original.clone(),
                        None,
                        (),
                        Arc::new(process::Expression::Variable(
                            original.span.clone(),
                            LocalName::subject(),
                            (),
                            VariableUsage::Unknown,
                        )),
                        catch_block,
                    )
                } else {
                    catch_block
                };
                let then_process = self.compile_pattern_helper(rest, level, process)?;
                Ok(self.compile_try(span, LocalName::match_(level), catch_block, then_process))
            }

            Pattern::Default(span, expr, rest) => {
                let default_expr = self.compile_expression(expr)?;
                let ok_process = self.compile_pattern_helper(rest, level, process)?;
                Ok(self.compile_default(span, LocalName::match_(level), default_expr, ok_process))
            }
        }
    }

    pub(crate) fn compile_expression(
        &mut self,
        expr: &Expression<Unresolved>,
    ) -> Result<Arc<process::Expression<(), Unresolved>>, CompileError> {
        let original_name = std::mem::take(&mut self.original_object_name);
        let res = Ok(match expr {
            Expression::Primitive(span, value) => Arc::new(process::Expression::Primitive(
                span.clone(),
                value.clone(),
                (),
            )),

            Expression::Template { parts, .. } => {
                let desugared = Self::desugar_template_expression(parts);
                self.compile_expression(&desugared)?
            }

            Expression::List(span, items) => {
                let mut process = process::Process::do_terminal(
                    span.clone(),
                    LocalName::result(),
                    VariableUsage::Unknown,
                    (),
                    process::TerminalCommand::Break,
                );
                process = process::Process::do_step(
                    span.clone(),
                    LocalName::result(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::Signal(LocalName {
                        span: span.clone(),
                        string: literal!("end"),
                    }),
                    process,
                );
                for item in items.iter().rev() {
                    let span = item.span();
                    process = process::Process::do_step(
                        span.clone(),
                        LocalName::result(),
                        VariableUsage::Unknown,
                        (),
                        process::Command::Send(self.compile_expression(item)?),
                        process,
                    );
                    process = process::Process::do_step(
                        span.clone(),
                        LocalName::result(),
                        VariableUsage::Unknown,
                        (),
                        process::Command::Signal(LocalName {
                            span,
                            string: literal!("item"),
                        }),
                        process,
                    );
                }
                Arc::new(process::Expression::Chan {
                    span: span.clone(),
                    captures: Captures::new(),
                    chan_name: LocalName::result(),
                    chan_annotation: None,
                    chan_type: (),
                    expr_type: (),
                    process,
                })
            }

            Expression::Global(span, name) => {
                Arc::new(process::Expression::Global(span.clone(), name.clone(), ()))
            }

            Expression::Variable(span, name) => Arc::new(process::Expression::Variable(
                span.clone(),
                name.clone(),
                (),
                VariableUsage::Unknown,
            )),

            Expression::Poll {
                span,
                label,
                clients,
                name,
                then,
                else_,
            } => {
                let clients: Vec<_> = clients
                    .iter()
                    .map(|e| self.compile_expression(e))
                    .collect::<Result<_, _>>()?;
                let process = self.make_poll_process(
                    span,
                    process::PollKind::Poll,
                    label,
                    clients,
                    name.clone(),
                    |pass| pass.compile_process(&link_process_from_expr(then)),
                    |pass| pass.compile_process(&link_process_from_expr(else_)),
                )?;

                Arc::new(process::Expression::Chan {
                    span: span.clone(),
                    captures: Captures::new(),
                    chan_name: LocalName::result(),
                    chan_annotation: None,
                    chan_type: (),
                    expr_type: (),
                    process,
                })
            }

            Expression::Repoll {
                span,
                label,
                clients,
                name,
                then,
                else_,
            } => {
                let clients: Result<Vec<_>, _> =
                    clients.iter().map(|e| self.compile_expression(e)).collect();
                let clients = clients?;
                let process = self.make_poll_process(
                    span,
                    process::PollKind::Repoll,
                    label,
                    clients,
                    name.clone(),
                    |pass| pass.compile_process(&link_process_from_expr(then)),
                    |pass| pass.compile_process(&link_process_from_expr(else_)),
                )?;

                Arc::new(process::Expression::Chan {
                    span: span.clone(),
                    captures: Captures::new(),
                    chan_name: LocalName::result(),
                    chan_annotation: None,
                    chan_type: (),
                    expr_type: (),
                    process,
                })
            }

            Expression::Submit {
                span,
                label,
                values,
            } => {
                let values: Result<Vec<_>, _> =
                    values.iter().map(|e| self.compile_expression(e)).collect();
                let values = values?;
                let process = self.make_submit_process(span, label, values)?;
                Arc::new(process::Expression::Chan {
                    span: span.clone(),
                    captures: Captures::new(),
                    chan_name: LocalName::result(),
                    chan_annotation: None,
                    chan_type: (),
                    expr_type: (),
                    process,
                })
            }

            Expression::Condition(span, condition) => {
                let make_bool = |variant| {
                    let end = process::Process::do_terminal(
                        span.clone(),
                        LocalName::result(),
                        VariableUsage::Unknown,
                        (),
                        process::TerminalCommand::Break,
                    );
                    process::Process::do_step(
                        span.clone(),
                        LocalName::result(),
                        VariableUsage::Unknown,
                        (),
                        process::Command::Signal(LocalName {
                            span: span.clone(),
                            string: ArcStr::from(variant),
                        }),
                        end,
                    )
                };
                let true_process = make_bool("true");
                let false_process = make_bool("false");
                let process = self.compile_condition_process(
                    condition.as_ref(),
                    true_process,
                    false_process,
                )?;
                Arc::new(process::Expression::Chan {
                    span: span.clone(),
                    captures: Captures::new(),
                    chan_name: LocalName::result(),
                    chan_annotation: None,
                    chan_type: (),
                    expr_type: (),
                    process,
                })
            }

            Expression::Arithmetic {
                op_span,
                op,
                left,
                right,
                ..
            } => {
                let desugared = Self::desugar_arithmetic_expression(
                    op_span,
                    *op,
                    left.as_ref().clone(),
                    right.as_ref().clone(),
                );
                self.compile_expression(&desugared)?
            }

            Expression::Neg { op_span, expr, .. } => {
                let desugared = Self::desugar_neg_expression(op_span, expr.as_ref().clone());
                self.compile_expression(&desugared)?
            }

            Expression::ComparisonChain { first, rest, .. } => {
                let desugared =
                    self.desugar_comparison_chain_expression(first.as_ref().clone(), rest);
                self.compile_expression(&desugared)?
            }

            Expression::Grouped(_, expression) => self.compile_expression(expression)?,

            Expression::TypeIn { span, typ, expr } => {
                let expression = self.compile_expression(expr)?;
                Arc::new(process::Expression::Chan {
                    span: span.clone(),
                    captures: Captures::new(),
                    chan_name: LocalName::result(),
                    chan_annotation: None,
                    chan_type: (),
                    expr_type: (),
                    process: process::Process::let_step(
                        span.clone(),
                        LocalName::object(),
                        Some(typ.clone()),
                        (),
                        expression,
                        process::Process::do_terminal(
                            span.clone(),
                            LocalName::result(),
                            VariableUsage::Unknown,
                            (),
                            process::TerminalCommand::Link(Arc::new(
                                process::Expression::Variable(
                                    span.clone(),
                                    LocalName::object(),
                                    (),
                                    VariableUsage::Unknown,
                                ),
                            )),
                        ),
                    ),
                })
            }

            Expression::Box(span, expression) => {
                let expression = self.compile_expression(expression)?;
                Arc::new(process::Expression::Box(
                    span.clone(),
                    Captures::new(),
                    expression,
                    (),
                ))
            }

            Expression::Let {
                span,
                pattern,
                expression,
                then: body,
            } => {
                self.disable_catches(CatchDisabledReason::DifferentProcess);
                let expression = self.compile_expression(expression)?;
                self.enable_catches();
                let body = self.compile_expression(body)?;
                Arc::new(process::Expression::Chan {
                    span: span.clone(),
                    captures: Captures::new(),
                    chan_name: LocalName::result(),
                    chan_annotation: None,
                    chan_type: (),
                    expr_type: (),
                    process: self.compile_pattern_let(
                        pattern,
                        span,
                        expression,
                        process::Process::do_terminal(
                            span.clone(),
                            LocalName::result(),
                            VariableUsage::Unknown,
                            (),
                            process::TerminalCommand::Link(body),
                        ),
                    )?,
                })
            }

            Expression::Catch {
                span,
                label,
                pattern,
                block,
                then,
            } => {
                let block = process::Process::do_terminal(
                    span.clone(),
                    LocalName::result(),
                    VariableUsage::Unknown,
                    (),
                    process::TerminalCommand::Link(self.compile_expression(block)?),
                );
                let block = self.compile_pattern_catch_block(pattern, span, block)?;
                self.expr_with_catch(span, label.clone(), block, |pass| {
                    pass.compile_expression(then)
                })?
            }

            Expression::Throw(span, label, expression) => {
                let catch_block = self.use_catch(span, label)?;
                self.expr_without_fallthrough(|pass| {
                    Ok(Arc::new(process::Expression::Chan {
                        span: span.clone(),
                        captures: Captures::new(),
                        chan_name: LocalName::result(),
                        chan_annotation: None,
                        chan_type: (),
                        expr_type: (),
                        process: process::Process::let_step(
                            span.clone(),
                            LocalName::error(),
                            None,
                            (),
                            pass.compile_expression(expression)?,
                            catch_block,
                        ),
                    }))
                })?
            }

            Expression::If {
                span,
                branches,
                else_,
            } => {
                let else_proc = match else_ {
                    Some(expr) => self.compile_process(&link_process_from_expr(expr))?,
                    None => {
                        process::Process::terminal(process::Terminator::Unreachable(span.clone()))
                    }
                };
                let compiled = self.compile_if_branches(branches, else_proc, |body, pass| {
                    pass.compile_process(&link_process_from_expr(body))
                })?;
                Arc::new(process::Expression::Chan {
                    span: span.clone(),
                    captures: Captures::new(),
                    chan_name: LocalName::result(),
                    chan_annotation: None,
                    chan_type: (),
                    expr_type: (),
                    process: compiled,
                })
            }

            Expression::Do {
                span,
                process,
                then: expression,
            } => {
                let expression = self.compile_expression(expression)?;
                self.expr_with_fallthrough(
                    span,
                    process::Process::do_terminal(
                        span.clone(),
                        LocalName::result(),
                        VariableUsage::Unknown,
                        (),
                        process::TerminalCommand::Link(expression),
                    ),
                    |pass| {
                        Ok(Arc::new(process::Expression::Chan {
                            span: span.clone(),
                            captures: Captures::new(),
                            chan_name: LocalName::result(),
                            chan_annotation: None,
                            chan_type: (),
                            expr_type: (),
                            process: pass.compile_process(process)?,
                        }))
                    },
                )?
            }

            Expression::Chan {
                span,
                pattern,
                process,
            } => {
                self.disable_catches(CatchDisabledReason::DifferentProcess);
                let proc = self.compile_process(process)?;
                self.enable_catches();
                self.compile_pattern_chan(pattern, span, proc)?
            }

            Expression::Construction(span, construct) => {
                self.disable_catches(CatchDisabledReason::ValuePartiallyConstructed);
                let process = self.compile_construct(construct)?;
                self.enable_catches();
                Arc::new(process::Expression::Chan {
                    span: span.clone(),
                    captures: Captures::new(),
                    chan_name: LocalName::result(),
                    chan_annotation: None,
                    chan_type: (),
                    expr_type: (),
                    process,
                })
            }

            Expression::Application(_, expr, Apply::Noop(_)) => self.compile_expression(expr)?,

            Expression::Application(span, expr, apply) => {
                let expr = self.compile_expression(expr)?;
                let process = self.compile_apply(apply)?;
                Arc::new(process::Expression::Chan {
                    span: span.clone(),
                    captures: Captures::new(),
                    chan_name: LocalName::result(),
                    chan_annotation: None,
                    chan_type: (),
                    expr_type: (),
                    process: process::Process::let_step(
                        span.clone(),
                        LocalName::object(),
                        None,
                        (),
                        expr,
                        process,
                    ),
                })
            }
        });
        let None = self.original_object_name else {
            unreachable!("original_object_name should be none after expression")
        };
        self.original_object_name = original_name;

        res
    }

    pub(crate) fn compile_construct(
        &mut self,
        construct: &Construct<Unresolved>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        Ok(match construct {
            Construct::Then(expression) => {
                let expression = self.compile_expression(expression)?;
                process::Process::do_terminal(
                    Span::None,
                    LocalName::result(),
                    VariableUsage::Unknown,
                    (),
                    process::TerminalCommand::Link(expression),
                )
            }

            Construct::Send(span, argument, construct) => {
                let argument = self.compile_expression(argument)?;
                let process = self.compile_construct(construct)?;
                process::Process::do_step(
                    span.clone(),
                    LocalName::result(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::Send(argument),
                    process,
                )
            }

            Construct::Receive(span, pattern, construct, vars) => {
                let process = self.compile_construct(construct)?;
                self.compile_pattern_receive(
                    pattern,
                    0,
                    span,
                    &LocalName::result(),
                    process,
                    vars.clone(),
                )?
            }

            Construct::Signal(span, chosen, construct) => {
                let process = self.compile_construct(construct)?;
                process::Process::do_step(
                    span.clone(),
                    LocalName::result(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::Signal(chosen.clone()),
                    process,
                )
            }

            Construct::Case(span, ConstructBranches(construct_branches), else_branch) => {
                let mut branches = Vec::new();
                let mut processes = Vec::new();
                for (branch_name, construct_branch) in construct_branches {
                    branches.push(branch_name.clone());
                    processes.push(self.compile_construct_branch(construct_branch)?);
                }
                let else_process = match else_branch {
                    Some(branch) => Some(self.compile_construct_branch(branch)?),
                    None => None,
                };
                let branches = Arc::from(branches);
                let processes = Box::from(processes);
                process::Process::do_terminal(
                    span.clone(),
                    LocalName::result(),
                    VariableUsage::Unknown,
                    (),
                    process::TerminalCommand::Case(branches, processes, else_process),
                )
            }

            Construct::Break(span) => process::Process::do_terminal(
                span.clone(),
                LocalName::result(),
                VariableUsage::Unknown,
                (),
                process::TerminalCommand::Break,
            ),

            Construct::Begin {
                span,
                unfounded,
                label,
                then: construct,
            } => {
                let process = self.compile_construct(construct)?;
                process::Process::do_terminal(
                    span.clone(),
                    LocalName::result(),
                    VariableUsage::Unknown,
                    (),
                    process::TerminalCommand::Begin {
                        unfounded: *unfounded,
                        label: label.clone(),
                        captures: Captures::new(),
                        body: process,
                    },
                )
            }

            Construct::Loop(span, label) => process::Process::do_terminal(
                span.clone(),
                LocalName::result(),
                VariableUsage::Unknown,
                (),
                process::TerminalCommand::Loop(
                    label.clone(),
                    LocalName::invalid(),
                    Captures::new(),
                ),
            ),

            Construct::SendType(span, argument, construct) => {
                let process = self.compile_construct(construct)?;
                process::Process::do_step(
                    span.clone(),
                    LocalName::result(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::SendType(argument.clone()),
                    process,
                )
            }

            Construct::ReceiveType(span, parameter, construct) => {
                let process = self.compile_construct(construct)?;
                process::Process::do_step(
                    span.clone(),
                    LocalName::result(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::ReceiveType(parameter.clone()),
                    process,
                )
            }
        })
    }

    pub(crate) fn compile_construct_branch(
        &mut self,
        branch: &ConstructBranch<Unresolved>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        Ok(match branch {
            ConstructBranch::Then(_, expression) => {
                let expression = self.compile_expression(expression)?;
                process::Process::do_terminal(
                    Span::None,
                    LocalName::result(),
                    VariableUsage::Unknown,
                    (),
                    process::TerminalCommand::Link(expression),
                )
            }

            ConstructBranch::Receive(span, pattern, branch, vars) => {
                let process = self.compile_construct_branch(branch)?;
                self.compile_pattern_receive(
                    pattern,
                    0,
                    span,
                    &LocalName::result(),
                    process,
                    vars.clone(),
                )?
            }

            ConstructBranch::ReceiveType(span, parameter, branch) => {
                let process = self.compile_construct_branch(branch)?;
                process::Process::do_step(
                    span.clone(),
                    LocalName::result(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::ReceiveType(parameter.clone()),
                    process,
                )
            }
        })
    }

    pub(crate) fn compile_apply(
        &mut self,
        apply: &Apply<Unresolved>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        Ok(match apply {
            Apply::Noop(span) => process::Process::do_terminal(
                span.clone(),
                LocalName::result(),
                VariableUsage::Unknown,
                (),
                process::TerminalCommand::Link(Arc::new(process::Expression::Variable(
                    span.clone(),
                    LocalName::object(),
                    (),
                    VariableUsage::Unknown,
                ))),
            ),

            Apply::Send(span, expression, apply) => {
                let expression = self.compile_expression(expression)?;
                let process = self.compile_apply(apply)?;
                process::Process::do_step(
                    span.clone(),
                    LocalName::object(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::Send(expression),
                    process,
                )
            }

            Apply::Signal(span, chosen, apply) => {
                let process = self.compile_apply(apply)?;
                process::Process::do_step(
                    span.clone(),
                    LocalName::object(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::Signal(chosen.clone()),
                    process,
                )
            }

            Apply::Case(span, ApplyBranches(expression_branches), else_branch) => {
                let mut branches = Vec::new();
                let mut processes = Vec::new();
                for (branch_name, expression_branch) in expression_branches {
                    branches.push(branch_name.clone());
                    processes.push(self.compile_apply_branch(expression_branch)?);
                }
                let else_process = match else_branch {
                    Some(branch) => Some(self.compile_apply_branch(branch)?),
                    None => None,
                };
                let branches = Arc::from(branches);
                let processes = Box::from(processes);
                process::Process::do_terminal(
                    span.clone(),
                    LocalName::object(),
                    VariableUsage::Unknown,
                    (),
                    process::TerminalCommand::Case(branches, processes, else_process),
                )
            }

            Apply::Begin {
                span,
                unfounded,
                label,
                then: apply,
            } => {
                let process = self.compile_apply(apply)?;
                process::Process::do_terminal(
                    span.clone(),
                    LocalName::object(),
                    VariableUsage::Unknown,
                    (),
                    process::TerminalCommand::Begin {
                        unfounded: *unfounded,
                        label: label.clone(),
                        captures: Captures::new(),
                        body: process,
                    },
                )
            }

            Apply::Loop(span, label) => process::Process::do_terminal(
                span.clone(),
                LocalName::object(),
                VariableUsage::Unknown,
                (),
                process::TerminalCommand::Loop(
                    label.clone(),
                    LocalName::invalid(),
                    Captures::new(),
                ),
            ),

            Apply::SendType(span, argument, apply) => {
                let process = self.compile_apply(apply)?;
                process::Process::do_step(
                    span.clone(),
                    LocalName::object(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::SendType(argument.clone()),
                    process,
                )
            }

            Apply::Default(span, expr, apply) => {
                let default_expr = self.compile_expression(expr)?;
                let ok_process = self.compile_apply(apply)?;
                self.compile_default(span, LocalName::object(), default_expr, ok_process)
            }

            Apply::Try(span, label, apply) => {
                let catch_block = self.use_catch(span, label)?;
                let ok_process = self.compile_apply(apply)?;
                self.compile_try(span, LocalName::object(), catch_block, ok_process)
            }

            Apply::Pipe(span, function, apply) => {
                let function = self.compile_expression(function)?;
                let then = self.compile_apply(apply)?;
                self.compile_pipe(span, LocalName::object(), function, then)
            }
        })
    }

    pub(crate) fn compile_apply_branch(
        &mut self,
        branch: &ApplyBranch<Unresolved>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        Ok(match branch {
            ApplyBranch::Then(span, name, expression) => {
                let expression = self.compile_expression(expression)?;
                process::Process::let_step(
                    span.clone(),
                    name.clone(),
                    None,
                    (),
                    Arc::new(process::Expression::Variable(
                        span.clone(),
                        LocalName::object(),
                        (),
                        VariableUsage::Unknown,
                    )),
                    process::Process::do_terminal(
                        span.clone(),
                        LocalName::result(),
                        VariableUsage::Unknown,
                        (),
                        process::TerminalCommand::Link(expression),
                    ),
                )
            }

            ApplyBranch::Receive(span, pattern, branch, vars) => {
                let process = self.compile_apply_branch(branch)?;
                self.compile_pattern_receive(
                    pattern,
                    0,
                    span,
                    &LocalName::object(),
                    process,
                    vars.clone(),
                )?
            }

            ApplyBranch::Continue(span, expression) => {
                let expression = self.compile_expression(expression)?;
                process::Process::do_step(
                    span.clone(),
                    LocalName::object(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::Continue,
                    process::Process::do_terminal(
                        span.clone(),
                        LocalName::result(),
                        VariableUsage::Unknown,
                        (),
                        process::TerminalCommand::Link(expression),
                    ),
                )
            }

            ApplyBranch::ReceiveType(span, parameter, branch) => {
                let process = self.compile_apply_branch(branch)?;
                process::Process::do_step(
                    span.clone(),
                    LocalName::object(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::ReceiveType(parameter.clone()),
                    process,
                )
            }

            ApplyBranch::Try(span, label, branch) => {
                let catch_block = self.use_catch(span, label)?;
                let process = self.compile_apply_branch(branch)?;
                self.compile_try(span, LocalName::object(), catch_block, process)
            }

            ApplyBranch::Default(span, expr, branch) => {
                let default_expr = self.compile_expression(expr)?;
                let ok_process = self.compile_apply_branch(branch)?;
                self.compile_default(span, LocalName::object(), default_expr, ok_process)
            }
        })
    }

    pub(crate) fn compile_process(
        &mut self,
        process: &Process<Unresolved>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        Ok(match process {
            Process::If {
                span,
                branches,
                else_,
                then,
            } => {
                if let Some(tail) = then {
                    let tail = self.compile_process(tail)?;
                    self.with_fallthrough(tail, |pass| {
                        let else_proc = match else_ {
                            Some(proc) => pass.compile_process(proc)?,
                            None => process::Process::terminal(process::Terminator::Unreachable(
                                span.clone(),
                            )),
                        };
                        pass.compile_if_branches(branches, else_proc, |body, pass| {
                            pass.compile_process(body)
                        })
                    })?
                } else {
                    let else_proc = match else_ {
                        Some(proc) => self.compile_process(proc)?,
                        None => process::Process::terminal(process::Terminator::Unreachable(
                            span.clone(),
                        )),
                    };
                    self.compile_if_branches(branches, else_proc, |body, pass| {
                        pass.compile_process(body)
                    })?
                }
            }

            Process::Poll {
                span,
                label,
                clients,
                name,
                then,
                else_,
            } => {
                let clients: Result<Vec<_>, _> =
                    clients.iter().map(|e| self.compile_expression(e)).collect();
                let clients = clients?;
                self.make_poll_process(
                    span,
                    process::PollKind::Poll,
                    label,
                    clients,
                    name.clone(),
                    |pass| pass.compile_process(then),
                    |pass| pass.compile_process(else_),
                )?
            }

            Process::Repoll {
                span,
                label,
                clients,
                name,
                then,
                else_,
            } => {
                let clients: Result<Vec<_>, _> =
                    clients.iter().map(|e| self.compile_expression(e)).collect();
                let clients = clients?;
                self.make_poll_process(
                    span,
                    process::PollKind::Repoll,
                    label,
                    clients,
                    name.clone(),
                    |pass| pass.compile_process(then),
                    |pass| pass.compile_process(else_),
                )?
            }

            Process::Submit {
                span,
                label,
                values,
            } => {
                let values: Result<Vec<_>, _> =
                    values.iter().map(|e| self.compile_expression(e)).collect();
                let values = values?;
                self.make_submit_process(span, label, values)?
            }

            Process::Let {
                span,
                pattern,
                value,
                then,
            } => {
                let value = self.expr_without_fallthrough(|pass| {
                    pass.disable_catches(CatchDisabledReason::DifferentProcess);
                    let value = pass.compile_expression(value)?;
                    pass.enable_catches();
                    Ok(value)
                })?;
                let then_process = self.compile_process(then)?;
                self.compile_pattern_let(pattern, span, value, then_process)?
            }

            Process::Catch {
                span,
                label,
                pattern,
                block,
                then,
            } => {
                let block = self.without_fallthrough(|pass| {
                    let block = pass.compile_process(block)?;
                    pass.compile_pattern_catch_block(pattern, span, block)
                })?;
                let process =
                    self.with_catch(label.clone(), block, |pass| pass.compile_process(then))?;
                process
            }

            Process::Throw(span, label, expression) => {
                let catch_block = self.use_catch(span, label)?;
                let expression = self.expr_without_fallthrough(|pass| {
                    pass.disable_catches(CatchDisabledReason::DifferentProcess);
                    let expression = pass.compile_expression(expression);
                    pass.enable_catches();
                    expression
                })?;
                process::Process::let_step(
                    span.clone(),
                    LocalName::error(),
                    None,
                    (),
                    expression,
                    catch_block,
                )
            }

            Process::GlobalCommand(_, global_name, command) => {
                let span = global_name.span.clone();
                let local_name = LocalName {
                    span: span.clone(),
                    string: ArcStr::from(global_name.to_string()),
                };
                process::Process::let_step(
                    span.clone(),
                    local_name.clone(),
                    None,
                    (),
                    Arc::new(process::Expression::Global(span, global_name.clone(), ())),
                    self.compile_command(command, &local_name)?,
                )
            }

            Process::Command(_, name, Command::Then(next)) => process::Process::do_step(
                name.span.clone(),
                name.clone(),
                VariableUsage::Unknown,
                (),
                process::Command::Noop,
                self.compile_process(next)?,
            ),

            Process::Command(_, name, command) => {
                let None = self.original_object_name else {
                    // this should never happen. If it did it means we forgot to exit the alias-mode.
                    unreachable!(
                        "Can't be in more than one command chain at once. currently set to: {}",
                        self.original_object_name.clone().unwrap().string
                    )
                };
                self.original_object_name = Some(name.clone());
                let then_process = self.compile_command(command, &LocalName::subject())?;
                let None = self.original_object_name else {
                    // this should never happen. If it did it means we forgot to exit the alias-mode.
                    unreachable!(
                        "Can't be in more than one command chain at once. {:?} was: {}",
                        command,
                        self.original_object_name.clone().unwrap().string
                    )
                };
                process::Process::let_step(
                    name.span.clone(),
                    LocalName::subject(),
                    None,
                    (),
                    Arc::new(process::Expression::Variable(
                        name.span.clone(),
                        name.clone(),
                        (),
                        VariableUsage::Unknown,
                    )),
                    then_process,
                )
            }

            Process::Fallthrough(span) => match self.use_fallthrough(span) {
                Some(process) => process,
                None => Err(CompileError::MustEndProcess(span.clone()))?,
            },
        })
    }

    pub(crate) fn compile_command(
        &mut self,
        command: &Command<Unresolved>,
        object_name: &LocalName,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        let mut frames = Vec::new();
        let mut command = command;

        let mut process = loop {
            match command {
                Command::Send(span, argument, then) => {
                    self.disable_catches(CatchDisabledReason::DifferentProcess);
                    let argument = self.compile_expression(argument)?;
                    self.enable_catches();
                    frames.push(CommandLoweringFrame::Step(process::Step::Do {
                        span: span.clone(),
                        name: object_name.clone(),
                        usage: VariableUsage::Unknown,
                        typ: (),
                        command: process::Command::Send(argument),
                    }));
                    command = then;
                }
                Command::Receive(span, pattern, then, vars) => {
                    frames.push(CommandLoweringFrame::Receive {
                        span: span.clone(),
                        pattern: pattern.clone(),
                        vars: vars.clone(),
                        original_object_name: self.original_object_name.clone(),
                    });
                    command = then;
                }
                Command::Signal(span, chosen, then) => {
                    frames.push(CommandLoweringFrame::Step(process::Step::Do {
                        span: span.clone(),
                        name: object_name.clone(),
                        usage: VariableUsage::Unknown,
                        typ: (),
                        command: process::Command::Signal(chosen.clone()),
                    }));
                    command = then;
                }
                Command::SendType(span, argument, then) => {
                    frames.push(CommandLoweringFrame::Step(process::Step::Do {
                        span: span.clone(),
                        name: object_name.clone(),
                        usage: VariableUsage::Unknown,
                        typ: (),
                        command: process::Command::SendType(argument.clone()),
                    }));
                    command = then;
                }
                Command::ReceiveType(span, parameter, then) => {
                    frames.push(CommandLoweringFrame::Step(process::Step::Do {
                        span: span.clone(),
                        name: object_name.clone(),
                        usage: VariableUsage::Unknown,
                        typ: (),
                        command: process::Command::ReceiveType(parameter.clone()),
                    }));
                    command = then;
                }
                Command::Try(span, label, then) => {
                    frames.push(CommandLoweringFrame::Try {
                        span: span.clone(),
                        catch_block: self.use_catch(span, label)?,
                    });
                    command = then;
                }
                Command::Default(span, expression, then) => {
                    frames.push(CommandLoweringFrame::Default {
                        span: span.clone(),
                        expression: self.compile_expression(expression)?,
                    });
                    command = then;
                }
                Command::Pipe(span, function, then) => {
                    self.disable_catches(CatchDisabledReason::DifferentProcess);
                    let function = self.compile_expression(function)?;
                    self.enable_catches();
                    frames.push(CommandLoweringFrame::Pipe {
                        span: span.clone(),
                        function,
                    });
                    command = then;
                }
                Command::Then(_)
                | Command::Link(..)
                | Command::Case(..)
                | Command::Break(_)
                | Command::Continue(..)
                | Command::Begin { .. }
                | Command::Loop(..) => {
                    break self.compile_command_terminal(command, object_name)?;
                }
            }
        };

        let flush_steps =
            |steps: &mut Vec<process::Step<(), Unresolved>>,
             process: Arc<process::Process<(), Unresolved>>| {
                if steps.is_empty() {
                    return process;
                }
                steps.reverse();
                let mut builder = process::ProcessBuilder::new();
                for step in steps.drain(..) {
                    builder.push(step);
                }
                builder.finish_with(process)
            };

        let mut steps = Vec::new();
        for frame in frames.into_iter().rev() {
            match frame {
                CommandLoweringFrame::Step(step) => steps.push(step),
                CommandLoweringFrame::Receive {
                    span,
                    pattern,
                    vars,
                    original_object_name,
                } => {
                    process = flush_steps(&mut steps, process);
                    let None = self.original_object_name else {
                        unreachable!("original_object_name should be none after command")
                    };
                    self.original_object_name = original_object_name;
                    process = self.compile_pattern_receive(
                        &pattern,
                        0,
                        &span,
                        object_name,
                        process,
                        vars,
                    )?;
                    self.original_object_name = None;
                }
                CommandLoweringFrame::Try { span, catch_block } => {
                    process = flush_steps(&mut steps, process);
                    process = self.compile_try(&span, object_name.clone(), catch_block, process);
                }
                CommandLoweringFrame::Default { span, expression } => {
                    process = flush_steps(&mut steps, process);
                    process = self.compile_default(&span, object_name.clone(), expression, process);
                }
                CommandLoweringFrame::Pipe { span, function } => {
                    process = flush_steps(&mut steps, process);
                    process = self.compile_pipe(&span, object_name.clone(), function, process);
                }
            }
        }
        Ok(flush_steps(&mut steps, process))
    }

    fn compile_command_terminal(
        &mut self,
        command: &Command<Unresolved>,
        object_name: &LocalName,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        Ok(match command {
            Command::Then(process) => {
                let original = std::mem::take(&mut self.original_object_name);
                let process = self.compile_process(process)?;
                self.restore_object_name(original, process)
            }

            Command::Link(span, expression) => {
                self.disable_catches(CatchDisabledReason::DifferentProcess);
                let expression = self.compile_expression(expression)?;
                self.enable_catches();
                self.original_object_name = None;
                process::Process::do_terminal(
                    span.clone(),
                    object_name.clone(),
                    VariableUsage::Unknown,
                    (),
                    process::TerminalCommand::Link(expression),
                )
            }

            Command::Send(span, argument, command) => {
                self.disable_catches(CatchDisabledReason::DifferentProcess);
                let argument = self.compile_expression(argument)?;
                self.enable_catches();
                let process = self.compile_command(command, object_name)?;
                process::Process::do_step(
                    span.clone(),
                    object_name.clone(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::Send(argument),
                    process,
                )
            }

            Command::Receive(span, pattern, command, vars) => {
                let original = self.original_object_name.clone();
                let process = self.compile_command(command, object_name)?;
                let None = self.original_object_name else {
                    unreachable!("original_object_name should be none after command")
                };
                self.original_object_name = original;
                let process = self.compile_pattern_receive(
                    pattern,
                    0,
                    span,
                    object_name,
                    process,
                    vars.clone(),
                )?;
                self.original_object_name = None;
                process
            }

            Command::Signal(span, chosen, command) => {
                let process = self.compile_command(command, object_name)?;
                process::Process::do_step(
                    span.clone(),
                    object_name.clone(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::Signal(chosen.clone()),
                    process,
                )
            }

            Command::Case(
                span,
                CommandBranches(process_branches),
                else_branch,
                optional_process,
            ) => {
                let original = std::mem::take(&mut self.original_object_name);
                let object_name = match &original {
                    None => object_name,
                    Some(original) => original,
                };

                let mut branches = Vec::new();
                let mut processes = Vec::new();

                let process = if let Some(process) = optional_process {
                    let process = self.compile_process(process)?;
                    self.with_fallthrough(process, |pass| {
                        for (branch_name, process_branch) in process_branches {
                            branches.push(branch_name.clone());
                            processes
                                .push(pass.compile_command_branch(process_branch, object_name)?);
                        }
                        let else_process = match else_branch {
                            Some(branch) => Some(pass.compile_command_branch(branch, object_name)?),
                            None => None,
                        };
                        let branches = Arc::from(branches);
                        let processes = Box::from(processes);
                        Ok(process::Process::do_terminal(
                            span.clone(),
                            object_name.clone(),
                            VariableUsage::Unknown,
                            (),
                            process::TerminalCommand::Case(branches, processes, else_process),
                        ))
                    })?
                } else {
                    for (branch_name, process_branch) in process_branches {
                        branches.push(branch_name.clone());
                        processes.push(self.compile_command_branch(process_branch, object_name)?);
                    }
                    let else_process = match else_branch {
                        Some(branch) => Some(self.compile_command_branch(branch, object_name)?),
                        None => None,
                    };
                    let branches = Arc::from(branches);
                    let processes = Box::from(processes);
                    process::Process::do_terminal(
                        span.clone(),
                        object_name.clone(),
                        VariableUsage::Unknown,
                        (),
                        process::TerminalCommand::Case(branches, processes, else_process),
                    )
                };
                self.restore_object_name(original, process)
            }

            Command::Break(span) => {
                self.original_object_name = None;
                process::Process::do_terminal(
                    span.clone(),
                    object_name.clone(),
                    VariableUsage::Unknown,
                    (),
                    process::TerminalCommand::Break,
                )
            }

            Command::Continue(span, process) => {
                self.original_object_name = None;
                let process = self.compile_process(process)?;
                process::Process::do_step(
                    span.clone(),
                    object_name.clone(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::Continue,
                    process,
                )
            }

            Command::Begin {
                span,
                unfounded,
                label,
                then: command,
            } => {
                let process = self.compile_command(command, object_name)?;
                process::Process::do_terminal(
                    span.clone(),
                    object_name.clone(),
                    VariableUsage::Unknown,
                    (),
                    process::TerminalCommand::Begin {
                        unfounded: *unfounded,
                        label: label.clone(),
                        captures: Captures::new(),
                        body: process,
                    },
                )
            }

            Command::Loop(span, label) => {
                self.original_object_name = None;
                process::Process::do_terminal(
                    span.clone(),
                    object_name.clone(),
                    VariableUsage::Unknown,
                    (),
                    process::TerminalCommand::Loop(
                        label.clone(),
                        LocalName::invalid(),
                        Captures::new(),
                    ),
                )
            }

            Command::SendType(span, argument, command) => {
                let process = self.compile_command(command, object_name)?;
                process::Process::do_step(
                    span.clone(),
                    object_name.clone(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::SendType(argument.clone()),
                    process,
                )
            }

            Command::ReceiveType(span, parameter, command) => {
                let process = self.compile_command(command, object_name)?;
                process::Process::do_step(
                    span.clone(),
                    object_name.clone(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::ReceiveType(parameter.clone()),
                    process,
                )
            }

            Command::Try(span, label, command) => {
                let catch_block = self.use_catch(span, label)?;
                let ok_process = self.compile_command(command, object_name)?;
                self.compile_try(span, object_name.clone(), catch_block, ok_process)
            }

            Command::Default(span, expr, command) => {
                let default_expr = self.compile_expression(expr)?;
                let ok_process = self.compile_command(command, object_name)?;
                self.compile_default(span, object_name.clone(), default_expr, ok_process)
            }

            Command::Pipe(span, function, command) => {
                self.disable_catches(CatchDisabledReason::DifferentProcess);
                let function = self.compile_expression(function)?;
                self.enable_catches();
                let process = self.compile_command(command, object_name)?;
                self.compile_pipe(span, object_name.clone(), function, process)
            }
        })
    }

    pub(crate) fn compile_command_branch(
        &mut self,
        branch: &CommandBranch<Unresolved>,
        object_name: &LocalName,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        Ok(match branch {
            CommandBranch::Then(_, process) => self.compile_process(process)?,

            CommandBranch::BindThen(span, name, process) => {
                let process = self.compile_process(process)?;
                process::Process::let_step(
                    span.clone(),
                    name.clone(),
                    None,
                    (),
                    Arc::new(process::Expression::Variable(
                        span.clone(),
                        object_name.clone(),
                        (),
                        VariableUsage::Unknown,
                    )),
                    process,
                )
            }

            CommandBranch::Receive(span, pattern, branch, vars) => {
                let process = self.compile_command_branch(branch, object_name)?;
                self.compile_pattern_receive(pattern, 0, span, object_name, process, vars.clone())?
            }

            CommandBranch::Continue(span, process) => {
                let process = self.compile_process(process)?;
                process::Process::do_step(
                    span.clone(),
                    object_name.clone(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::Continue,
                    process,
                )
            }

            CommandBranch::ReceiveType(span, parameter, branch) => {
                let process = self.compile_command_branch(branch, object_name)?;
                process::Process::do_step(
                    span.clone(),
                    object_name.clone(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::ReceiveType(parameter.clone()),
                    process,
                )
            }

            CommandBranch::Try(span, label, branch) => {
                let catch_block = self.use_catch(span, label)?;
                let process = self.compile_command_branch(branch, object_name)?;
                self.compile_try(span, object_name.clone(), catch_block, process)
            }
            CommandBranch::Default(span, expr, branch) => {
                let default_expr = self.compile_expression(expr)?;
                let ok_process = self.compile_command_branch(branch, object_name)?;
                self.compile_default(span, object_name.clone(), default_expr, ok_process)
            }
        })
    }

    fn compile_try(
        &self,
        span: &Span,
        variable: LocalName,
        catch_block: Arc<process::Process<(), Unresolved>>,
        ok_process: Arc<process::Process<(), Unresolved>>,
    ) -> Arc<process::Process<(), Unresolved>> {
        process::Process::do_terminal(
            span.clone(),
            variable.clone(),
            VariableUsage::Unknown,
            (),
            process::TerminalCommand::Case(
                Arc::from([
                    LocalName::from(literal!("err")),
                    LocalName::from(literal!("ok")),
                ]),
                Box::from([
                    process::Process::let_step(
                        span.clone(),
                        LocalName::error(),
                        None,
                        (),
                        Arc::new(process::Expression::Variable(
                            span.clone(),
                            variable,
                            (),
                            VariableUsage::Unknown,
                        )),
                        catch_block,
                    ),
                    ok_process,
                ]),
                None,
            ),
        )
    }

    fn compile_default(
        &mut self,
        span: &Span,
        variable: LocalName,
        default_expr: Arc<process::Expression<(), Unresolved>>,
        ok_process: Arc<process::Process<(), Unresolved>>,
    ) -> Arc<process::Process<(), Unresolved>> {
        self.with_fallthrough(ok_process, |pass| {
            Ok(process::Process::do_terminal(
                span.clone(),
                variable.clone(),
                VariableUsage::Unknown,
                (),
                process::TerminalCommand::Case(
                    Arc::from([
                        LocalName::from(literal!("none")),
                        LocalName::from(literal!("some")),
                    ]),
                    Box::from([
                        process::Process::let_step(
                            span.clone(),
                            variable.clone(),
                            None,
                            (),
                            default_expr,
                            pass.use_fallthrough(span).unwrap(),
                        ),
                        pass.use_fallthrough(span).unwrap(),
                    ]),
                    None,
                ),
            ))
        })
        .unwrap()
    }

    fn compile_pipe(
        &self,
        span: &Span,
        variable: LocalName,
        function: Arc<process::Expression<(), Unresolved>>,
        then: Arc<process::Process<(), Unresolved>>,
    ) -> Arc<process::Process<(), Unresolved>> {
        process::Process::let_step(
            span.clone(),
            LocalName::temp(),
            None,
            (),
            function,
            process::Process::do_step(
                span.clone(),
                LocalName::temp(),
                VariableUsage::Unknown,
                (),
                process::Command::Send(Arc::new(process::Expression::Variable(
                    span.clone(),
                    variable.clone(),
                    (),
                    VariableUsage::Unknown,
                ))),
                process::Process::let_step(
                    span.clone(),
                    variable,
                    None,
                    (),
                    Arc::new(process::Expression::Variable(
                        span.clone(),
                        LocalName::temp(),
                        (),
                        VariableUsage::Unknown,
                    )),
                    then,
                ),
            ),
        )
    }

    fn attach_pattern_to_process_compiled(
        &mut self,
        pattern: &Pattern<Unresolved>,
        body: Arc<process::Process<(), Unresolved>>,
        subject: &LocalName,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        if matches!(pattern, Pattern::Continue(_)) {
            return Ok(body);
        }
        self.compile_pattern_let(
            pattern,
            &pattern.span().join(body.span()),
            Arc::new(process::Expression::Variable(
                pattern.span(),
                subject.clone(),
                (),
                VariableUsage::Unknown,
            )),
            body,
        )
    }

    fn compile_restorations(
        &mut self,
        restores: &[Restoration],
        tail: Arc<process::Process<(), Unresolved>>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        restores.iter().rev().try_fold(tail, |acc, restore| {
            let value = self.compile_expression(&restore.value)?;
            Ok(process::Process::let_step(
                restore.span.clone(),
                restore.name.clone(),
                None,
                (),
                value,
                acc,
            ))
        })
    }

    fn condition_process_core(
        &mut self,
        condition: &Condition<Unresolved>,
        success: Arc<process::Process<(), Unresolved>>,
        failure: Arc<process::Process<(), Unresolved>>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        Ok(match condition {
            Condition::Bool(_, expr) => {
                let temp = LocalName::temp();
                let expr = self.compile_expression(expr)?;
                process::Process::let_step(
                    Span::None,
                    temp.clone(),
                    None,
                    (),
                    expr,
                    process::Process::do_terminal(
                        Span::None,
                        temp.clone(),
                        VariableUsage::Unknown,
                        (),
                        process::TerminalCommand::Case(
                            Arc::from([
                                LocalName::from(literal!("false")),
                                LocalName::from(literal!("true")),
                            ]),
                            Box::from([failure, success]),
                            None,
                        ),
                    ),
                )
            }
            Condition::Is {
                span,
                value,
                variant,
                pattern,
            } => {
                let (subject_name, binding_value) = match value {
                    Expression::Variable(_, name) => (name.clone(), None),
                    _ => (LocalName::temp(), Some(self.compile_expression(value)?)),
                };

                let success_process =
                    self.attach_pattern_to_process_compiled(pattern, success, &subject_name)?;

                let command_process = process::Process::do_terminal(
                    span.clone(),
                    subject_name.clone(),
                    VariableUsage::Unknown,
                    (),
                    process::TerminalCommand::Case(
                        Arc::from([variant.clone()]),
                        Box::from([success_process]),
                        Some(failure),
                    ),
                );

                match binding_value {
                    Some(value) => process::Process::let_step(
                        span.clone(),
                        subject_name.clone(),
                        None,
                        (),
                        value,
                        command_process,
                    ),
                    None => command_process,
                }
            }
            Condition::And(span, left, right) => self.with_fallthrough(failure, |pass| {
                let left_fallthrough = pass.use_fallthrough(span).unwrap();
                let right_fallthrough = pass.use_fallthrough(span).unwrap();
                let restored_failure =
                    pass.compile_restorations(&collect_restorations(left), right_fallthrough)?;
                let right_process =
                    pass.condition_process_core(right, success, restored_failure)?;
                pass.condition_process_core(left, right_process, left_fallthrough)
            })?,
            Condition::Or(span, left, right) => self.with_fallthrough(success, |pass| {
                let left_fallthrough = pass.use_fallthrough(span).unwrap();
                let right_fallthrough = pass.use_fallthrough(span).unwrap();
                let right_process =
                    pass.condition_process_core(right, right_fallthrough, failure)?;
                pass.condition_process_core(left, left_fallthrough, right_process)
            })?,
            Condition::Not(_, inner) => self.condition_process_core(inner, failure, success)?,
        })
    }

    fn compile_condition_process(
        &mut self,
        condition: &Condition<Unresolved>,
        success_body: Arc<process::Process<(), Unresolved>>,
        failure_body: Arc<process::Process<(), Unresolved>>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        let span = condition.span();
        self.with_fallthrough(failure_body, |pass| {
            let goto_failure = pass.use_fallthrough(&span).unwrap();
            pass.with_fallthrough(success_body, |pass| {
                let goto_success = pass.use_fallthrough(&span).unwrap();
                pass.condition_process_core(condition, goto_success, goto_failure)
            })
        })
    }

    fn compile_if_branches<B>(
        &mut self,
        branches: &[(Condition<Unresolved>, B)],
        else_proc: Arc<process::Process<(), Unresolved>>,
        mut compile_body: impl FnMut(
            &B,
            &mut Self,
        )
            -> Result<Arc<process::Process<(), Unresolved>>, CompileError>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        branches
            .iter()
            .rev()
            .try_fold(else_proc, |acc, (condition, body)| {
                let success = compile_body(body, self)?;
                self.compile_condition_process(condition, success, acc)
            })
    }
}

impl Passes {
    pub(crate) fn new() -> Self {
        Passes {
            next_block_index: 1,
            next_poll_index: 1,
            fallthrough: None,
            fallthrough_stash: Vec::new(),
            catch: HashMap::new(),
            catch_stash: HashMap::new(),
            poll: None,
            poll_stash: Vec::new(),
        }
    }

    fn get_block_index(&mut self) -> usize {
        let index = self.next_block_index;
        self.next_block_index += 1;
        index
    }

    fn get_poll_index(&mut self) -> usize {
        let index = self.next_poll_index;
        self.next_poll_index += 1;
        index
    }
}

impl Pass {
    fn new(block_index: usize) -> Self {
        Pass {
            block_index,
            used: false,
            disabled_reasons: Vec::new(),
        }
    }

    fn use_at(&mut self, span: &Span) -> Arc<process::Process<(), Unresolved>> {
        self.used = true;
        process::Process::terminal(process::Terminator::Goto(
            span.clone(),
            self.block_index,
            Captures::new(),
        ))
    }
}

impl<S> Pattern<S> {
    fn annotation(&self) -> Option<Type<S>>
    where
        S: Clone,
    {
        match self {
            Self::Name(_, _, annotation) => annotation.clone(),
            Self::Receive(span, first, rest, vars) => {
                let first = first.annotation()?;
                let rest = rest.annotation()?;
                Some(Type::Pair(
                    span.clone(),
                    Box::new(first),
                    Box::new(rest),
                    vars.clone(),
                ))
            }
            Self::Continue(span) => Some(Type::Break(span.clone())),
            Self::ReceiveType(span, parameter, rest) => {
                let rest = rest.annotation()?;
                Some(Type::Exists(
                    span.clone(),
                    parameter.clone(),
                    Box::new(rest),
                ))
            }
            Self::Try(_, _, _) => None,
            Self::Default(_, _, rest) => rest.annotation(),
        }
    }
}

impl<S> Spanning for Pattern<S> {
    fn span(&self) -> Span {
        match self {
            Self::Name(span, _, _)
            | Self::Continue(span)
            | Self::Receive(span, _, _, _)
            | Self::ReceiveType(span, _, _)
            | Self::Try(span, _, _)
            | Self::Default(span, _, _) => span.clone(),
        }
    }
}

impl<S> Spanning for Expression<S> {
    fn span(&self) -> Span {
        match self {
            Self::Primitive(span, _)
            | Self::Template { span, .. }
            | Self::List(span, _)
            | Self::Global(span, _)
            | Self::Variable(span, _)
            | Self::Poll { span, .. }
            | Self::Repoll { span, .. }
            | Self::Submit { span, .. }
            | Self::Condition(span, _)
            | Self::Grouped(span, _)
            | Self::TypeIn { span, .. }
            | Self::Let { span, .. }
            | Self::Catch { span, .. }
            | Self::Throw(span, _, _)
            | Self::If { span, .. }
            | Self::Do { span, .. }
            | Self::Box(span, _)
            | Self::Chan { span, .. }
            | Self::Arithmetic { span, .. }
            | Self::Neg { span, .. }
            | Self::ComparisonChain { span, .. }
            | Self::Application(span, _, _)
            | Self::Construction(span, _) => span.clone(),
        }
    }
}

impl<S> Spanning for Construct<S> {
    fn span(&self) -> Span {
        match self {
            Self::Send(span, _, _)
            | Self::Receive(span, _, _, _)
            | Self::Signal(span, _, _)
            | Self::Case(span, _, _)
            | Self::Break(span)
            | Self::Begin { span, .. }
            | Self::Loop(span, _)
            | Self::SendType(span, _, _)
            | Self::ReceiveType(span, _, _) => span.clone(),

            Self::Then(expression) => expression.span(),
        }
    }
}

impl<S> Spanning for ConstructBranch<S> {
    fn span(&self) -> Span {
        match self {
            Self::Then(span, _) | Self::Receive(span, _, _, _) | Self::ReceiveType(span, _, _) => {
                span.clone()
            }
        }
    }
}

impl<S> Spanning for Apply<S> {
    fn span(&self) -> Span {
        match self {
            Self::Send(span, _, _)
            | Self::Signal(span, _, _)
            | Self::Case(span, _, _)
            | Self::Begin { span, .. }
            | Self::Loop(span, _)
            | Self::SendType(span, _, _)
            | Self::Noop(span)
            | Self::Try(span, _, _)
            | Self::Default(span, _, _)
            | Self::Pipe(span, _, _) => span.clone(),
        }
    }
}

impl<S> Spanning for ApplyBranch<S> {
    fn span(&self) -> Span {
        match self {
            Self::Then(span, _, _)
            | Self::Receive(span, _, _, _)
            | Self::Continue(span, _)
            | Self::ReceiveType(span, _, _)
            | Self::Try(span, _, _)
            | Self::Default(span, _, _) => span.clone(),
        }
    }
}

impl<S> Spanning for Process<S> {
    fn span(&self) -> Span {
        match self {
            Self::Let { span, .. } => span.clone(),
            Self::Poll { span, .. } => span.clone(),
            Self::Repoll { span, .. } => span.clone(),
            Self::Submit { span, .. } => span.clone(),
            Self::Catch { span, .. } => span.clone(),
            Self::Throw(span, _, _) => span.clone(),
            Self::If { span, .. } => span.clone(),
            Self::GlobalCommand(span, _, _) => span.clone(),
            Self::Command(span, _, _) => span.clone(),
            Self::Fallthrough(span) => span.clone(),
        }
    }
}

impl<S> Spanning for Command<S> {
    fn span(&self) -> Span {
        match self {
            Self::Link(span, _)
            | Self::Send(span, _, _)
            | Self::Receive(span, _, _, _)
            | Self::Signal(span, _, _)
            | Self::Case(span, _, _, _)
            | Self::Break(span)
            | Self::Continue(span, _)
            | Self::Begin { span, .. }
            | Self::Loop(span, _)
            | Self::SendType(span, _, _)
            | Self::ReceiveType(span, _, _)
            | Self::Try(span, _, _)
            | Self::Default(span, _, _)
            | Self::Pipe(span, _, _) => span.clone(),

            Self::Then(process) => process.span(),
        }
    }
}

impl<S> Spanning for CommandBranch<S> {
    fn span(&self) -> Span {
        match self {
            Self::Then(span, _)
            | Self::BindThen(span, _, _)
            | Self::Receive(span, _, _, _)
            | Self::Continue(span, _)
            | Self::ReceiveType(span, _, _)
            | Self::Try(span, _, _)
            | Self::Default(span, _, _) => span.clone(),
        }
    }
}

#[derive(Clone)]
struct Restoration {
    span: Span,
    name: LocalName,
    value: Expression<Unresolved>,
}

fn pattern_to_expression(pattern: &Pattern<Unresolved>) -> Option<Expression<Unresolved>> {
    match pattern {
        Pattern::Name(span, name, _) => Some(Expression::Variable(span.clone(), name.clone())),
        Pattern::Receive(span, first, rest, _vars) => {
            let first_expr = pattern_to_expression(first)?;
            let then = construct_from_pattern(rest)?;
            Some(Expression::Construction(
                span.clone(),
                Construct::Send(span.clone(), Box::new(first_expr), Box::new(then)),
            ))
        }
        Pattern::Continue(span) => Some(Expression::Construction(
            span.clone(),
            Construct::Break(span.clone()),
        )),
        Pattern::ReceiveType(_, _, _) | Pattern::Try(_, _, _) | Pattern::Default(_, _, _) => None,
    }
}

fn construct_from_pattern(pattern: &Pattern<Unresolved>) -> Option<Construct<Unresolved>> {
    match pattern {
        Pattern::Name(span, name, _) => Some(Construct::Then(Box::new(Expression::Variable(
            span.clone(),
            name.clone(),
        )))),
        Pattern::Receive(span, first, rest, _vars) => {
            let expression = pattern_to_expression(first)?;
            let then = construct_from_pattern(rest)?;
            Some(Construct::Send(
                span.clone(),
                Box::new(expression),
                Box::new(then),
            ))
        }
        Pattern::Continue(span) => Some(Construct::Break(span.clone())),
        Pattern::ReceiveType(_, _, _) | Pattern::Try(_, _, _) | Pattern::Default(_, _, _) => None,
    }
}

fn reconstruction_for_is(
    span: &Span,
    value: &Expression<Unresolved>,
    variant: &LocalName,
    pattern: &Pattern<Unresolved>,
) -> Option<Restoration> {
    let Expression::Variable(_, name) = value else {
        return None;
    };
    let payload = construct_from_pattern(pattern)?;
    let reconstruction = Expression::Construction(
        span.clone(),
        Construct::Signal(span.clone(), variant.clone(), Box::new(payload)),
    );
    Some(Restoration {
        span: span.clone(),
        name: name.clone(),
        value: reconstruction,
    })
}

fn collect_restorations(condition: &Condition<Unresolved>) -> Vec<Restoration> {
    match condition {
        Condition::Bool(_, _) => Vec::new(),
        Condition::Is {
            span,
            value,
            variant,
            pattern,
        } => reconstruction_for_is(span, value, variant, pattern)
            .into_iter()
            .collect(),
        Condition::And(_, left, right) => {
            let mut restores = collect_restorations(left);
            restores.extend(collect_restorations(right));
            restores
        }
        Condition::Or(_, _, _) => Vec::new(),
        Condition::Not(_, _) => Vec::new(),
    }
}

fn link_process_from_expr(expr: &Expression<Unresolved>) -> Process<Unresolved> {
    let span = expr.span();
    Process::Command(
        span.clone(),
        LocalName::result(),
        Command::Link(span, Box::new(expr.clone())),
    )
}

#[cfg(test)]
mod flat_ir_tests {
    use super::*;

    #[test]
    fn lowers_long_command_chain_without_recursive_ir_walk() {
        const STEP_COUNT: usize = 20_000;
        let subject = LocalName::from(literal!("subject"));
        let chosen = LocalName::from(literal!("next"));
        let mut command = Command::Break(Span::None);
        for _ in 0..STEP_COUNT {
            command = Command::Signal(Span::None, chosen.clone(), Box::new(command));
        }

        let mut context = Context::new();
        let lowered = context.compile_command(&command, &subject).unwrap();
        assert_eq!(lowered.steps.len(), STEP_COUNT);
        assert!(matches!(
            lowered.terminator,
            process::Terminator::Do {
                command: process::TerminalCommand::Break,
                ..
            }
        ));

        let (fixed, _) = lowered.fix_captures();
        assert_eq!(fixed.steps.len(), STEP_COUNT);

        // The source AST is intentionally recursive and outside this refactor's scope. Avoid a
        // recursive destructor walk obscuring what this test is measuring.
        std::mem::forget(command);
    }
}
