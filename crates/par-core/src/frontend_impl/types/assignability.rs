use crate::frontend_impl::language::TypeConstraint;
use crate::frontend_impl::language::TypeParameter;
use crate::frontend_impl::types::assignability::SubtypeResult::{Compatible, Cycle, Incompatible};
use crate::frontend_impl::types::{PrimitiveType, Type, TypeDefs, TypeError};
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

enum SubtypeResult<S> {
    Compatible,
    Incompatible,
    Cycle {
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
                    min_left: min_left1,
                    size_left: size_left1,
                    min_right: min_right1,
                    size_right: size_right1,
                    ttl: ttl1,
                },
                Cycle {
                    min_left: min_left2,
                    size_left: size_left2,
                    min_right: min_right2,
                    size_right: size_right2,
                    ttl: ttl2,
                },
            ) => {
                let (min_left, size_left) = if size_left1 <= size_left2 {
                    (min_left1, size_left1)
                } else {
                    (min_left2, size_left2)
                };
                let (min_right, size_right) = if size_right1 <= size_right2 {
                    (min_right1, size_right1)
                } else {
                    (min_right2, size_right2)
                };
                let ttl = max(ttl1, ttl2);
                if !matches!(min_left, Type::Recursive { .. })
                    && !matches!(min_right, Type::Iterative { .. })
                {
                    Incompatible
                } else {
                    Cycle {
                        min_left,
                        size_left,
                        min_right,
                        size_right,
                        ttl,
                    }
                }
            }
            (_, Incompatible) | (Incompatible, _) => Incompatible,
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
        if !self.require_assignable_to(u, type_defs)? {
            return Err(TypeError::CannotAssignFromTo(
                span.clone(),
                self.clone(),
                u.clone(),
            ));
        }
        Ok(())
    }

    pub fn require_assignable_to(
        &self,
        other: &Self,
        type_defs: &TypeDefs<S>,
    ) -> Result<bool, TypeError<S>> {
        self.is_assignable_to(other, type_defs, true)
    }

    pub fn is_definitely_assignable_to(
        &self,
        other: &Self,
        type_defs: &TypeDefs<S>,
    ) -> Result<bool, TypeError<S>> {
        self.is_assignable_to(other, type_defs, false)
    }

    fn is_assignable_to(
        &self,
        other: &Self,
        type_defs: &TypeDefs<S>,
        constrain_holes: bool,
    ) -> Result<bool, TypeError<S>> {
        match Type::is_subtype_helper(
            self.clone(),
            other.clone(),
            SubtypeContext::new(type_defs, constrain_holes),
        )? {
            Compatible => Ok(true),
            Incompatible => Ok(false),
            Cycle {
                min_left,
                min_right,
                ..
            } => {
                if matches!(min_left, Type::Recursive { .. }) {
                    Ok(true)
                } else if matches!(min_right, Type::Iterative { .. }) {
                    Ok(true)
                } else {
                    Ok(false)
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

        if let Some(result) = Type::is_subtype_hole(&type1, &type2, ctx.constrain_holes) {
            return Ok(result);
        }

        type1 = ctx.normalize(type1)?;
        type2 = ctx.normalize(type2)?;

        if type1 == type2 {
            return Ok(Compatible);
        }

        let pair = (type1, type2);

        if let Some(result) = Type::is_subtype_cycle(&pair, &ctx)? {
            return Ok(result);
        }

        ctx.visited.insert(pair.clone());
        let (type1, type2) = pair;

        if let Some(result) = Type::is_subtype_fixpoint_guard(&type1, &type2) {
            return Ok(result);
        }

        if let Some(result) = Type::is_subtype_box_positive(&type1, &type2, &ctx)? {
            return Ok(result.ttl_dec());
        }

        if let Some(result) = Type::is_subtype_expand_fixpoints(&type1, &type2, &ctx)? {
            return Ok(result);
        }

        Ok(Type::is_subtype_structural(type1, type2, ctx)?.ttl_dec())
    }

    fn is_subtype_hole(
        type1: &Type<S>,
        type2: &Type<S>,
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
            | (_, Self::DualHole(..)) => Some(Incompatible),
            _ => None,
        }
    }

    fn is_subtype_box_positive(
        type1: &Type<S>,
        type2: &Type<S>,
        ctx: &SubtypeContext<S>,
    ) -> Result<Option<SubtypeResult<S>>, TypeError<S>> {
        match (type1, type2) {
            (t1, Self::Box(_, t2))
                if t1.satisfies_constraint(TypeConstraint::Box, ctx.type_defs)? =>
            {
                Ok(Some(Type::is_subtype_helper(
                    t1.clone(),
                    t2.as_ref().clone(),
                    ctx.clone(),
                )?))
            }
            (Self::DualBox(_, t1), t2)
                if t1.satisfies_constraint(TypeConstraint::Box, ctx.type_defs)? =>
            {
                Ok(Some(Type::is_subtype_helper(
                    t1.as_ref().clone().dual(Span::None),
                    t2.clone(),
                    ctx.clone(),
                )?))
            }
            _ => Ok(None),
        }
    }

    fn is_subtype_cycle(
        pair: &(Type<S>, Type<S>),
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
            .filter_map(|t1| t1.size(ctx.type_defs).ok().map(|size| (size, t1)))
            .min_by_key(|(size, _)| *size)
            .map(|(_, typ)| typ)
            .expect("minimum should exist");
        let min_right = ctx
            .visited
            .iter()
            .skip(ind)
            .map(|(_t1, t2)| t2)
            .filter(|t2| t2.is_fixpoint())
            .filter_map(|t2| t2.size(ctx.type_defs).ok().map(|size| (size, t2)))
            .min_by_key(|(size, _)| *size)
            .map(|(_, typ)| typ)
            .expect("minimum should exist");
        if !matches!(min_left, Type::Recursive { .. })
            && !matches!(min_right, Type::Iterative { .. })
        {
            return Ok(Some(Incompatible));
        }
        Ok(Some(Cycle {
            min_left: min_left.clone(),
            size_left: min_left.size(ctx.type_defs)?,
            min_right: min_right.clone(),
            size_right: min_right.size(ctx.type_defs)?,
            ttl: ctx.visited.len(),
        }))
    }

    fn is_subtype_fixpoint_guard(type1: &Type<S>, type2: &Type<S>) -> Option<SubtypeResult<S>> {
        if let Type::Iterative { asc: asc1, .. } = type1 {
            if !asc1.is_empty() {
                return Some(if let Self::Recursive { asc: asc2, .. } = type2 {
                    if asc1.is_subset(asc2) {
                        Compatible
                    } else {
                        Incompatible
                    }
                } else {
                    Incompatible
                });
            }
        }

        if let Type::Recursive { asc: asc2, .. } = type2 {
            if !asc2.is_empty() {
                return Some(if let Self::Recursive { asc: asc1, .. } = type1 {
                    if asc2.is_subset(asc1) {
                        Compatible
                    } else {
                        Incompatible
                    }
                } else {
                    Incompatible
                });
            }
        }

        None
    }

    fn is_subtype_expand_fixpoints(
        type1: &Type<S>,
        type2: &Type<S>,
        ctx: &SubtypeContext<S>,
    ) -> Result<Option<SubtypeResult<S>>, TypeError<S>> {
        if let Type::Recursive { .. } | Type::Iterative { .. } = type1 {
            let type1 = Type::expand_fixpoint_unfounded(type1)?;
            return Ok(Some(
                Type::is_subtype_helper(type1, type2.clone(), ctx.clone())?.ttl_dec(),
            ));
        }

        if let Type::Recursive { .. } | Type::Iterative { .. } = type2 {
            let type2 = Type::expand_fixpoint_unfounded(type2)?;
            return Ok(Some(
                Type::is_subtype_helper(type1.clone(), type2, ctx.clone())?.ttl_dec(),
            ));
        }

        Ok(None)
    }

    fn is_subtype_structural(
        type1: Self,
        type2: Self,
        ctx: SubtypeContext<S>,
    ) -> Result<SubtypeResult<S>, TypeError<S>> {
        match (type1, type2) {
            (Self::Primitive(_, p1), Self::Primitive(_, p2)) => {
                Ok(if Self::is_primitive_subtype(&p1, &p2) {
                    Compatible
                } else {
                    Incompatible
                })
            }
            (Self::DualPrimitive(_, p1), Self::DualPrimitive(_, p2)) => {
                Ok(if Self::is_primitive_subtype(&p2, &p1) {
                    Compatible
                } else {
                    Incompatible
                })
            }

            (Self::Var(_, name1), Self::Var(_, name2)) => Ok(if name1 == name2 {
                Compatible
            } else {
                Incompatible
            }),
            (Self::DualVar(_, name1), Self::DualVar(_, name2)) => Ok(if name1 == name2 {
                Compatible
            } else {
                Incompatible
            }),

            (t1, t2) => Type::is_subtype_box_structural(t1, t2, ctx),
        }
    }

    fn is_subtype_box_structural(
        type1: Self,
        type2: Self,
        ctx: SubtypeContext<S>,
    ) -> Result<SubtypeResult<S>, TypeError<S>> {
        match (type1, type2) {
            (Self::Box(_, t1), Self::Box(_, t2)) => {
                Type::is_subtype_helper(t1.as_ref().clone(), t2.as_ref().clone(), ctx)
            }
            (Self::Box(_, t1), t2) => Type::is_subtype_helper(t1.as_ref().clone(), t2, ctx),
            (Self::DualBox(_, t1), Self::DualBox(_, t2)) => {
                let t1 = t1.as_ref().clone().dual(Span::None);
                let t2 = t2.as_ref().clone().dual(Span::None);
                Type::is_subtype_helper(t1, t2, ctx)
            }
            (t1, Self::DualBox(_, t2)) => {
                let t2 = t2.as_ref().clone().dual(Span::None);
                Type::is_subtype_helper(t1, t2, ctx)
            }
            (t1, t2) => Type::is_subtype_pair_like(t1, t2, ctx),
        }
    }

    fn is_subtype_pair_like(
        type1: Self,
        type2: Self,
        ctx: SubtypeContext<S>,
    ) -> Result<SubtypeResult<S>, TypeError<S>> {
        match (type1, type2) {
            (Self::Pair(_, t1, u1, vars1), Self::Pair(_, t2, u2, vars2)) => {
                if vars1.len() != vars2.len() {
                    return Ok(Incompatible);
                }
                let mut t2: Type<S> = *t2.clone();
                let mut u2: Type<S> = *u2.clone();
                for (var1, var2) in vars1.iter().zip(vars2.iter()) {
                    // Covariant, like `Exists`: pair vars are existential binders.
                    if !var2.constraint.is_broader_or_equal_than(var1.constraint) {
                        return Ok(Incompatible);
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
                Ok(Type::is_subtype_helper(*t1, t2, ctx.clone())?
                    & Type::is_subtype_helper(*u1, u2, ctx)?)
            }
            (Self::Function(_, t1, u1, vars1), Self::Function(_, t2, u2, vars2)) => {
                let t1 = t1.clone().dual(Span::None);
                let t2 = t2.clone().dual(Span::None);
                if vars1.len() != vars2.len() {
                    return Ok(Incompatible);
                }
                let mut t2: Type<S> = t2;
                let mut u2: Type<S> = *u2.clone();
                for (var1, var2) in vars1.iter().zip(vars2.iter()) {
                    if !var1.constraint.is_broader_or_equal_than(var2.constraint) {
                        return Ok(Incompatible);
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
                Ok(Type::is_subtype_helper(t1, t2, ctx.clone())?
                    & Type::is_subtype_helper(*u1, u2, ctx)?)
            }
            (t1, t2) => Type::is_subtype_branching(t1, t2, ctx),
        }
    }

    fn is_subtype_branching(
        type1: Self,
        type2: Self,
        ctx: SubtypeContext<S>,
    ) -> Result<SubtypeResult<S>, TypeError<S>> {
        match (type1, type2) {
            (Self::Either(_, branches1), _) if branches1.is_empty() => Ok(Compatible),
            (Self::Either(_, branches1), Self::Either(_, branches2)) => {
                let mut res = Compatible;
                for (branch, t1) in branches1 {
                    let Some(t2) = branches2.get(&branch) else {
                        return Ok(Incompatible);
                    };
                    res = res & Type::is_subtype_helper(t1.clone(), t2.clone(), ctx.clone())?;
                }
                Ok(res)
            }
            (_, Self::Choice(_, branches2)) if branches2.is_empty() => Ok(Compatible),
            (Self::Choice(_, branches1), Self::Choice(_, branches2)) => {
                let mut res = Compatible;
                for (branch, t2) in branches2 {
                    let Some(t1) = branches1.get(&branch) else {
                        return Ok(Incompatible);
                    };
                    res = res & Type::is_subtype_helper(t1.clone(), t2.clone(), ctx.clone())?;
                }
                Ok(res)
            }
            (Self::Break(_), Self::Break(_)) => Ok(Compatible),
            (Self::Continue(_), Self::Continue(_)) => Ok(Compatible),

            (Self::Exists(loc, name1, body1), Self::Exists(_, name2, body2)) => {
                // Covariant: the provider picks the witness, so its constraint must
                // imply the constraint the target type promises to its consumer.
                if !name2.constraint.is_broader_or_equal_than(name1.constraint) {
                    return Ok(Incompatible);
                }
                Type::is_subtype_quantified(loc, name1, body1, name2, body2, ctx)
            }
            (Self::Forall(loc, name1, body1), Self::Forall(_, name2, body2)) => {
                // Contravariant: the consumer picks the type, so the subtype must
                // accept every type the target type promises to accept.
                if !name1.constraint.is_broader_or_equal_than(name2.constraint) {
                    return Ok(Incompatible);
                }
                Type::is_subtype_quantified(loc, name1, body1, name2, body2, ctx)
            }

            (_t1, _t2) => {
                if debug_enabled() {
                    debug_log("fallback => false");
                    debug_log_stack(&ctx);
                }
                Ok(Incompatible)
            }
        }
    }

    fn is_subtype_quantified(
        loc: Span,
        param1: TypeParameter,
        body1: Box<Self>,
        param2: TypeParameter,
        body2: Box<Self>,
        ctx: SubtypeContext<S>,
    ) -> Result<SubtypeResult<S>, TypeError<S>> {
        let body2 = body2.substitute(BTreeMap::from([(
            &param2.name,
            &Type::Var(loc.clone(), param1.name.clone()),
        )]))?;
        Type::is_subtype_helper(*body1, body2, ctx)
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
