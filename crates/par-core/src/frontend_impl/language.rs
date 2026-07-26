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
    Open,
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
            TypeConstraint::Open => 1,
            TypeConstraint::Box => 2,
            TypeConstraint::Data => 3,
            TypeConstraint::Number => 4,
            TypeConstraint::Signed => 5,
        }
    }
}

impl Display for TypeConstraint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeConstraint::Any => Ok(()),
            TypeConstraint::Open => write!(f, "open"),
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
pub struct Pattern<S> {
    pub steps: Vec<PatternStep<S>>,
    pub terminal: PatternTerminal<S>,
}

#[derive(Clone, Debug)]
pub enum PatternStep<S> {
    Receive(Span, Box<Pattern<S>>, Vec<TypeParameter>),
    ReceiveType(Span, TypeParameter),
    Try(Span, Option<LocalName>),
    Default(Span, Box<Expression<S>>),
}

#[derive(Clone, Debug)]
pub enum PatternTerminal<S> {
    Name(Span, LocalName, Option<Type<S>>),
    Continue(Span),
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
pub struct Construct<S> {
    pub steps: Vec<ConstructStep<S>>,
    pub terminator: ConstructTerminator<S>,
}

#[derive(Clone, Debug)]
pub enum ConstructStep<S> {
    Send(Span, Box<Expression<S>>),
    Receive(Span, Pattern<S>, Vec<TypeParameter>),
    Signal(Span, LocalName),
    SendType(Span, Type<S>),
    ReceiveType(Span, TypeParameter),
}

#[derive(Clone, Debug)]
pub enum ConstructTerminator<S> {
    Then(Box<Expression<S>>),
    Case(Span, ConstructBranches<S>, Option<Box<ConstructBranch<S>>>),
    Break(Span),
    Begin {
        span: Span,
        unfounded: bool,
        label: Option<LocalName>,
        body: Box<Construct<S>>,
    },
    Loop(Span, Option<LocalName>),
    Submit {
        span: Span,
        label: Option<LocalName>,
        values: Vec<Expression<S>>,
    },
}

#[derive(Clone, Debug)]
pub struct ConstructBranches<S>(pub BTreeMap<LocalName, ConstructBranch<S>>);

#[derive(Clone, Debug)]
pub struct ConstructBranch<S> {
    pub steps: Vec<ConstructBranchStep<S>>,
    pub terminator: ConstructBranchTerminator<S>,
}

#[derive(Clone, Debug)]
pub enum ConstructBranchStep<S> {
    Receive(Span, Pattern<S>, Vec<TypeParameter>),
    ReceiveType(Span, TypeParameter),
}

#[derive(Clone, Debug)]
pub enum ConstructBranchTerminator<S> {
    Then(Span, Expression<S>),
}

#[derive(Clone, Debug)]
pub struct Apply<S> {
    pub steps: Vec<ApplyStep<S>>,
    pub terminator: ApplyTerminator<S>,
}

#[derive(Clone, Debug)]
pub enum ApplyStep<S> {
    Send(Span, Box<Expression<S>>),
    Signal(Span, LocalName),
    SendType(Span, Type<S>),
    Try(Span, Option<LocalName>),
    Default(Span, Box<Expression<S>>),
    Pipe(Span, Box<Expression<S>>),
}

#[derive(Clone, Debug)]
pub enum ApplyTerminator<S> {
    Noop(Span),
    Case(Span, ApplyBranches<S>, Option<Box<ApplyBranch<S>>>),
    Begin {
        span: Span,
        unfounded: bool,
        label: Option<LocalName>,
        body: Box<Apply<S>>,
    },
    Loop(Span, Option<LocalName>),
}

#[derive(Clone, Debug)]
pub struct ApplyBranches<S>(pub BTreeMap<LocalName, ApplyBranch<S>>);

#[derive(Clone, Debug)]
pub struct ApplyBranch<S> {
    pub steps: Vec<ApplyBranchStep<S>>,
    pub terminator: ApplyBranchTerminator<S>,
}

#[derive(Clone, Debug)]
pub enum ApplyBranchStep<S> {
    Receive(Span, Pattern<S>, Vec<TypeParameter>),
    ReceiveType(Span, TypeParameter),
    Try(Span, Option<LocalName>),
    Default(Span, Box<Expression<S>>),
}

#[derive(Clone, Debug)]
pub enum ApplyBranchTerminator<S> {
    Then(Span, LocalName, Expression<S>),
    Continue(Span, Expression<S>),
}

#[derive(Clone, Debug)]
pub struct Process<S> {
    pub steps: Vec<ProcessStep<S>>,
    pub terminator: ProcessTerminator<S>,
}

#[derive(Clone, Debug)]
pub enum ProcessStep<S> {
    Let {
        span: Span,
        pattern: Pattern<S>,
        value: Box<Expression<S>>,
    },
    Catch {
        span: Span,
        label: Option<LocalName>,
        pattern: Pattern<S>,
        block: Box<Process<S>>,
    },
    If {
        span: Span,
        branches: Vec<(Condition<S>, Process<S>)>,
        else_: Option<Box<Process<S>>>,
    },
    Command(ProcessCommand<S>),
}

#[derive(Clone, Debug)]
pub enum ProcessTerminator<S> {
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
    Throw(Span, Option<LocalName>, Box<Expression<S>>),
    If {
        span: Span,
        branches: Vec<(Condition<S>, Process<S>)>,
        else_: Option<Box<Process<S>>>,
    },
    Command(ProcessCommand<S>),
    Fallthrough(Span),
}

#[derive(Clone, Debug)]
pub struct ProcessCommand<S> {
    pub span: Span,
    pub target: CommandTarget<S>,
    pub command: Command<S>,
}

#[derive(Clone, Debug)]
pub enum CommandTarget<S> {
    Global(GlobalName<S>),
    Local(LocalName),
    Expression(Box<Expression<S>>),
}

#[derive(Clone, Debug)]
pub struct Command<S> {
    pub steps: Vec<CommandStep<S>>,
    pub terminator: CommandTerminator<S>,
}

#[derive(Clone, Debug)]
pub enum CommandStep<S> {
    Send(Span, Expression<S>),
    Receive(Span, Pattern<S>, Vec<TypeParameter>),
    Signal(Span, LocalName),
    Continue(Span),
    SendType(Span, Type<S>),
    ReceiveType(Span, TypeParameter),
    Try(Span, Option<LocalName>),
    Default(Span, Box<Expression<S>>),
    Pipe(Span, Box<Expression<S>>),
}

#[derive(Clone, Debug)]
pub enum CommandTerminator<S> {
    Then(Span),
    Link(Span, Box<Expression<S>>),
    Case(Span, CommandBranches<S>, Option<Box<CommandBranch<S>>>),
    Break(Span),
    Begin {
        span: Span,
        unfounded: bool,
        label: Option<LocalName>,
        body: Box<Command<S>>,
        continuation: Option<Box<Process<S>>>,
    },
    Loop(Span, Option<LocalName>),
}

#[derive(Clone, Debug)]
pub struct CommandBranches<S>(pub BTreeMap<LocalName, CommandBranch<S>>);

#[derive(Clone, Debug)]
pub struct CommandBranch<S> {
    pub steps: Vec<CommandBranchStep<S>>,
    pub terminator: CommandBranchTerminator<S>,
}

#[derive(Clone, Debug)]
pub enum CommandBranchStep<S> {
    Receive(Span, Pattern<S>, Vec<TypeParameter>),
    ReceiveType(Span, TypeParameter),
    Try(Span, Option<LocalName>),
    Default(Span, Box<Expression<S>>),
}

#[derive(Clone, Debug)]
pub enum CommandBranchTerminator<S> {
    Then(Span, Process<S>),
    BindThen(Span, LocalName, Process<S>),
    Continue(Span, Process<S>),
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

enum ProcessLoweringFrame {
    Let {
        span: Span,
        pattern: Pattern<Unresolved>,
        value: Arc<process::Expression<(), Unresolved>>,
    },
    Command {
        target: CommandTarget<Unresolved>,
        object_name: LocalName,
        original_object_name: Option<LocalName>,
        frames: Vec<CommandLoweringFrame>,
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
            Apply {
                steps: vec![ApplyStep::Send(span.clone(), Box::new(argument))],
                terminator: ApplyTerminator::Noop(span.clone()),
            },
        )
    }

    fn pair_expression(
        span: &Span,
        left: Expression<Unresolved>,
        right: Expression<Unresolved>,
    ) -> Expression<Unresolved> {
        Expression::Construction(
            span.clone(),
            Construct {
                steps: vec![ConstructStep::Send(span.clone(), Box::new(left))],
                terminator: ConstructTerminator::Then(Box::new(right)),
            },
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
            pattern: Pattern {
                steps: Vec::new(),
                terminal: PatternTerminal::Continue(Span::None),
            },
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
                        pattern: Pattern {
                            steps: Vec::new(),
                            terminal: PatternTerminal::Name(binding_span.clone(), name, None),
                        },
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
        if let Some((_, name, annotation)) = pattern.as_name() {
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
        if let Some((_, name, annotation)) = pattern.as_name() {
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
        if let Some((_, name, annotation)) = pattern.as_name() {
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
        if let Some((_, name, annotation)) = pattern.as_name() {
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
        let mut process = match &pattern.terminal {
            PatternTerminal::Name(span, name, annotation) => process::Process::let_step(
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
            ),
            PatternTerminal::Continue(span) => process::Process::do_step(
                span.clone(),
                LocalName::match_(level),
                VariableUsage::Unknown,
                (),
                process::Command::Continue,
                process,
            ),
        };
        for step in pattern.steps.iter().rev() {
            process = match step {
                PatternStep::Receive(span, first, vars) => self.compile_pattern_receive(
                    first,
                    level + 1,
                    span,
                    &LocalName::match_(level),
                    process,
                    vars.clone(),
                )?,
                PatternStep::ReceiveType(span, parameter) => process::Process::do_step(
                    span.clone(),
                    LocalName::match_(level),
                    VariableUsage::Unknown,
                    (),
                    process::Command::ReceiveType(parameter.clone()),
                    process,
                ),
                PatternStep::Try(span, label) => {
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
                    self.compile_try(span, LocalName::match_(level), catch_block, process)
                }
                PatternStep::Default(span, expr) => {
                    let default_expr = self.compile_expression(expr)?;
                    self.compile_default(span, LocalName::match_(level), default_expr, process)
                }
            };
        }
        Ok(process)
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

            Expression::Application(
                _,
                expr,
                Apply {
                    steps,
                    terminator: ApplyTerminator::Noop(_),
                },
            ) if steps.is_empty() => self.compile_expression(expr)?,

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
        let mut process = match &construct.terminator {
            ConstructTerminator::Then(expression) => {
                let expression = self.compile_expression(expression)?;
                process::Process::do_terminal(
                    Span::None,
                    LocalName::result(),
                    VariableUsage::Unknown,
                    (),
                    process::TerminalCommand::Link(expression),
                )
            }
            ConstructTerminator::Case(span, ConstructBranches(construct_branches), else_branch) => {
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
            ConstructTerminator::Break(span) => process::Process::do_terminal(
                span.clone(),
                LocalName::result(),
                VariableUsage::Unknown,
                (),
                process::TerminalCommand::Break,
            ),

            ConstructTerminator::Begin {
                span,
                unfounded,
                label,
                body: construct,
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

            ConstructTerminator::Loop(span, label) => process::Process::do_terminal(
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
            ConstructTerminator::Submit {
                span,
                label,
                values,
            } => {
                let values = values
                    .iter()
                    .map(|value| self.compile_expression(value))
                    .collect::<Result<_, _>>()?;
                self.make_submit_process(span, label, values)?
            }
        };
        for step in construct.steps.iter().rev() {
            process = match step {
                ConstructStep::Send(span, argument) => process::Process::do_step(
                    span.clone(),
                    LocalName::result(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::Send(self.compile_expression(argument)?),
                    process,
                ),
                ConstructStep::Receive(span, pattern, vars) => self.compile_pattern_receive(
                    pattern,
                    0,
                    span,
                    &LocalName::result(),
                    process,
                    vars.clone(),
                )?,
                ConstructStep::Signal(span, chosen) => process::Process::do_step(
                    span.clone(),
                    LocalName::result(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::Signal(chosen.clone()),
                    process,
                ),
                ConstructStep::SendType(span, argument) => process::Process::do_step(
                    span.clone(),
                    LocalName::result(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::SendType(argument.clone()),
                    process,
                ),
                ConstructStep::ReceiveType(span, parameter) => process::Process::do_step(
                    span.clone(),
                    LocalName::result(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::ReceiveType(parameter.clone()),
                    process,
                ),
            };
        }
        Ok(process)
    }

    pub(crate) fn compile_construct_branch(
        &mut self,
        branch: &ConstructBranch<Unresolved>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        let mut process = match &branch.terminator {
            ConstructBranchTerminator::Then(_, expression) => {
                let expression = self.compile_expression(expression)?;
                process::Process::do_terminal(
                    Span::None,
                    LocalName::result(),
                    VariableUsage::Unknown,
                    (),
                    process::TerminalCommand::Link(expression),
                )
            }
        };
        for step in branch.steps.iter().rev() {
            process = match step {
                ConstructBranchStep::Receive(span, pattern, vars) => self.compile_pattern_receive(
                    pattern,
                    0,
                    span,
                    &LocalName::result(),
                    process,
                    vars.clone(),
                )?,
                ConstructBranchStep::ReceiveType(span, parameter) => process::Process::do_step(
                    span.clone(),
                    LocalName::result(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::ReceiveType(parameter.clone()),
                    process,
                ),
            };
        }
        Ok(process)
    }

    pub(crate) fn compile_apply(
        &mut self,
        apply: &Apply<Unresolved>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        let mut process = match &apply.terminator {
            ApplyTerminator::Noop(span) => process::Process::do_terminal(
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

            ApplyTerminator::Case(span, ApplyBranches(expression_branches), else_branch) => {
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

            ApplyTerminator::Begin {
                span,
                unfounded,
                label,
                body: apply,
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

            ApplyTerminator::Loop(span, label) => process::Process::do_terminal(
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
        };
        for step in apply.steps.iter().rev() {
            process = match step {
                ApplyStep::Send(span, expression) => process::Process::do_step(
                    span.clone(),
                    LocalName::object(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::Send(self.compile_expression(expression)?),
                    process,
                ),
                ApplyStep::Signal(span, chosen) => process::Process::do_step(
                    span.clone(),
                    LocalName::object(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::Signal(chosen.clone()),
                    process,
                ),
                ApplyStep::SendType(span, argument) => process::Process::do_step(
                    span.clone(),
                    LocalName::object(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::SendType(argument.clone()),
                    process,
                ),
                ApplyStep::Default(span, expr) => {
                    let default_expr = self.compile_expression(expr)?;
                    self.compile_default(span, LocalName::object(), default_expr, process)
                }
                ApplyStep::Try(span, label) => {
                    let catch_block = self.use_catch(span, label)?;
                    self.compile_try(span, LocalName::object(), catch_block, process)
                }
                ApplyStep::Pipe(span, function) => {
                    let function = self.compile_expression(function)?;
                    self.compile_pipe(span, LocalName::object(), function, process)
                }
            };
        }
        Ok(process)
    }

    pub(crate) fn compile_apply_branch(
        &mut self,
        branch: &ApplyBranch<Unresolved>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        let mut process = match &branch.terminator {
            ApplyBranchTerminator::Then(span, name, expression) => {
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
            ApplyBranchTerminator::Continue(span, expression) => {
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
        };
        for step in branch.steps.iter().rev() {
            process = match step {
                ApplyBranchStep::Receive(span, pattern, vars) => self.compile_pattern_receive(
                    pattern,
                    0,
                    span,
                    &LocalName::object(),
                    process,
                    vars.clone(),
                )?,
                ApplyBranchStep::ReceiveType(span, parameter) => process::Process::do_step(
                    span.clone(),
                    LocalName::object(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::ReceiveType(parameter.clone()),
                    process,
                ),
                ApplyBranchStep::Try(span, label) => {
                    let catch_block = self.use_catch(span, label)?;
                    self.compile_try(span, LocalName::object(), catch_block, process)
                }
                ApplyBranchStep::Default(span, expr) => {
                    let default_expr = self.compile_expression(expr)?;
                    self.compile_default(span, LocalName::object(), default_expr, process)
                }
            };
        }
        Ok(process)
    }

    pub(crate) fn compile_process(
        &mut self,
        process: &Process<Unresolved>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        self.compile_process_from(process, 0)
    }

    fn compile_process_from(
        &mut self,
        source: &Process<Unresolved>,
        mut index: usize,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        let mut frames = Vec::new();
        let mut process = loop {
            let Some(step) = source.steps.get(index) else {
                break self.compile_process_terminator(&source.terminator)?;
            };
            match step {
                ProcessStep::Let {
                    span,
                    pattern,
                    value,
                } => {
                    let value = self.expr_without_fallthrough(|pass| {
                        pass.disable_catches(CatchDisabledReason::DifferentProcess);
                        let value = pass.compile_expression(value)?;
                        pass.enable_catches();
                        Ok(value)
                    })?;
                    frames.push(ProcessLoweringFrame::Let {
                        span: span.clone(),
                        pattern: pattern.clone(),
                        value,
                    });
                    index += 1;
                }
                ProcessStep::Command(command)
                    if matches!(command.command.terminator, CommandTerminator::Then(_)) =>
                {
                    frames.push(self.prepare_process_command(command)?);
                    index += 1;
                }
                ProcessStep::Catch {
                    span,
                    label,
                    pattern,
                    block,
                } => {
                    let block = self.without_fallthrough(|pass| {
                        let block = pass.compile_process(block)?;
                        pass.compile_pattern_catch_block(pattern, span, block)
                    })?;
                    break self.with_catch(label.clone(), block, |pass| {
                        pass.compile_process_from(source, index + 1)
                    })?;
                }
                ProcessStep::If {
                    span,
                    branches,
                    else_,
                } => {
                    let tail = self.compile_process_from(source, index + 1)?;
                    break self.with_fallthrough(tail, |pass| {
                        let else_proc = match else_ {
                            Some(proc) => pass.compile_process(proc)?,
                            None => process::Process::terminal(process::Terminator::Unreachable(
                                span.clone(),
                            )),
                        };
                        pass.compile_if_branches(branches, else_proc, |body, pass| {
                            pass.compile_process(body)
                        })
                    })?;
                }
                ProcessStep::Command(command) => {
                    break self.compile_process_command(command, Some((source, index + 1)))?;
                }
            }
        };
        for frame in frames.into_iter().rev() {
            process = self.apply_process_lowering_frame(frame, process)?;
        }
        Ok(process)
    }

    fn compile_process_terminator(
        &mut self,
        terminator: &ProcessTerminator<Unresolved>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        Ok(match terminator {
            ProcessTerminator::Poll {
                span,
                label,
                clients,
                name,
                then,
                else_,
            }
            | ProcessTerminator::Repoll {
                span,
                label,
                clients,
                name,
                then,
                else_,
            } => {
                let kind = if matches!(terminator, ProcessTerminator::Poll { .. }) {
                    process::PollKind::Poll
                } else {
                    process::PollKind::Repoll
                };
                let clients: Result<Vec<_>, _> =
                    clients.iter().map(|e| self.compile_expression(e)).collect();
                self.make_poll_process(
                    span,
                    kind,
                    label,
                    clients?,
                    name.clone(),
                    |pass| pass.compile_process(then),
                    |pass| pass.compile_process(else_),
                )?
            }
            ProcessTerminator::Submit {
                span,
                label,
                values,
            } => {
                let values: Result<Vec<_>, _> =
                    values.iter().map(|e| self.compile_expression(e)).collect();
                self.make_submit_process(span, label, values?)?
            }
            ProcessTerminator::Throw(span, label, expression) => {
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
            ProcessTerminator::If {
                span,
                branches,
                else_,
            } => {
                let else_proc = match else_ {
                    Some(proc) => self.compile_process(proc)?,
                    None => {
                        process::Process::terminal(process::Terminator::Unreachable(span.clone()))
                    }
                };
                self.compile_if_branches(branches, else_proc, |body, pass| {
                    pass.compile_process(body)
                })?
            }
            ProcessTerminator::Command(command) => self.compile_process_command(command, None)?,
            ProcessTerminator::Fallthrough(span) => match self.use_fallthrough(span) {
                Some(process) => process,
                None => Err(CompileError::MustEndProcess(span.clone()))?,
            },
        })
    }

    fn compile_process_command(
        &mut self,
        source: &ProcessCommand<Unresolved>,
        continuation: Option<(&Process<Unresolved>, usize)>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        match &source.target {
            CommandTarget::Global(global_name) => {
                let span = global_name.span.clone();
                let local_name = LocalName {
                    span: span.clone(),
                    string: ArcStr::from(global_name.to_string()),
                };
                let command = self.compile_command(&source.command, &local_name, continuation)?;
                Ok(process::Process::let_step(
                    span.clone(),
                    local_name,
                    None,
                    (),
                    Arc::new(process::Expression::Global(span, global_name.clone(), ())),
                    command,
                ))
            }
            CommandTarget::Local(name) => {
                let None = self.original_object_name else {
                    unreachable!("can't be in more than one command chain at once")
                };
                self.original_object_name = Some(name.clone());
                let command =
                    self.compile_command(&source.command, &LocalName::subject(), continuation)?;
                let None = self.original_object_name else {
                    unreachable!("command lowering did not leave alias mode")
                };
                Ok(process::Process::let_step(
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
                    command,
                ))
            }
            CommandTarget::Expression(expression) => {
                let span = expression.span();
                let command =
                    self.compile_command(&source.command, &LocalName::subject(), continuation)?;
                let expression = self.compile_expression(expression)?;
                Ok(process::Process::let_step(
                    span,
                    LocalName::subject(),
                    None,
                    (),
                    expression,
                    command,
                ))
            }
        }
    }

    fn prepare_process_command(
        &mut self,
        source: &ProcessCommand<Unresolved>,
    ) -> Result<ProcessLoweringFrame, CompileError> {
        let object_name = match &source.target {
            CommandTarget::Global(global_name) => LocalName {
                span: global_name.span.clone(),
                string: ArcStr::from(global_name.to_string()),
            },
            CommandTarget::Local(name) => {
                let None = self.original_object_name else {
                    unreachable!("can't be in more than one command chain at once")
                };
                self.original_object_name = Some(name.clone());
                LocalName::subject()
            }
            CommandTarget::Expression(_) => LocalName::subject(),
        };
        let frames = self.prepare_command_frames(&source.command.steps, &object_name)?;
        if matches!(source.command.steps.last(), Some(CommandStep::Continue(_))) {
            self.original_object_name = None;
        }
        let original_object_name = std::mem::take(&mut self.original_object_name);
        Ok(ProcessLoweringFrame::Command {
            target: source.target.clone(),
            object_name,
            original_object_name,
            frames,
        })
    }

    fn apply_process_lowering_frame(
        &mut self,
        frame: ProcessLoweringFrame,
        process: Arc<process::Process<(), Unresolved>>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        Ok(match frame {
            ProcessLoweringFrame::Let {
                span,
                pattern,
                value,
            } => self.compile_pattern_let(&pattern, &span, value, process)?,
            ProcessLoweringFrame::Command {
                target,
                object_name,
                original_object_name,
                frames,
            } => {
                let process = self.restore_object_name(original_object_name, process);
                let process = self.apply_command_frames(frames, &object_name, process)?;
                match target {
                    CommandTarget::Global(global_name) => {
                        let span = global_name.span.clone();
                        process::Process::let_step(
                            span.clone(),
                            object_name,
                            None,
                            (),
                            Arc::new(process::Expression::Global(span, global_name, ())),
                            process,
                        )
                    }
                    CommandTarget::Local(name) => process::Process::let_step(
                        name.span.clone(),
                        LocalName::subject(),
                        None,
                        (),
                        Arc::new(process::Expression::Variable(
                            name.span.clone(),
                            name,
                            (),
                            VariableUsage::Unknown,
                        )),
                        process,
                    ),
                    CommandTarget::Expression(expression) => {
                        let span = expression.span();
                        let expression = self.compile_expression(&expression)?;
                        process::Process::let_step(
                            span,
                            LocalName::subject(),
                            None,
                            (),
                            expression,
                            process,
                        )
                    }
                }
            }
        })
    }

    pub(crate) fn compile_command(
        &mut self,
        command: &Command<Unresolved>,
        object_name: &LocalName,
        continuation: Option<(&Process<Unresolved>, usize)>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        let frames = self.prepare_command_frames(&command.steps, object_name)?;
        if matches!(command.steps.last(), Some(CommandStep::Continue(_))) {
            self.original_object_name = None;
        }

        let process =
            self.compile_command_terminal(&command.terminator, object_name, continuation)?;
        self.apply_command_frames(frames, object_name, process)
    }

    fn prepare_command_frames(
        &mut self,
        steps: &[CommandStep<Unresolved>],
        object_name: &LocalName,
    ) -> Result<Vec<CommandLoweringFrame>, CompileError> {
        let mut frames = Vec::with_capacity(steps.len());
        for step in steps {
            match step {
                CommandStep::Send(span, argument) => {
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
                }
                CommandStep::Receive(span, pattern, vars) => {
                    frames.push(CommandLoweringFrame::Receive {
                        span: span.clone(),
                        pattern: pattern.clone(),
                        vars: vars.clone(),
                        original_object_name: self.original_object_name.clone(),
                    });
                }
                CommandStep::Signal(span, chosen) => {
                    frames.push(CommandLoweringFrame::Step(process::Step::Do {
                        span: span.clone(),
                        name: object_name.clone(),
                        usage: VariableUsage::Unknown,
                        typ: (),
                        command: process::Command::Signal(chosen.clone()),
                    }));
                }
                CommandStep::Continue(span) => {
                    frames.push(CommandLoweringFrame::Step(process::Step::Do {
                        span: span.clone(),
                        name: object_name.clone(),
                        usage: VariableUsage::Unknown,
                        typ: (),
                        command: process::Command::Continue,
                    }));
                }
                CommandStep::SendType(span, argument) => {
                    frames.push(CommandLoweringFrame::Step(process::Step::Do {
                        span: span.clone(),
                        name: object_name.clone(),
                        usage: VariableUsage::Unknown,
                        typ: (),
                        command: process::Command::SendType(argument.clone()),
                    }));
                }
                CommandStep::ReceiveType(span, parameter) => {
                    frames.push(CommandLoweringFrame::Step(process::Step::Do {
                        span: span.clone(),
                        name: object_name.clone(),
                        usage: VariableUsage::Unknown,
                        typ: (),
                        command: process::Command::ReceiveType(parameter.clone()),
                    }));
                }
                CommandStep::Try(span, label) => {
                    frames.push(CommandLoweringFrame::Try {
                        span: span.clone(),
                        catch_block: self.use_catch(span, label)?,
                    });
                }
                CommandStep::Default(span, expression) => {
                    frames.push(CommandLoweringFrame::Default {
                        span: span.clone(),
                        expression: self.compile_expression(expression)?,
                    });
                }
                CommandStep::Pipe(span, function) => {
                    self.disable_catches(CatchDisabledReason::DifferentProcess);
                    let function = self.compile_expression(function)?;
                    self.enable_catches();
                    frames.push(CommandLoweringFrame::Pipe {
                        span: span.clone(),
                        function,
                    });
                }
            }
        }
        Ok(frames)
    }

    fn apply_command_frames(
        &mut self,
        frames: Vec<CommandLoweringFrame>,
        object_name: &LocalName,
        mut process: Arc<process::Process<(), Unresolved>>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
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
        command: &CommandTerminator<Unresolved>,
        object_name: &LocalName,
        process_continuation: Option<(&Process<Unresolved>, usize)>,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        Ok(match command {
            CommandTerminator::Then(span) => {
                let original = std::mem::take(&mut self.original_object_name);
                let process = match process_continuation {
                    Some((source, index)) => self.compile_process_from(source, index)?,
                    None => match self.use_fallthrough(span) {
                        Some(process) => process,
                        None => Err(CompileError::MustEndProcess(span.clone()))?,
                    },
                };
                self.restore_object_name(original, process)
            }

            CommandTerminator::Link(span, expression) => {
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

            CommandTerminator::Case(span, CommandBranches(process_branches), else_branch) => {
                let original = std::mem::take(&mut self.original_object_name);
                let object_name = match &original {
                    None => object_name,
                    Some(original) => original,
                };

                let mut branches = Vec::new();
                let mut processes = Vec::new();

                let process = if let Some((source, index)) = process_continuation {
                    let process = self.compile_process_from(source, index)?;
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

            CommandTerminator::Break(span) => {
                self.original_object_name = None;
                process::Process::do_terminal(
                    span.clone(),
                    object_name.clone(),
                    VariableUsage::Unknown,
                    (),
                    process::TerminalCommand::Break,
                )
            }

            CommandTerminator::Begin {
                span,
                unfounded,
                label,
                body: command,
                continuation,
            } => {
                let nested_continuation = continuation
                    .as_deref()
                    .map(|continuation| (continuation, 0));
                let continuation = nested_continuation.or(process_continuation);
                let process = self.compile_command(command, object_name, continuation)?;
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

            CommandTerminator::Loop(span, label) => {
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
        })
    }

    pub(crate) fn compile_command_branch(
        &mut self,
        branch: &CommandBranch<Unresolved>,
        object_name: &LocalName,
    ) -> Result<Arc<process::Process<(), Unresolved>>, CompileError> {
        let mut process = match &branch.terminator {
            CommandBranchTerminator::Then(_, process) => self.compile_process(process)?,
            CommandBranchTerminator::BindThen(span, name, process) => {
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
            CommandBranchTerminator::Continue(span, source) => {
                let process = self.compile_process(source)?;
                process::Process::do_step(
                    span.clone(),
                    object_name.clone(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::Continue,
                    process,
                )
            }
        };
        for step in branch.steps.iter().rev() {
            process = match step {
                CommandBranchStep::Receive(span, pattern, vars) => self.compile_pattern_receive(
                    pattern,
                    0,
                    span,
                    object_name,
                    process,
                    vars.clone(),
                )?,
                CommandBranchStep::ReceiveType(span, parameter) => process::Process::do_step(
                    span.clone(),
                    object_name.clone(),
                    VariableUsage::Unknown,
                    (),
                    process::Command::ReceiveType(parameter.clone()),
                    process,
                ),
                CommandBranchStep::Try(span, label) => {
                    let catch_block = self.use_catch(span, label)?;
                    self.compile_try(span, object_name.clone(), catch_block, process)
                }
                CommandBranchStep::Default(span, expr) => {
                    let default_expr = self.compile_expression(expr)?;
                    self.compile_default(span, object_name.clone(), default_expr, process)
                }
            };
        }
        Ok(process)
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
        if pattern.is_continue() {
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
    fn as_name(&self) -> Option<(&Span, &LocalName, &Option<Type<S>>)> {
        if !self.steps.is_empty() {
            return None;
        }
        match &self.terminal {
            PatternTerminal::Name(span, name, annotation) => Some((span, name, annotation)),
            PatternTerminal::Continue(_) => None,
        }
    }

    fn is_continue(&self) -> bool {
        self.steps.is_empty() && matches!(self.terminal, PatternTerminal::Continue(_))
    }

    fn annotation(&self) -> Option<Type<S>>
    where
        S: Clone,
    {
        let mut annotation = match &self.terminal {
            PatternTerminal::Name(_, _, annotation) => annotation.clone(),
            PatternTerminal::Continue(span) => Some(Type::Break(span.clone())),
        };
        for step in self.steps.iter().rev() {
            annotation = match step {
                PatternStep::Receive(span, first, vars) => {
                    let first = first.annotation()?;
                    let rest = annotation?;
                    Some(Type::Pair(
                        span.clone(),
                        Box::new(first),
                        Box::new(rest),
                        vars.clone(),
                    ))
                }
                PatternStep::ReceiveType(span, parameter) => {
                    let rest = annotation?;
                    Some(Type::Exists(
                        span.clone(),
                        parameter.clone(),
                        Box::new(rest),
                    ))
                }
                PatternStep::Try(_, _) => None,
                PatternStep::Default(_, _) => annotation,
            };
        }
        annotation
    }
}

impl<S> Spanning for Pattern<S> {
    fn span(&self) -> Span {
        match self.steps.first() {
            Some(step) => step.span(),
            None => self.terminal.span(),
        }
    }
}

impl<S> Spanning for PatternStep<S> {
    fn span(&self) -> Span {
        match self {
            Self::Receive(span, _, _)
            | Self::ReceiveType(span, _)
            | Self::Try(span, _)
            | Self::Default(span, _) => span.clone(),
        }
    }
}

impl<S> PatternTerminal<S> {
    fn span(&self) -> Span {
        match self {
            Self::Name(span, _, _) | Self::Continue(span) => span.clone(),
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
        match self.steps.first() {
            Some(step) => step.span(),
            None => self.terminator.span(),
        }
    }
}

impl<S> Spanning for ConstructStep<S> {
    fn span(&self) -> Span {
        match self {
            Self::Send(span, _)
            | Self::Receive(span, _, _)
            | Self::Signal(span, _)
            | Self::SendType(span, _)
            | Self::ReceiveType(span, _) => span.clone(),
        }
    }
}

impl<S> ConstructTerminator<S> {
    fn span(&self) -> Span {
        match self {
            Self::Then(expression) => expression.span(),
            Self::Case(span, _, _)
            | Self::Break(span)
            | Self::Begin { span, .. }
            | Self::Loop(span, _)
            | Self::Submit { span, .. } => span.clone(),
        }
    }
}

impl<S> Spanning for ConstructBranch<S> {
    fn span(&self) -> Span {
        match self.steps.first() {
            Some(step) => step.span(),
            None => self.terminator.span(),
        }
    }
}

impl<S> Spanning for ConstructBranchStep<S> {
    fn span(&self) -> Span {
        match self {
            Self::Receive(span, _, _) | Self::ReceiveType(span, _) => span.clone(),
        }
    }
}

impl<S> ConstructBranchTerminator<S> {
    fn span(&self) -> Span {
        match self {
            Self::Then(span, _) => span.clone(),
        }
    }
}

impl<S> Spanning for Apply<S> {
    fn span(&self) -> Span {
        match self.steps.first() {
            Some(step) => step.span(),
            None => self.terminator.span(),
        }
    }
}

impl<S> Spanning for ApplyStep<S> {
    fn span(&self) -> Span {
        match self {
            Self::Send(span, _)
            | Self::Signal(span, _)
            | Self::SendType(span, _)
            | Self::Try(span, _)
            | Self::Default(span, _)
            | Self::Pipe(span, _) => span.clone(),
        }
    }
}

impl<S> ApplyTerminator<S> {
    fn span(&self) -> Span {
        match self {
            Self::Noop(span)
            | Self::Case(span, _, _)
            | Self::Begin { span, .. }
            | Self::Loop(span, _) => span.clone(),
        }
    }
}

impl<S> Spanning for ApplyBranch<S> {
    fn span(&self) -> Span {
        match self.steps.first() {
            Some(step) => step.span(),
            None => self.terminator.span(),
        }
    }
}

impl<S> Spanning for ApplyBranchStep<S> {
    fn span(&self) -> Span {
        match self {
            Self::Receive(span, _, _)
            | Self::ReceiveType(span, _)
            | Self::Try(span, _)
            | Self::Default(span, _) => span.clone(),
        }
    }
}

impl<S> ApplyBranchTerminator<S> {
    fn span(&self) -> Span {
        match self {
            Self::Then(span, _, _) | Self::Continue(span, _) => span.clone(),
        }
    }
}

impl<S> Process<S> {
    pub fn fallthrough(span: Span) -> Self {
        Self {
            steps: Vec::new(),
            terminator: ProcessTerminator::Fallthrough(span),
        }
    }
}

impl<S> Spanning for Process<S> {
    fn span(&self) -> Span {
        match self.steps.first() {
            Some(step) => step.span(),
            None => self.terminator.span(),
        }
    }
}

impl<S> Spanning for ProcessStep<S> {
    fn span(&self) -> Span {
        match self {
            Self::Let { span, .. } | Self::Catch { span, .. } | Self::If { span, .. } => {
                span.clone()
            }
            Self::Command(command) => command.span.clone(),
        }
    }
}

impl<S> Spanning for ProcessTerminator<S> {
    fn span(&self) -> Span {
        match self {
            Self::Poll { span, .. }
            | Self::Repoll { span, .. }
            | Self::Submit { span, .. }
            | Self::If { span, .. }
            | Self::Throw(span, ..)
            | Self::Fallthrough(span) => span.clone(),
            Self::Command(command) => command.span.clone(),
        }
    }
}

impl<S> CommandTerminator<S> {
    pub fn span(&self) -> Span {
        match self {
            Self::Then(span)
            | Self::Link(span, _)
            | Self::Case(span, _, _)
            | Self::Break(span)
            | Self::Begin { span, .. }
            | Self::Loop(span, _) => span.clone(),
        }
    }
}

impl<S> Spanning for Command<S> {
    fn span(&self) -> Span {
        match self.steps.first() {
            Some(step) => step.span(),
            None => self.terminator.span(),
        }
    }
}

impl<S> Spanning for CommandStep<S> {
    fn span(&self) -> Span {
        match self {
            Self::Send(span, _)
            | Self::Receive(span, _, _)
            | Self::Signal(span, _)
            | Self::Continue(span)
            | Self::SendType(span, _)
            | Self::ReceiveType(span, _)
            | Self::Try(span, _)
            | Self::Default(span, _)
            | Self::Pipe(span, _) => span.clone(),
        }
    }
}

impl<S> Spanning for CommandBranch<S> {
    fn span(&self) -> Span {
        match self.steps.first() {
            Some(step) => step.span(),
            None => self.terminator.span(),
        }
    }
}

impl<S> Spanning for CommandBranchStep<S> {
    fn span(&self) -> Span {
        match self {
            Self::Receive(span, _, _)
            | Self::ReceiveType(span, _)
            | Self::Try(span, _)
            | Self::Default(span, _) => span.clone(),
        }
    }
}

impl<S> CommandBranchTerminator<S> {
    pub fn span(&self) -> Span {
        match self {
            Self::Then(span, _) | Self::BindThen(span, _, _) | Self::Continue(span, _) => {
                span.clone()
            }
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
    if let Some((span, name, _)) = pattern.as_name() {
        return Some(Expression::Variable(span.clone(), name.clone()));
    }
    Some(Expression::Construction(
        pattern.span(),
        construct_from_pattern(pattern)?,
    ))
}

fn construct_from_pattern(pattern: &Pattern<Unresolved>) -> Option<Construct<Unresolved>> {
    let terminator = match &pattern.terminal {
        PatternTerminal::Name(span, name, _) => {
            ConstructTerminator::Then(Box::new(Expression::Variable(span.clone(), name.clone())))
        }
        PatternTerminal::Continue(span) => ConstructTerminator::Break(span.clone()),
    };
    let mut steps = Vec::with_capacity(pattern.steps.len());
    for step in &pattern.steps {
        match step {
            PatternStep::Receive(span, payload, _) => steps.push(ConstructStep::Send(
                span.clone(),
                Box::new(pattern_to_expression(payload)?),
            )),
            PatternStep::ReceiveType(..) | PatternStep::Try(..) | PatternStep::Default(..) => {
                return None;
            }
        }
    }
    Some(Construct { steps, terminator })
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
    let mut payload = construct_from_pattern(pattern)?;
    payload
        .steps
        .insert(0, ConstructStep::Signal(span.clone(), variant.clone()));
    let reconstruction = Expression::Construction(span.clone(), payload);
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
    Process {
        steps: Vec::new(),
        terminator: ProcessTerminator::Command(ProcessCommand {
            span: span.clone(),
            target: CommandTarget::Local(LocalName::result()),
            command: Command {
                steps: Vec::new(),
                terminator: CommandTerminator::Link(span, Box::new(expr.clone())),
            },
        }),
    }
}

#[cfg(test)]
mod flat_ir_tests {
    use super::*;

    #[test]
    fn lowers_long_command_chain_without_recursive_ir_walk() {
        const STEP_COUNT: usize = 20_000;
        let subject = LocalName::from(literal!("subject"));
        let chosen = LocalName::from(literal!("next"));
        let mut steps = Vec::with_capacity(STEP_COUNT);
        for _ in 0..STEP_COUNT {
            steps.push(CommandStep::Signal(Span::None, chosen.clone()));
        }
        let command = Command {
            steps,
            terminator: CommandTerminator::Break(Span::None),
        };

        let mut context = Context::new();
        let lowered = context.compile_command(&command, &subject, None).unwrap();
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
    }

    #[test]
    fn lowers_long_high_level_sequences_without_recursive_spines() {
        const STEP_COUNT: usize = 20_000;
        let value = LocalName::from(literal!("value"));
        let binding = LocalName::from(literal!("binding"));
        let exit = LocalName::from(literal!("exit"));
        let chosen = LocalName::from(literal!("next"));

        let process = Process {
            steps: (0..STEP_COUNT)
                .map(|_| ProcessStep::Let {
                    span: Span::None,
                    pattern: Pattern {
                        steps: Vec::new(),
                        terminal: PatternTerminal::Name(Span::None, binding.clone(), None),
                    },
                    value: Box::new(Expression::Variable(Span::None, value.clone())),
                })
                .collect(),
            terminator: ProcessTerminator::Command(ProcessCommand {
                span: Span::None,
                target: CommandTarget::Local(exit),
                command: Command {
                    steps: Vec::new(),
                    terminator: CommandTerminator::Break(Span::None),
                },
            }),
        };
        let lowered = Context::new().compile_process(&process).unwrap();
        assert_eq!(lowered.steps.len(), STEP_COUNT + 1);

        let construct = Construct {
            steps: (0..STEP_COUNT)
                .map(|_| ConstructStep::Signal(Span::None, chosen.clone()))
                .collect(),
            terminator: ConstructTerminator::Break(Span::None),
        };
        let lowered = Context::new().compile_construct(&construct).unwrap();
        assert_eq!(lowered.steps.len(), STEP_COUNT);

        let apply = Apply {
            steps: (0..STEP_COUNT)
                .map(|_| ApplyStep::Signal(Span::None, chosen.clone()))
                .collect(),
            terminator: ApplyTerminator::Noop(Span::None),
        };
        let lowered = Context::new().compile_apply(&apply).unwrap();
        assert_eq!(lowered.steps.len(), STEP_COUNT);

        let pattern = Pattern {
            steps: (0..STEP_COUNT)
                .map(|_| {
                    PatternStep::ReceiveType(
                        Span::None,
                        TypeParameter::any(LocalName::from(literal!("a"))),
                    )
                })
                .collect(),
            terminal: PatternTerminal::Name(Span::None, binding, None),
        };
        let tail = process::Process::do_terminal(
            Span::None,
            LocalName::result(),
            VariableUsage::Unknown,
            (),
            process::TerminalCommand::Break,
        );
        let lowered = Context::new()
            .compile_pattern_helper(&pattern, 0, tail)
            .unwrap();
        assert_eq!(lowered.steps.len(), STEP_COUNT + 1);
    }
}
