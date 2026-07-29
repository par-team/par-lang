use super::super::language::{GlobalName, LocalName, TypeParameter, Unresolved};
use crate::frontend_impl::types::visit::Polarity;
use crate::frontend_impl::types::{TypeDefs, TypeError, visit};
use crate::location::{Span, Spanning};
use arcstr::ArcStr;
use im::HashSet;
use num_bigint::BigInt;
use par_runtime::primitive::Primitive;
use par_runtime::readback::Number;
use std::collections::BTreeMap;
use std::fmt::{Debug, Formatter};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LoopId(u64);

static NEXT_LOOP_ID: AtomicU64 = AtomicU64::new(0);

impl LoopId {
    pub fn new() -> Self {
        let id = NEXT_LOOP_ID.fetch_add(1, Ordering::SeqCst);
        Self(id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PrimitiveType {
    Nat,
    Int,
    Float,
    String,
    Char,
    Byte,
    Bytes,
}

#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct Ignored<T>(pub(crate) T);

impl<T: Default> Default for Ignored<T> {
    fn default() -> Self {
        Self(T::default())
    }
}

impl<T> PartialEq for Ignored<T> {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl<T> Eq for Ignored<T> {}

impl<T> Hash for Ignored<T> {
    fn hash<H: Hasher>(&self, _state: &mut H) {}
}

#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct NamedTypeDisplay<S> {
    pub(crate) name: GlobalName<S>,
    pub(crate) args: Vec<Type<S>>,
    pub(crate) dual: bool,
}

impl<S> NamedTypeDisplay<S> {
    pub(crate) fn new(name: GlobalName<S>, args: Vec<Type<S>>, dual: bool) -> Self {
        Self { name, args, dual }
    }

    pub(crate) fn dual(self) -> Self {
        Self {
            dual: !self.dual,
            ..self
        }
    }
}

pub(crate) fn get_primitive_type<S: Clone>(primitive: &Primitive) -> Type<S> {
    match primitive {
        Primitive::Number(Number::Zero) => Type::nat(),
        Primitive::Number(Number::Int(n)) if n >= &BigInt::ZERO => Type::nat(),
        Primitive::Number(Number::Int(_)) => Type::int(),
        Primitive::Number(Number::Float(_)) => Type::float(),
        Primitive::String(s) if is_single_char(s.as_str()) => Type::char(),
        Primitive::String(_) => Type::string(),
        Primitive::Bytes(b) if b.len() == 1 => Type::byte(),
        Primitive::Bytes(_) => Type::bytes(),
    }
}

pub(crate) fn is_single_char(string: &str) -> bool {
    let mut chars = string.chars();
    chars.next().is_some() && chars.next().is_none()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operation {
    Send,
    Receive {
        #[allow(unused)]
        generics: usize,
    },
    Signal,
    Case,
    Break,
    Continue,
    Begin,
    Loop,
    SendType,
    ReceiveType,
}

struct HoleConstraints<S> {
    upper_bounds: Vec<Type<S>>,
    lower_bounds: Vec<Type<S>>,
}

#[derive(Clone)]
pub struct Hole<S>(Arc<Mutex<HoleConstraints<S>>>);

impl<S: Debug> Debug for Hole<S> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let constraints = self.0.lock().unwrap();
        f.debug_struct("Hole")
            .field("upper_bounds", &constraints.upper_bounds)
            .field("lower_bounds", &constraints.lower_bounds)
            .finish()
    }
}

impl<S> PartialEq for Hole<S> {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}
impl<S> Eq for Hole<S> {}

impl<S> Hash for Hole<S> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // All holes are considered equal for hashing purposes
        0.hash(state);
    }
}

impl<S: Clone> Hole<S> {
    pub fn add_lower_bound(&self, bound: Type<S>) {
        let mut constraints = self.0.lock().unwrap();
        constraints.lower_bounds.push(bound);
    }

    pub fn add_upper_bound(&self, bound: Type<S>) {
        let mut constraints = self.0.lock().unwrap();
        constraints.upper_bounds.push(bound);
    }

    pub fn get_constraints(&self) -> (Vec<Type<S>>, Vec<Type<S>>) {
        let constraints = self.0.lock().unwrap();
        (
            constraints.lower_bounds.clone(),
            constraints.upper_bounds.clone(),
        )
    }

    fn map_global_names<T, E>(
        &self,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Hole<T>, E> {
        let constraints = self.0.lock().unwrap();
        let lower_bounds = constraints
            .lower_bounds
            .clone()
            .into_iter()
            .map(|typ| typ.map_global_names(f))
            .collect::<Result<Vec<_>, _>>()?;
        let upper_bounds = constraints
            .upper_bounds
            .clone()
            .into_iter()
            .map(|typ| typ.map_global_names(f))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Hole(Arc::new(Mutex::new(HoleConstraints {
            upper_bounds,
            lower_bounds,
        }))))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type<S> {
    Primitive(Span, PrimitiveType),
    DualPrimitive(Span, PrimitiveType),
    Var(Span, LocalName),
    DualVar(Span, LocalName),
    Name(Span, GlobalName<S>, Vec<Self>),
    DualName(Span, GlobalName<S>, Vec<Self>),
    Box(Span, Box<Self>),
    DualBox(Span, Box<Self>),
    Pair(Span, Box<Self>, Box<Self>, Vec<TypeParameter>),
    Function(Span, Box<Self>, Box<Self>, Vec<TypeParameter>),
    Either(Span, BTreeMap<LocalName, Self>),
    Choice(Span, BTreeMap<LocalName, Self>),
    Break(Span),
    Continue(Span),
    Recursive {
        span: Span,
        // The ascendents of the type (denoted by unique IDs generated for each loop):
        // If you `begin` on a `recursive`, and it expands, so its `self`s get replaced by new
        // `recursive`s, these new `recursive`s will have as their *ascendent* the original `recursive`.
        // This is for totality checking.
        asc: HashSet<LoopId>,
        label: Option<LocalName>,
        body: Box<Self>,
        display_hint: Ignored<Option<NamedTypeDisplay<S>>>,
    },
    Iterative {
        span: Span,
        asc: HashSet<LoopId>,
        label: Option<LocalName>,
        body: Box<Self>,
        display_hint: Ignored<Option<NamedTypeDisplay<S>>>,
    },
    Self_(Span, Option<LocalName>),
    DualSelf(Span, Option<LocalName>),
    Exists(Span, TypeParameter, Box<Self>),
    Forall(Span, TypeParameter, Box<Self>),
    Hole(Span, LocalName, Hole<S>),
    DualHole(Span, LocalName, Hole<S>),
    /// A type that could not be determined due to a type error.
    /// Self-dual, assignable to/from anything, non-linear.
    /// Used to suppress cascading errors during multi-error type checking.
    Fail(Span),
}

fn current_depth_from_children(children: impl IntoIterator<Item = usize>) -> usize {
    1 + children.into_iter().max().unwrap_or(0)
}

fn flattened_depth_with_tail(
    tail_depth: usize,
    side_children: impl IntoIterator<Item = usize>,
) -> usize {
    match side_children.into_iter().max() {
        Some(side_depth) => tail_depth.max(1 + side_depth),
        None => tail_depth.max(1),
    }
}

fn flattened_depth_from_side_children(children: impl IntoIterator<Item = usize>) -> usize {
    match children.into_iter().max() {
        Some(depth) => 1 + depth,
        None => 1,
    }
}

fn flattened_depth_from_branches(children: impl IntoIterator<Item = usize>) -> usize {
    let mut deepest = 0;
    let mut second_deepest = 0;
    let mut saw_any = false;

    for depth in children {
        saw_any = true;
        if depth >= deepest {
            second_deepest = deepest;
            deepest = depth;
        } else if depth > second_deepest {
            second_deepest = depth;
        }
    }

    if !saw_any {
        1
    } else {
        deepest.max(1 + second_deepest)
    }
}

#[allow(unused)]
impl<S: Clone> Type<S> {
    pub fn current_depth(&self) -> usize {
        match self {
            Self::Primitive(..)
            | Self::DualPrimitive(..)
            | Self::Var(..)
            | Self::DualVar(..)
            | Self::Break(..)
            | Self::Continue(..)
            | Self::Self_(..)
            | Self::DualSelf(..)
            | Self::Hole(..)
            | Self::DualHole(..)
            | Self::Fail(..) => 1,
            Self::Name(_, _, args) | Self::DualName(_, _, args) => {
                current_depth_from_children(args.iter().map(|arg| arg.current_depth()))
            }
            Self::Box(_, inner)
            | Self::DualBox(_, inner)
            | Self::Exists(_, _, inner)
            | Self::Forall(_, _, inner) => current_depth_from_children([inner.current_depth()]),
            Self::Pair(_, left, right, _) | Self::Function(_, left, right, _) => {
                current_depth_from_children([left.current_depth(), right.current_depth()])
            }
            Self::Either(_, branches) | Self::Choice(_, branches) => {
                current_depth_from_children(branches.values().map(|branch| branch.current_depth()))
            }
            Self::Recursive { body, .. } | Self::Iterative { body, .. } => {
                current_depth_from_children([body.current_depth()])
            }
        }
    }

    pub fn flattened_depth(&self) -> usize {
        match self {
            Self::Primitive(..)
            | Self::DualPrimitive(..)
            | Self::Var(..)
            | Self::DualVar(..)
            | Self::Break(..)
            | Self::Continue(..)
            | Self::Self_(..)
            | Self::DualSelf(..)
            | Self::Hole(..)
            | Self::DualHole(..)
            | Self::Fail(..) => 1,
            Self::Name(_, _, args) | Self::DualName(_, _, args) => {
                flattened_depth_from_side_children(args.iter().map(|arg| arg.flattened_depth()))
            }
            Self::Box(_, inner)
            | Self::DualBox(_, inner)
            | Self::Exists(_, _, inner)
            | Self::Forall(_, _, inner) => inner.flattened_depth(),
            Self::Pair(_, left, right, _) | Self::Function(_, left, right, _) => {
                flattened_depth_with_tail(right.flattened_depth(), [left.flattened_depth()])
            }
            Self::Either(_, branches) | Self::Choice(_, branches) => flattened_depth_from_branches(
                branches.values().map(|branch| branch.flattened_depth()),
            ),
            Self::Recursive { body, .. } | Self::Iterative { body, .. } => body.flattened_depth(),
        }
    }

    pub fn nat() -> Self {
        Self::Primitive(Span::None, PrimitiveType::Nat)
    }

    pub fn int() -> Self {
        Self::Primitive(Span::None, PrimitiveType::Int)
    }

    pub fn float() -> Self {
        Self::Primitive(Span::None, PrimitiveType::Float)
    }

    pub fn string() -> Self {
        Self::Primitive(Span::None, PrimitiveType::String)
    }

    pub fn char() -> Self {
        Self::Primitive(Span::None, PrimitiveType::Char)
    }

    pub fn byte() -> Self {
        Self::Primitive(Span::None, PrimitiveType::Byte)
    }

    pub fn bytes() -> Self {
        Self::Primitive(Span::None, PrimitiveType::Bytes)
    }

    pub fn var(name: &'static str) -> Self {
        Self::Var(
            Span::None,
            LocalName {
                span: Span::None,
                string: ArcStr::from(name),
            },
        )
    }

    pub fn hole(name: LocalName) -> (Self, Hole<S>) {
        let constraints = Arc::new(Mutex::new(HoleConstraints {
            upper_bounds: Vec::new(),
            lower_bounds: Vec::new(),
        }));
        let hole = Hole(constraints);
        let typ = Self::Hole(Span::None, name, hole.clone());
        (typ, hole)
    }

    pub fn box_(t: Self) -> Self {
        Self::Box(Span::None, Box::new(t))
    }

    pub fn pair(t: Self, u: Self) -> Self {
        Self::Pair(Span::None, Box::new(t), Box::new(u), vec![])
    }

    pub fn generic_pair(vars: Vec<&'static str>, t: Self, u: Self) -> Self {
        Self::Pair(
            Span::None,
            Box::new(t),
            Box::new(u),
            vars.into_iter()
                .map(|name| {
                    TypeParameter::any(LocalName {
                        span: Span::None,
                        string: ArcStr::from(name),
                    })
                })
                .collect(),
        )
    }

    pub fn function(t: Self, u: Self) -> Self {
        Self::Function(Span::None, Box::new(t), Box::new(u), vec![])
    }

    pub fn generic_function(vars: Vec<&'static str>, t: Self, u: Self) -> Self {
        Self::Function(
            Span::None,
            Box::new(t),
            Box::new(u),
            vars.into_iter()
                .map(|name| {
                    TypeParameter::any(LocalName {
                        span: Span::None,
                        string: ArcStr::from(name),
                    })
                })
                .collect(),
        )
    }

    pub(crate) fn with_type_parameters<R>(
        defs: &TypeDefs<S>,
        params: &[TypeParameter],
        f: impl FnOnce(&TypeDefs<S>) -> Result<R, TypeError<S>>,
    ) -> Result<R, TypeError<S>>
    where
        S: Clone + Eq + std::hash::Hash,
    {
        if params.is_empty() {
            f(defs)
        } else {
            let mut defs = defs.clone();
            defs.extend_vars(params.iter().cloned());
            f(&defs)
        }
    }

    pub(crate) fn with_type_parameter<R>(
        defs: &TypeDefs<S>,
        param: &TypeParameter,
        f: impl FnOnce(&TypeDefs<S>) -> Result<R, TypeError<S>>,
    ) -> Result<R, TypeError<S>>
    where
        S: Clone + Eq + std::hash::Hash,
    {
        let mut defs = defs.clone();
        defs.insert_var(param.clone());
        f(&defs)
    }

    pub fn either(branches: Vec<(&'static str, Self)>) -> Self {
        Self::Either(
            Span::None,
            branches
                .into_iter()
                .map(|(name, typ)| {
                    (
                        LocalName {
                            span: Span::None,
                            string: ArcStr::from(name),
                        },
                        typ,
                    )
                })
                .collect(),
        )
    }

    pub fn choice(branches: Vec<(&'static str, Self)>) -> Self {
        Self::Choice(
            Span::None,
            branches
                .into_iter()
                .map(|(name, typ)| {
                    (
                        LocalName {
                            span: Span::None,
                            string: ArcStr::from(name),
                        },
                        typ,
                    )
                })
                .collect(),
        )
    }

    pub fn break_() -> Self {
        Self::Break(Span::None)
    }

    pub fn continue_() -> Self {
        Self::Continue(Span::None)
    }

    pub fn recursive(label: Option<&'static str>, body: Self) -> Self {
        Self::Recursive {
            span: Span::None,
            asc: HashSet::new(),
            label: label.map(|label| LocalName {
                span: Span::None,
                string: ArcStr::from(label),
            }),
            body: Box::new(body),
            display_hint: Default::default(),
        }
    }

    pub fn iterative(label: Option<&'static str>, body: Self) -> Self {
        Self::Iterative {
            span: Span::None,
            asc: HashSet::new(),
            label: label.map(|label| LocalName {
                span: Span::None,
                string: ArcStr::from(label),
            }),
            body: Box::new(body),
            display_hint: Default::default(),
        }
    }

    pub fn iterative_box_choice(
        label: Option<&'static str>,
        branches: Vec<(&'static str, Self)>,
    ) -> Self {
        Self::iterative(label, Self::box_(Self::choice(branches)))
    }

    pub fn self_(label: Option<&'static str>) -> Self {
        Self::Self_(
            Span::None,
            label.map(|label| LocalName {
                span: Span::None,
                string: ArcStr::from(label),
            }),
        )
    }

    pub fn forall(var: &'static str, body: Self) -> Self {
        Self::Forall(
            Span::None,
            TypeParameter::any(LocalName {
                span: Span::None,
                string: ArcStr::from(var),
            }),
            Box::new(body),
        )
    }

    pub fn is_fixpoint(&self) -> bool {
        matches!(self, Self::Recursive { .. } | Self::Iterative { .. })
    }

    pub fn size(&self, defs: &TypeDefs<S>) -> Result<u32, TypeError<S>>
    where
        S: Eq + std::hash::Hash,
    {
        Ok(match self {
            Self::Primitive(_, _) | Self::DualPrimitive(_, _) => 1,
            Self::Var(_, _) | Self::DualVar(_, _) => 1,
            Self::Name(span, name, args) => defs.get(span, name, args)?.size(defs)?,
            Self::DualName(span, name, args) => defs.get_dual(span, name, args)?.size(defs)?,
            Self::Box(_, inner) | Self::DualBox(_, inner) => 1 + inner.size(defs)?,
            Self::Pair(_, left, right, _) => 1 + left.size(defs)? + right.size(defs)?,
            Self::Function(_, input, output, _) => 1 + input.size(defs)? + output.size(defs)?,
            Self::Either(_, branches) | Self::Choice(_, branches) => {
                let mut res: u32 = 1;
                for branch in branches.values() {
                    res += branch.size(defs)?
                }
                res
            }
            Self::Break(_) | Self::Continue(_) => 1,
            Self::Recursive { body, .. } | Self::Iterative { body, .. } => 1 + body.size(defs)?,
            Self::Self_(_, _) | Self::DualSelf(_, _) => 1,
            Self::Exists(_, _, body) | Self::Forall(_, _, body) => 1 + body.size(defs)?,
            Self::Hole(_, _, _) | Self::DualHole(_, _, _) => 1,
            Self::Fail(_) => 1,
        })
    }
}

impl Type<Unresolved> {
    pub fn name(module: Option<&'static str>, primary: &'static str, args: Vec<Self>) -> Self {
        Self::Name(Span::None, GlobalName::external(module, primary), args)
    }
}

impl<S: Clone> Type<S> {
    pub fn map_global_names<T, E>(
        self,
        f: &mut impl FnMut(GlobalName<S>) -> Result<GlobalName<T>, E>,
    ) -> Result<Type<T>, E> {
        Ok(match self {
            Self::Primitive(span, primitive) => Type::Primitive(span, primitive),
            Self::DualPrimitive(span, primitive) => Type::DualPrimitive(span, primitive),
            Self::Var(span, name) => Type::Var(span, name),
            Self::DualVar(span, name) => Type::DualVar(span, name),
            Self::Break(span) => Type::Break(span),
            Self::Continue(span) => Type::Continue(span),
            Self::Self_(span, label) => Type::Self_(span, label),
            Self::DualSelf(span, label) => Type::DualSelf(span, label),
            Self::Name(span, name, args) => {
                let mapped_name = f(name)?;
                let mapped_args = args
                    .into_iter()
                    .map(|arg| arg.map_global_names(f))
                    .collect::<Result<Vec<_>, _>>()?;
                Type::Name(span, mapped_name, mapped_args)
            }
            Self::DualName(span, name, args) => {
                let mapped_name = f(name)?;
                let mapped_args = args
                    .into_iter()
                    .map(|arg| arg.map_global_names(f))
                    .collect::<Result<Vec<_>, _>>()?;
                Type::DualName(span, mapped_name, mapped_args)
            }
            Self::Box(span, inner) => {
                let mapped_inner = Box::new(inner.map_global_names(f)?);
                Type::Box(span, mapped_inner)
            }
            Self::DualBox(span, inner) => {
                let mapped_inner = Box::new(inner.map_global_names(f)?);
                Type::DualBox(span, mapped_inner)
            }
            Self::Pair(span, left, right, vars) => {
                let left = Box::new(left.map_global_names(f)?);
                let right = Box::new(right.map_global_names(f)?);
                Type::Pair(span, left, right, vars)
            }
            Self::Function(span, left, right, vars) => {
                let left = Box::new(left.map_global_names(f)?);
                let right = Box::new(right.map_global_names(f)?);
                Type::Function(span, left, right, vars)
            }
            Self::Either(span, branches) => {
                let mapped = branches
                    .into_iter()
                    .map(|(name, branch)| Ok((name, branch.map_global_names(f)?)))
                    .collect::<Result<BTreeMap<_, _>, E>>()?;
                Type::Either(span, mapped)
            }
            Self::Choice(span, branches) => {
                let mapped = branches
                    .into_iter()
                    .map(|(name, branch)| Ok((name, branch.map_global_names(f)?)))
                    .collect::<Result<BTreeMap<_, _>, E>>()?;
                Type::Choice(span, mapped)
            }
            Self::Recursive {
                span,
                asc,
                label,
                body,
                display_hint,
            } => Type::Recursive {
                span,
                asc,
                label,
                body: Box::new(body.map_global_names(f)?),
                display_hint: Ignored(match display_hint.0 {
                    Some(display_hint) => Some(NamedTypeDisplay {
                        name: f(display_hint.name)?,
                        args: display_hint
                            .args
                            .into_iter()
                            .map(|arg| arg.map_global_names(f))
                            .collect::<Result<Vec<_>, _>>()?,
                        dual: display_hint.dual,
                    }),
                    None => None,
                }),
            },
            Self::Iterative {
                span,
                asc,
                label,
                body,
                display_hint,
            } => Type::Iterative {
                span,
                asc,
                label,
                body: Box::new(body.map_global_names(f)?),
                display_hint: Ignored(match display_hint.0 {
                    Some(display_hint) => Some(NamedTypeDisplay {
                        name: f(display_hint.name)?,
                        args: display_hint
                            .args
                            .into_iter()
                            .map(|arg| arg.map_global_names(f))
                            .collect::<Result<Vec<_>, _>>()?,
                        dual: display_hint.dual,
                    }),
                    None => None,
                }),
            },
            Self::Exists(span, name, body) => {
                Type::Exists(span, name, Box::new(body.map_global_names(f)?))
            }
            Self::Forall(span, name, body) => {
                Type::Forall(span, name, Box::new(body.map_global_names(f)?))
            }
            Self::Hole(span, name, hole) => Type::Hole(span, name, hole.map_global_names(f)?),
            Self::DualHole(span, name, hole) => {
                Type::DualHole(span, name, hole.map_global_names(f)?)
            }
            Self::Fail(span) => Type::Fail(span),
        })
    }
}

impl<S> Spanning for Type<S> {
    fn span(&self) -> Span {
        match self {
            Self::Primitive(span, _)
            | Self::DualPrimitive(span, _)
            | Self::Var(span, _)
            | Self::DualVar(span, _)
            | Self::Name(span, _, _)
            | Self::DualName(span, _, _)
            | Self::Box(span, _)
            | Self::DualBox(span, _)
            | Self::Pair(span, _, _, _)
            | Self::Function(span, _, _, _)
            | Self::Either(span, _)
            | Self::Choice(span, _)
            | Self::Break(span)
            | Self::Continue(span)
            | Self::Recursive { span, .. }
            | Self::Iterative { span, .. }
            | Self::Self_(span, _)
            | Self::DualSelf(span, _)
            | Self::Exists(span, _, _)
            | Self::Forall(span, _, _)
            | Self::Hole(span, _, _)
            | Self::DualHole(span, _, _)
            | Self::Fail(span) => span.clone(),
        }
    }
}

impl<S> Type<S> {
    pub(crate) fn display_hint(&self) -> Option<&NamedTypeDisplay<S>> {
        match self {
            Self::Recursive { display_hint, .. } | Self::Iterative { display_hint, .. } => {
                display_hint.0.as_ref()
            }
            _ => None,
        }
    }

    pub(crate) fn with_display_hint(self, display_hint: NamedTypeDisplay<S>) -> Self {
        match self {
            Self::Recursive {
                span,
                asc,
                label,
                body,
                ..
            } => Self::Recursive {
                span,
                asc,
                label,
                body,
                display_hint: Ignored(Some(display_hint)),
            },
            Self::Iterative {
                span,
                asc,
                label,
                body,
                ..
            } => Self::Iterative {
                span,
                asc,
                label,
                body,
                display_hint: Ignored(Some(display_hint)),
            },
            typ => typ,
        }
    }
}

impl<S: Clone + Eq + std::hash::Hash> Type<S> {
    pub fn remove_asc(&mut self, defs: &TypeDefs<S>) -> Result<(), TypeError<S>> {
        fn inner<S: Clone + Eq + std::hash::Hash>(
            typ: &mut Type<S>,
            polarity: Polarity,
            defs: &TypeDefs<S>,
        ) -> Result<(), TypeError<S>> {
            match (typ, polarity) {
                (Type::Recursive { asc, body, .. }, Polarity::Positive | Polarity::Neither)
                | (Type::Iterative { asc, body, .. }, Polarity::Negative | Polarity::Neither) => {
                    asc.clear();
                    inner(body, polarity, defs)?;
                }
                (
                    Type::Iterative {
                        span, asc, body, ..
                    },
                    Polarity::Positive | Polarity::Both,
                )
                | (
                    Type::Recursive {
                        span, asc, body, ..
                    },
                    Polarity::Negative | Polarity::Both,
                ) => {
                    if !asc.is_empty() {
                        return Err(TypeError::CannotUnrollAscendantIterative(
                            span.clone(),
                            None,
                        ));
                    }
                    inner(body, polarity, defs)?;
                }
                (typ, polarity) => visit::continue_mut_polarized(
                    typ,
                    polarity,
                    defs,
                    |child: &mut Type<S>, polarity| inner(child, polarity, defs),
                )?,
            }
            Ok(())
        }
        inner(self, Polarity::Positive, defs)
    }
}
