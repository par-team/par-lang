use crate::frontend_impl::language::LocalName;
use crate::frontend_impl::language::TypeConstraint;
use crate::frontend_impl::language::TypeParameter;
use crate::frontend_impl::types::assignability::SubtypeResult::{Compatible, Cycle, Incompatible};
use crate::frontend_impl::types::{
    PrimitiveType, Type, TypeDefs, TypeError, TypePath, TypePathSegment,
};
use crate::location::Span;
use indexmap::IndexSet;
use std::cmp::max;
use std::collections::BTreeMap;
use std::env;
use std::ops::BitAnd;

#[derive(Clone)]
struct SubtypeContext<'a, S> {
    type_defs: &'a TypeDefs<S>,
    visited: IndexSet<(Type<S>, Type<S>)>,
    constrain_holes: bool,
}

impl<'a, S: Clone + Eq + std::hash::Hash> SubtypeContext<'a, S> {
    fn new<'b>(type_defs: &'b TypeDefs<S>, constrain_holes: bool) -> SubtypeContext<'b, S> {
        SubtypeContext {
            type_defs,
            visited: Default::default(),
            constrain_holes,
        }
    }
    fn normalize(&mut self, typ: Type<S>) -> Result<Type<S>, TypeError<S>> {
        Ok(match typ {
            Type::Name(span, name, args) => {
                self.normalize(self.type_defs.get(&span, &name, &args)?)?
            }
            Type::DualName(span, name, args) => {
                self.normalize(self.type_defs.get(&span, &name, &args)?.dual(Span::None))?
            }
            t => t,
        })
    }
}

use std::fmt::Display;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TypeConstructor {
    Primitive(PrimitiveType),
    Var,
    Name,
    Box,
    Pair,
    Function,
    Either,
    Choice,
    Break,
    Continue,
    Recursive,
    Iterative,
    Self_,
    Exists,
    Forall,
    Hole,
}

impl Display for TypeConstructor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeConstructor::Primitive(p) => write!(f, "`{p}`"),
            TypeConstructor::Var => write!(f, "a type variable"),
            TypeConstructor::Name => write!(f, "a named type"),
            TypeConstructor::Box => write!(f, "a `box` type"),
            TypeConstructor::Pair => write!(f, "a tuple/pair type"),
            TypeConstructor::Function => write!(f, "a function type"),
            TypeConstructor::Either => write!(f, "an `either` type"),
            TypeConstructor::Choice => write!(f, "a `choice` type"),
            TypeConstructor::Break => write!(f, "a `break` type"),
            TypeConstructor::Continue => write!(f, "a `continue` type"),
            TypeConstructor::Recursive => write!(f, "a `recursive` type"),
            TypeConstructor::Iterative => write!(f, "an `iterative` type"),
            TypeConstructor::Self_ => write!(f, "a `self` type"),
            TypeConstructor::Exists => write!(f, "an existential type"),
            TypeConstructor::Forall => write!(f, "a universal type"),
            TypeConstructor::Hole => write!(f, "a type hole"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ConstructorDifference {
    Primitive {
        provided: PrimitiveType,
        expected: PrimitiveType,
    },
    TypeConstructor {
        provided: TypeConstructor,
        expected: TypeConstructor,
    },
}

impl Display for ConstructorDifference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConstructorDifference::Primitive { provided, expected } => {
                write!(f, "Expected `{expected}`, got `{provided}`.")
            }
            ConstructorDifference::TypeConstructor { provided, expected } => {
                if provided == expected {
                    write!(f, "Expected a different {expected}.")
                } else {
                    write!(f, "Expected {expected}, got {provided}.")
                }
            }
        }
    }
}

impl<S> Type<S> {
    pub(crate) fn constructor(&self) -> TypeConstructor {
        match self {
            Type::Primitive(_, p) | Type::DualPrimitive(_, p) => TypeConstructor::Primitive(*p),
            Type::Var(_, _) | Type::DualVar(_, _) => TypeConstructor::Var,
            Type::Name(_, _, _) | Type::DualName(_, _, _) => TypeConstructor::Name,
            Type::Box(_, _) | Type::DualBox(_, _) => TypeConstructor::Box,
            Type::Pair(_, _, _, _) => TypeConstructor::Pair,
            Type::Function(_, _, _, _) => TypeConstructor::Function,
            Type::Either(_, _) => TypeConstructor::Either,
            Type::Choice(_, _) => TypeConstructor::Choice,
            Type::Break(_) => TypeConstructor::Break,
            Type::Continue(_) => TypeConstructor::Continue,
            Type::Recursive { .. } => TypeConstructor::Recursive,
            Type::Iterative { .. } => TypeConstructor::Iterative,
            Type::Self_(_, _) | Type::DualSelf(_, _) => TypeConstructor::Self_,
            Type::Exists(_, _, _) => TypeConstructor::Exists,
            Type::Forall(_, _, _) => TypeConstructor::Forall,
            Type::Hole(_, _, _) | Type::DualHole(_, _, _) | Type::Fail(_) => TypeConstructor::Hole,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SubtypeMismatchKind {
    MissingEitherBranch {
        branch: LocalName,
    },
    MissingChoiceBranch {
        branch: LocalName,
    },
    CleanupBranchMismatch {
        branch: LocalName,
        provided: bool,
        expected: bool,
    },
    ConstructorMismatch(ConstructorDifference),
    ImplicitGenericCountMismatch {
        from_count: usize,
        to_count: usize,
    },
    TypeParameterConstraintMismatch {
        param_name: LocalName,
        provided: TypeConstraint,
        expected: TypeConstraint,
    },
    TypeVariableMismatch,
    HoleConstrainingIsDisabled,
    InvalidCycle,
    CannotCastDownIterative,
    CannotCastUpRecursive,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SubtypeMismatchCause {
    pub(crate) from_path: TypePath,
    pub(crate) to_path: TypePath,
    pub(crate) kind: SubtypeMismatchKind,
}

fn incompatible<S>(
    path1: &TypePath,
    path2: &TypePath,
    kind: SubtypeMismatchKind,
) -> SubtypeResult<S> {
    Incompatible(SubtypeMismatchCause {
        from_path: path1.clone(),
        to_path: path2.clone(),
        kind,
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Assignability {
    Assignable,
    Incompatible(SubtypeMismatchCause),
}

impl Assignability {
    pub fn is_assignable(&self) -> bool {
        matches!(self, Self::Assignable)
    }
}

enum SubtypeResult<S> {
    Compatible,
    Incompatible(SubtypeMismatchCause),
    Cycle {
        from_path: TypePath,
        to_path: TypePath,
        min_left: Type<S>,
        size_left: u32,
        min_right: Type<S>,
        size_right: u32,
        /**
        Time To Live. To avoid merging cycles that don't intersect, as we bubble up the recursive call stack,
        we want to keep the cycle only until its starting point, then simplify it to Compatible.
        Any cycles encountered before that do not intersect it.

        In order to do that, we set ttl to the length of the cycle at creation, and decrease it at any return.
        Once it reaches 0, we simplify it to Compatible.
        */
        ttl: usize,
    },
}

impl<S: Clone> BitAnd for SubtypeResult<S> {
    type Output = SubtypeResult<S>;

    fn bitand(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Compatible, Compatible) => Compatible,
            (c @ Cycle { .. }, Compatible) | (Compatible, c @ Cycle { .. }) => c,
            (
                Cycle {
                    from_path: from_path1,
                    to_path: to_path1,
                    min_left: min_left1,
                    size_left: size_left1,
                    min_right: min_right1,
                    size_right: size_right1,
                    ttl: ttl1,
                },
                Cycle {
                    from_path: from_path2,
                    to_path: to_path2,
                    min_left: min_left2,
                    size_left: size_left2,
                    min_right: min_right2,
                    size_right: size_right2,
                    ttl: ttl2,
                },
            ) => {
                let (from_path, min_left, size_left) = if size_left1 <= size_left2 {
                    (from_path1, min_left1, size_left1)
                } else {
                    (from_path2, min_left2, size_left2)
                };
                let (to_path, min_right, size_right) = if size_right1 <= size_right2 {
                    (to_path1, min_right1, size_right1)
                } else {
                    (to_path2, min_right2, size_right2)
                };
                let ttl = max(ttl1, ttl2);
                if !matches!(min_left, Type::Recursive { .. })
                    && !matches!(min_right, Type::Iterative { .. })
                {
                    Incompatible(SubtypeMismatchCause {
                        from_path,
                        to_path,
                        kind: SubtypeMismatchKind::InvalidCycle,
                    })
                } else {
                    Cycle {
                        from_path,
                        to_path,
                        min_left,
                        size_left,
                        min_right,
                        size_right,
                        ttl,
                    }
                }
            }
            (Incompatible(c), _) | (_, Incompatible(c)) => Incompatible(c),
        }
    }
}

impl<S: Clone> SubtypeResult<S> {
    fn ttl_dec(mut self) -> Self {
        match &mut self {
            Cycle { ttl, .. } => {
                if *ttl == 0 {
                    Compatible
                } else {
                    *ttl -= 1;
                    self
                }
            }
            _ => self,
        }
    }
}

impl<S: Clone + Eq + std::hash::Hash> Type<S> {
    pub fn check_assignable(
        &self,
        span: &Span,
        u: &Type<S>,
        type_defs: &TypeDefs<S>,
    ) -> Result<(), TypeError<S>> {
        if let Assignability::Incompatible(cause) = self.require_assignable_to(u, type_defs)? {
            let from_type = Self::unroll_type_along_path(self.clone(), &cause.from_path, type_defs);
            let to_type = Self::unroll_type_along_path(u.clone(), &cause.to_path, type_defs);

            return Err(TypeError::CannotAssignFromTo(
                span.clone(),
                from_type,
                to_type,
                cause,
            ));
        }
        Ok(())
    }

    pub(crate) fn unroll_type_along_path(
        typ: Self,
        path: &TypePath,
        type_defs: &TypeDefs<S>,
    ) -> Self {
        Self::unroll_along_path_segments(typ, path.as_slice(), type_defs)
    }

    fn unroll_along_path_segments(
        mut typ: Self,
        segments: &[TypePathSegment],
        type_defs: &TypeDefs<S>,
    ) -> Self {
        while matches!(typ, Type::Name(..) | Type::DualName(..)) || typ.display_hint().is_some() {
            if let Ok(expanded) = typ.expand_definition(type_defs) {
                if expanded == typ {
                    break;
                }
                typ = expanded;
            } else {
                break;
            }
        }

        let Some((first, rest)) = segments.split_first() else {
            return typ;
        };

        match (typ, first) {
            (Type::Pair(span, left, right, vars), TypePathSegment::PairLeft) => {
                let unrolled_left = Self::unroll_along_path_segments(*left, rest, type_defs);
                Type::Pair(span, Box::new(unrolled_left), right, vars)
            }
            (Type::Pair(span, left, right, vars), TypePathSegment::PairRight) => {
                let unrolled_right = Self::unroll_along_path_segments(*right, rest, type_defs);
                Type::Pair(span, left, Box::new(unrolled_right), vars)
            }
            (Type::Function(span, arg, ret, vars), TypePathSegment::FunctionParam) => {
                let unrolled_arg = Self::unroll_along_path_segments(*arg, rest, type_defs);
                Type::Function(span, Box::new(unrolled_arg), ret, vars)
            }
            (Type::Function(span, arg, ret, vars), TypePathSegment::FunctionReturn) => {
                let unrolled_ret = Self::unroll_along_path_segments(*ret, rest, type_defs);
                Type::Function(span, arg, Box::new(unrolled_ret), vars)
            }
            (Type::Either(span, mut branches), TypePathSegment::EitherBranch(label))
            | (Type::Either(span, mut branches), TypePathSegment::EitherBranchLabel(label)) => {
                if let Some(mut branch) = branches.remove(label) {
                    branch.typ = Self::unroll_along_path_segments(branch.typ, rest, type_defs);
                    branches.insert(label.clone(), branch);
                }
                Type::Either(span, branches)
            }
            (Type::Choice(span, mut branches), TypePathSegment::ChoiceBranch(label))
            | (Type::Choice(span, mut branches), TypePathSegment::ChoiceBranchLabel(label)) => {
                if let Some(mut branch) = branches.remove(label) {
                    branch.typ = Self::unroll_along_path_segments(branch.typ, rest, type_defs);
                    branches.insert(label.clone(), branch);
                }
                Type::Choice(span, branches)
            }
            (other, _) => other,
        }
    }

    pub fn require_assignable_to(
        &self,
        other: &Self,
        type_defs: &TypeDefs<S>,
    ) -> Result<Assignability, TypeError<S>> {
        self.is_assignable_to(other, type_defs, true)
    }

    pub fn is_definitely_assignable_to(
        &self,
        other: &Self,
        type_defs: &TypeDefs<S>,
    ) -> Result<Assignability, TypeError<S>> {
        self.is_assignable_to(other, type_defs, false)
    }

    fn is_assignable_to(
        &self,
        other: &Self,
        type_defs: &TypeDefs<S>,
        constrain_holes: bool,
    ) -> Result<Assignability, TypeError<S>> {
        let mut path1 = TypePath::new();
        let mut path2 = TypePath::new();
        match Type::is_subtype_helper(
            self.clone(),
            other.clone(),
            &mut path1,
            &mut path2,
            SubtypeContext::new(type_defs, constrain_holes),
        )? {
            Compatible => Ok(Assignability::Assignable),
            Incompatible(cause) => Ok(Assignability::Incompatible(cause)),
            Cycle {
                from_path,
                to_path,
                min_left,
                min_right,
                ..
            } => {
                if matches!(min_left, Type::Recursive { .. }) {
                    Ok(Assignability::Assignable)
                } else if matches!(min_right, Type::Iterative { .. }) {
                    Ok(Assignability::Assignable)
                } else {
                    Ok(Assignability::Incompatible(SubtypeMismatchCause {
                        from_path,
                        to_path,
                        kind: SubtypeMismatchKind::InvalidCycle,
                    }))
                }
            }
        }
    }

    /**
    This function checks if `self` <: `other`.

    The algorithm is based on the subtyping relation in `A Logical Account of Subtyping for Session Types (2023)`.

    The implementation takes inspiration from `Subtyping recursive types (1993)`.
    */
    pub(crate) fn is_primitive_subtype(p1: &PrimitiveType, p2: &PrimitiveType) -> bool {
        match (p1, p2) {
            (PrimitiveType::Nat, PrimitiveType::Int) => true,
            (PrimitiveType::Char, PrimitiveType::String) => true,
            (PrimitiveType::Byte, PrimitiveType::Bytes) => true,
            (PrimitiveType::String, PrimitiveType::Bytes) => true,
            (PrimitiveType::Char, PrimitiveType::Bytes) => true,
            _ => p1 == p2,
        }
    }

    fn is_subtype_helper(
        mut type1: Self,
        mut type2: Self,
        path1: &mut TypePath,
        path2: &mut TypePath,
        mut ctx: SubtypeContext<S>,
    ) -> Result<SubtypeResult<S>, TypeError<S>> {
        // Debug trace helper
        if debug_enabled() {
            debug_log_entry(&type1, &type2, &ctx);
        }

        // Fail is compatible with everything — prevents cascading errors.
        if matches!(type1, Type::Fail(_)) || matches!(type2, Type::Fail(_)) {
            return Ok(Compatible);
        }

        if let Some(result) =
            Type::is_subtype_hole(&type1, &type2, path1, path2, ctx.constrain_holes)
        {
            return Ok(result);
        }

        type1 = ctx.normalize(type1)?;
        type2 = ctx.normalize(type2)?;

        if type1 == type2 {
            return Ok(Compatible);
        }

        let pair = (type1, type2);

        if let Some(result) = Type::is_subtype_cycle(&pair, path1, path2, &ctx)? {
            return Ok(result);
        }

        ctx.visited.insert(pair.clone());
        let (type1, type2) = pair;

        if let Some(result) = Type::is_subtype_fixpoint_guard(&type1, &type2, path1, path2) {
            return Ok(result);
        }

        if let Some(result) = Type::is_subtype_expand_fixpoints(&type1, &type2, path1, path2, &ctx)?
        {
            return Ok(result);
        }

        Ok(Type::is_subtype_structural(type1, type2, path1, path2, ctx)?.ttl_dec())
    }

    fn is_subtype_hole(
        type1: &Type<S>,
        type2: &Type<S>,
        path1: &TypePath,
        path2: &TypePath,
        constrain_holes: bool,
    ) -> Option<SubtypeResult<S>> {
        match (type1, type2) {
            (Self::Hole(_, name1, _), Self::Hole(_, name2, _)) if name1 == name2 => {
                Some(Compatible)
            }
            (Self::DualHole(_, name1, _), Self::DualHole(_, name2, _)) if name1 == name2 => {
                Some(Compatible)
            }
            (Self::Hole(_, _, hole), t2) if constrain_holes => {
                hole.add_upper_bound(t2.clone());
                Some(Compatible)
            }
            (t1, Self::Hole(_, _, hole)) if constrain_holes => {
                hole.add_lower_bound(t1.clone());
                Some(Compatible)
            }
            (Self::DualHole(_, _, hole), t2) if constrain_holes => {
                hole.add_lower_bound(t2.clone().dual(Span::None));
                Some(Compatible)
            }
            (t1, Self::DualHole(_, _, hole)) if constrain_holes => {
                hole.add_upper_bound(t1.clone().dual(Span::None));
                Some(Compatible)
            }
            (Self::Hole(..), _)
            | (_, Self::Hole(..))
            | (Self::DualHole(..), _)
            | (_, Self::DualHole(..)) => Some(incompatible(
                path1,
                path2,
                SubtypeMismatchKind::HoleConstrainingIsDisabled,
            )),
            _ => None,
        }
    }

    fn is_subtype_cycle(
        pair: &(Type<S>, Type<S>),
        path1: &TypePath,
        path2: &TypePath,
        ctx: &SubtypeContext<S>,
    ) -> Result<Option<SubtypeResult<S>>, TypeError<S>> {
        let Some(ind) = ctx.visited.get_index_of(pair) else {
            return Ok(None);
        };
        if debug_enabled() {
            debug_log_stack(ctx);
        }

        let min_left = ctx
            .visited
            .iter()
            .skip(ind)
            .map(|(t1, _t2)| t1)
            .filter(|t1| t1.is_fixpoint())
            .map(|t1| t1.size(ctx.type_defs).map(|size| (size, t1)))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .min_by_key(|(size, _)| *size)
            .map(|(_, typ)| typ)
            .expect("minimum should exist");
        let min_right = ctx
            .visited
            .iter()
            .skip(ind)
            .map(|(_t1, t2)| t2)
            .filter(|t2| t2.is_fixpoint())
            .map(|t2| t2.size(ctx.type_defs).map(|size| (size, t2)))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .min_by_key(|(size, _)| *size)
            .map(|(_, typ)| typ)
            .expect("minimum should exist");
        if !matches!(min_left, Type::Recursive { .. })
            && !matches!(min_right, Type::Iterative { .. })
        {
            let mut from_path = path1.clone();
            let mut to_path = path2.clone();
            if from_path.last() == Some(&TypePathSegment::IterativeBody)
                || from_path.last() == Some(&TypePathSegment::RecursiveBody)
            {
                from_path.pop();
            }
            if to_path.last() == Some(&TypePathSegment::IterativeBody)
                || to_path.last() == Some(&TypePathSegment::RecursiveBody)
            {
                to_path.pop();
            }
            from_path.push(TypePathSegment::Self_);
            to_path.push(TypePathSegment::Self_);
            return Ok(Some(Incompatible(SubtypeMismatchCause {
                from_path,
                to_path,
                kind: SubtypeMismatchKind::InvalidCycle,
            })));
        }
        Ok(Some(Cycle {
            from_path: path1.clone(),
            to_path: path2.clone(),
            min_left: min_left.clone(),
            size_left: min_left.size(ctx.type_defs)?,
            min_right: min_right.clone(),
            size_right: min_right.size(ctx.type_defs)?,
            ttl: ctx.visited.len(),
        }))
    }

    fn is_subtype_fixpoint_guard(
        type1: &Type<S>,
        type2: &Type<S>,
        path1: &TypePath,
        path2: &TypePath,
    ) -> Option<SubtypeResult<S>> {
        if let Type::Iterative { asc: asc1, .. } = type1 {
            if !asc1.is_empty() {
                return Some(if let Self::Iterative { asc: asc2, .. } = type2 {
                    if asc1.is_subset(asc2) {
                        Compatible
                    } else {
                        incompatible(path1, path2, SubtypeMismatchKind::CannotCastDownIterative)
                    }
                } else {
                    incompatible(path1, path2, SubtypeMismatchKind::CannotCastDownIterative)
                });
            }
        }

        if let Type::Recursive { asc: asc2, .. } = type2 {
            if !asc2.is_empty() {
                return Some(if let Self::Recursive { asc: asc1, .. } = type1 {
                    if asc2.is_subset(asc1) {
                        Compatible
                    } else {
                        incompatible(path1, path2, SubtypeMismatchKind::CannotCastUpRecursive)
                    }
                } else {
                    incompatible(path1, path2, SubtypeMismatchKind::CannotCastUpRecursive)
                });
            }
        }

        None
    }

    fn is_subtype_expand_fixpoints(
        type1: &Type<S>,
        type2: &Type<S>,
        path1: &mut TypePath,
        path2: &mut TypePath,
        ctx: &SubtypeContext<S>,
    ) -> Result<Option<SubtypeResult<S>>, TypeError<S>> {
        if let Type::Recursive { .. } | Type::Iterative { .. } = type1 {
            let seg = if matches!(type1, Type::Recursive { .. }) {
                TypePathSegment::RecursiveBody
            } else {
                TypePathSegment::IterativeBody
            };
            let type1 = Type::expand_fixpoint_unfounded(type1)?;
            path1.push(seg);
            let res =
                Type::is_subtype_helper(type1, type2.clone(), path1, path2, ctx.clone())?.ttl_dec();
            path1.pop();
            return Ok(Some(res));
        }

        if let Type::Recursive { .. } | Type::Iterative { .. } = type2 {
            let seg = if matches!(type2, Type::Recursive { .. }) {
                TypePathSegment::RecursiveBody
            } else {
                TypePathSegment::IterativeBody
            };
            let type2 = Type::expand_fixpoint_unfounded(type2)?;
            path2.push(seg);
            let res =
                Type::is_subtype_helper(type1.clone(), type2, path1, path2, ctx.clone())?.ttl_dec();
            path2.pop();
            return Ok(Some(res));
        }

        Ok(None)
    }

    fn is_subtype_structural(
        type1: Self,
        type2: Self,
        path1: &mut TypePath,
        path2: &mut TypePath,
        ctx: SubtypeContext<S>,
    ) -> Result<SubtypeResult<S>, TypeError<S>> {
        match (type1, type2) {
            (Self::Primitive(_, p1), Self::Primitive(_, p2)) => {
                Ok(if Self::is_primitive_subtype(&p1, &p2) {
                    Compatible
                } else {
                    incompatible(
                        path1,
                        path2,
                        SubtypeMismatchKind::ConstructorMismatch(
                            ConstructorDifference::Primitive {
                                provided: p1,
                                expected: p2,
                            },
                        ),
                    )
                })
            }
            (Self::DualPrimitive(_, p1), Self::DualPrimitive(_, p2)) => {
                Ok(if Self::is_primitive_subtype(&p2, &p1) {
                    Compatible
                } else {
                    incompatible(
                        path1,
                        path2,
                        SubtypeMismatchKind::ConstructorMismatch(
                            ConstructorDifference::Primitive {
                                provided: p1,
                                expected: p2,
                            },
                        ),
                    )
                })
            }

            (Self::Var(_, name1), Self::Var(_, name2)) => Ok(if name1 == name2 {
                Compatible
            } else {
                incompatible(path1, path2, SubtypeMismatchKind::TypeVariableMismatch)
            }),
            (Self::DualVar(_, name1), Self::DualVar(_, name2)) => Ok(if name1 == name2 {
                Compatible
            } else {
                incompatible(path1, path2, SubtypeMismatchKind::TypeVariableMismatch)
            }),

            (t1, t2) => Type::is_subtype_box_structural(t1, t2, path1, path2, ctx),
        }
    }

    fn is_subtype_box_structural(
        type1: Self,
        type2: Self,
        path1: &mut TypePath,
        path2: &mut TypePath,
        ctx: SubtypeContext<S>,
    ) -> Result<SubtypeResult<S>, TypeError<S>> {
        match (type1, type2) {
            (Self::Box(_, t1), Self::Box(_, t2)) => {
                path1.push(TypePathSegment::BoxBody);
                path2.push(TypePathSegment::BoxBody);
                let res = Type::is_subtype_helper(
                    t1.as_ref().clone(),
                    t2.as_ref().clone(),
                    path1,
                    path2,
                    ctx,
                );
                path1.pop();
                path2.pop();
                res
            }
            (Self::DualBox(_, t1), Self::DualBox(_, t2)) => {
                let t1 = t1.as_ref().clone().dual(Span::None);
                let t2 = t2.as_ref().clone().dual(Span::None);
                path1.push(TypePathSegment::BoxBody);
                path2.push(TypePathSegment::BoxBody);
                let res = Type::is_subtype_helper(t1, t2, path1, path2, ctx);
                path1.pop();
                path2.pop();
                res
            }
            (t1, t2) => Type::is_subtype_pair_like(t1, t2, path1, path2, ctx),
        }
    }

    fn is_subtype_pair_like(
        type1: Self,
        type2: Self,
        path1: &mut TypePath,
        path2: &mut TypePath,
        ctx: SubtypeContext<S>,
    ) -> Result<SubtypeResult<S>, TypeError<S>> {
        match (type1, type2) {
            (Self::Pair(_, t1, u1, vars1), Self::Pair(_, t2, u2, vars2)) => {
                if vars1.len() != vars2.len() {
                    path1.push(TypePathSegment::ImplicitGenerics);
                    path2.push(TypePathSegment::ImplicitGenerics);
                    let res = incompatible(
                        path1,
                        path2,
                        SubtypeMismatchKind::ImplicitGenericCountMismatch {
                            from_count: vars1.len(),
                            to_count: vars2.len(),
                        },
                    );
                    path1.pop();
                    path2.pop();
                    return Ok(res);
                }
                let mut t2: Type<S> = *t2.clone();
                let mut u2: Type<S> = *u2.clone();
                for (var1, var2) in vars1.iter().zip(vars2.iter()) {
                    // Covariant, like `Exists`: pair vars are existential binders.
                    if !var2.constraint.is_broader_or_equal_than(var1.constraint) {
                        path1.push(TypePathSegment::ImplicitGenerics);
                        path1.push(TypePathSegment::TypeParameter(var1.name.clone()));
                        path2.push(TypePathSegment::ImplicitGenerics);
                        path2.push(TypePathSegment::TypeParameter(var2.name.clone()));
                        let res = incompatible(
                            path1,
                            path2,
                            SubtypeMismatchKind::TypeParameterConstraintMismatch {
                                param_name: var1.name.clone(),
                                provided: var1.constraint,
                                expected: var2.constraint,
                            },
                        );
                        path1.pop();
                        path1.pop();
                        path2.pop();
                        path2.pop();
                        return Ok(res);
                    }
                    t2 = t2.substitute(BTreeMap::from([(
                        &var2.name,
                        &Type::Var(Span::None, var1.name.clone()),
                    )]))?;
                    u2 = u2.substitute(BTreeMap::from([(
                        &var2.name,
                        &Type::Var(Span::None, var1.name.clone()),
                    )]))?;
                }
                path1.push(TypePathSegment::PairLeft);
                path2.push(TypePathSegment::PairLeft);
                let res1 = Type::is_subtype_helper(*t1, t2, path1, path2, ctx.clone())?;
                path1.pop();
                path2.pop();

                path1.push(TypePathSegment::PairRight);
                path2.push(TypePathSegment::PairRight);
                let res2 = Type::is_subtype_helper(*u1, u2, path1, path2, ctx)?;
                path1.pop();
                path2.pop();

                Ok(res1 & res2)
            }
            (Self::Function(_, t1, u1, vars1), Self::Function(_, t2, u2, vars2)) => {
                let t1 = t1.clone().dual(Span::None);
                let t2 = t2.clone().dual(Span::None);
                if vars1.len() != vars2.len() {
                    path1.push(TypePathSegment::ImplicitGenerics);
                    path2.push(TypePathSegment::ImplicitGenerics);
                    let res = incompatible(
                        path1,
                        path2,
                        SubtypeMismatchKind::ImplicitGenericCountMismatch {
                            from_count: vars1.len(),
                            to_count: vars2.len(),
                        },
                    );
                    path1.pop();
                    path2.pop();
                    return Ok(res);
                }
                let mut t2: Type<S> = t2;
                let mut u2: Type<S> = *u2.clone();
                for (var1, var2) in vars1.iter().zip(vars2.iter()) {
                    if !var1.constraint.is_broader_or_equal_than(var2.constraint) {
                        path1.push(TypePathSegment::ImplicitGenerics);
                        path1.push(TypePathSegment::TypeParameter(var1.name.clone()));
                        path2.push(TypePathSegment::ImplicitGenerics);
                        path2.push(TypePathSegment::TypeParameter(var2.name.clone()));
                        let res = incompatible(
                            path1,
                            path2,
                            SubtypeMismatchKind::TypeParameterConstraintMismatch {
                                param_name: var1.name.clone(),
                                provided: var1.constraint,
                                expected: var2.constraint,
                            },
                        );
                        path1.pop();
                        path1.pop();
                        path2.pop();
                        path2.pop();
                        return Ok(res);
                    }
                    t2 = t2.substitute(BTreeMap::from([(
                        &var2.name,
                        &Type::Var(Span::None, var1.name.clone()),
                    )]))?;
                    u2 = u2.substitute(BTreeMap::from([(
                        &var2.name,
                        &Type::Var(Span::None, var1.name.clone()),
                    )]))?;
                }
                path1.push(TypePathSegment::FunctionParam);
                path2.push(TypePathSegment::FunctionParam);
                let res1 = Type::is_subtype_helper(t1, t2, path1, path2, ctx.clone())?;
                path1.pop();
                path2.pop();

                path1.push(TypePathSegment::FunctionReturn);
                path2.push(TypePathSegment::FunctionReturn);
                let res2 = Type::is_subtype_helper(*u1, u2, path1, path2, ctx)?;
                path1.pop();
                path2.pop();

                Ok(res1 & res2)
            }
            (t1, t2) => Type::is_subtype_branching(t1, t2, path1, path2, ctx),
        }
    }

    fn is_subtype_branching(
        type1: Self,
        type2: Self,
        path1: &mut TypePath,
        path2: &mut TypePath,
        ctx: SubtypeContext<S>,
    ) -> Result<SubtypeResult<S>, TypeError<S>> {
        match (type1, type2) {
            (Self::Either(_, branches1), _) if branches1.is_empty() => Ok(Compatible),
            (Self::Either(_, branches1), Self::Either(_, branches2)) => {
                let mut res = Compatible;
                for (branch, t1) in branches1 {
                    let Some(t2) = branches2.get(&branch) else {
                        path1.push(TypePathSegment::EitherBranchLabel(branch.clone()));
                        let res = incompatible(
                            path1,
                            path2,
                            SubtypeMismatchKind::MissingEitherBranch {
                                branch: branch.clone(),
                            },
                        );
                        path1.pop();
                        return Ok(res);
                    };
                    if t1.cleanup && !t2.cleanup {
                        path1.push(TypePathSegment::EitherBranchLabel(branch.clone()));
                        path2.push(TypePathSegment::EitherBranchLabel(branch.clone()));
                        let res = incompatible(
                            path1,
                            path2,
                            SubtypeMismatchKind::CleanupBranchMismatch {
                                branch: branch.clone(),
                                provided: t1.cleanup,
                                expected: t2.cleanup,
                            },
                        );
                        path1.pop();
                        path2.pop();
                        return Ok(res);
                    }
                    path1.push(TypePathSegment::EitherBranch(branch.clone()));
                    path2.push(TypePathSegment::EitherBranch(branch.clone()));
                    let branch_res = Type::is_subtype_helper(
                        t1.typ.clone(),
                        t2.typ.clone(),
                        path1,
                        path2,
                        ctx.clone(),
                    )?;
                    path1.pop();
                    path2.pop();
                    res = res & branch_res;
                }
                Ok(res)
            }
            (_, Self::Choice(_, branches2)) if branches2.is_empty() => Ok(Compatible),
            (Self::Choice(_, branches1), Self::Choice(_, branches2)) => {
                let mut res = Compatible;
                for (branch, t2) in branches2 {
                    let Some(t1) = branches1.get(&branch) else {
                        path2.push(TypePathSegment::ChoiceBranchLabel(branch.clone()));
                        let res = incompatible(
                            path1,
                            path2,
                            SubtypeMismatchKind::MissingChoiceBranch {
                                branch: branch.clone(),
                            },
                        );
                        path2.pop();
                        return Ok(res);
                    };
                    if t2.cleanup && !t1.cleanup {
                        path1.push(TypePathSegment::ChoiceBranchLabel(branch.clone()));
                        path2.push(TypePathSegment::ChoiceBranchLabel(branch.clone()));
                        let res = incompatible(
                            path1,
                            path2,
                            SubtypeMismatchKind::CleanupBranchMismatch {
                                branch: branch.clone(),
                                provided: t1.cleanup,
                                expected: t2.cleanup,
                            },
                        );
                        path1.pop();
                        path2.pop();
                        return Ok(res);
                    }
                    path1.push(TypePathSegment::ChoiceBranch(branch.clone()));
                    path2.push(TypePathSegment::ChoiceBranch(branch.clone()));
                    let branch_res = Type::is_subtype_helper(
                        t1.typ.clone(),
                        t2.typ.clone(),
                        path1,
                        path2,
                        ctx.clone(),
                    )?;
                    path1.pop();
                    path2.pop();
                    res = res & branch_res;
                }
                Ok(res)
            }
            (Self::Break(_), Self::Break(_)) => Ok(Compatible),
            (Self::Continue(_), Self::Continue(_)) => Ok(Compatible),

            (Self::Exists(loc, name1, body1), Self::Exists(_, name2, body2)) => {
                if !name2.constraint.is_broader_or_equal_than(name1.constraint) {
                    path1.push(TypePathSegment::TypeParameter(name1.name.clone()));
                    path2.push(TypePathSegment::TypeParameter(name2.name.clone()));
                    let res = incompatible(
                        path1,
                        path2,
                        SubtypeMismatchKind::TypeParameterConstraintMismatch {
                            param_name: name1.name.clone(),
                            provided: name1.constraint,
                            expected: name2.constraint,
                        },
                    );
                    path1.pop();
                    path2.pop();
                    return Ok(res);
                }
                Type::is_subtype_quantified(loc, name1, body1, name2, body2, path1, path2, ctx)
            }
            (Self::Forall(loc, name1, body1), Self::Forall(_, name2, body2)) => {
                if !name1.constraint.is_broader_or_equal_than(name2.constraint) {
                    path1.push(TypePathSegment::TypeParameter(name1.name.clone()));
                    path2.push(TypePathSegment::TypeParameter(name2.name.clone()));
                    let res = incompatible(
                        path1,
                        path2,
                        SubtypeMismatchKind::TypeParameterConstraintMismatch {
                            param_name: name1.name.clone(),
                            provided: name1.constraint,
                            expected: name2.constraint,
                        },
                    );
                    path1.pop();
                    path2.pop();
                    return Ok(res);
                }
                Type::is_subtype_quantified(loc, name1, body1, name2, body2, path1, path2, ctx)
            }

            (_t1, _t2) => {
                if debug_enabled() {
                    debug_log("fallback => false");
                    debug_log_stack(&ctx);
                }
                Ok(incompatible(
                    path1,
                    path2,
                    SubtypeMismatchKind::ConstructorMismatch(
                        ConstructorDifference::TypeConstructor {
                            provided: _t1.constructor(),
                            expected: _t2.constructor(),
                        },
                    ),
                ))
            }
        }
    }

    fn is_subtype_quantified(
        loc: Span,
        param1: TypeParameter,
        body1: Box<Self>,
        param2: TypeParameter,
        body2: Box<Self>,
        path1: &mut TypePath,
        path2: &mut TypePath,
        ctx: SubtypeContext<S>,
    ) -> Result<SubtypeResult<S>, TypeError<S>> {
        let body2 = body2.substitute(BTreeMap::from([(
            &param2.name,
            &Type::Var(loc.clone(), param1.name.clone()),
        )]))?;
        path1.push(TypePathSegment::ExistsBody);
        path2.push(TypePathSegment::ExistsBody);
        let res = Type::is_subtype_helper(*body1, body2, path1, path2, ctx);
        path1.pop();
        path2.pop();
        res
    }
}

fn debug_enabled() -> bool {
    env::var("PAR_SUBTYPE_DEBUG").is_ok()
}

fn debug_log(msg: &str) {
    eprintln!("[subtype] {}", msg);
}

fn debug_log_entry<S>(_left: &Type<S>, _right: &Type<S>, ctx: &SubtypeContext<S>) {
    eprintln!("-----------------------");
    eprintln!("[subtype]   visited={}", ctx.visited.len());
}

fn debug_log_stack<S>(ctx: &SubtypeContext<S>) {
    eprintln!("[subtype] -------Stack-------");
    for (i, _) in ctx.visited.iter().rev().enumerate() {
        eprintln!("[subtype] #{i}: <pair>");
    }
    eprintln!("[subtype] -------Stack-End-------");
}
