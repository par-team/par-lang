use crate::frontend_impl::types::{PrimitiveType, Type, TypeDefs, TypeError};
use crate::location::Span;
use std::collections::BTreeMap;

pub(crate) fn union_primitives<S: Clone + Eq + std::hash::Hash>(
    p1: &PrimitiveType,
    p2: &PrimitiveType,
) -> Option<PrimitiveType> {
    if Type::<S>::is_primitive_subtype(p1, p2) {
        Some(p2.clone())
    } else if Type::<S>::is_primitive_subtype(p2, p1) {
        Some(p1.clone())
    } else {
        None
    }
}

pub(crate) fn intersect_primitives<S: Clone + Eq + std::hash::Hash>(
    p1: &PrimitiveType,
    p2: &PrimitiveType,
) -> Option<PrimitiveType> {
    if Type::<S>::is_primitive_subtype(p1, p2) {
        Some(p1.clone())
    } else if Type::<S>::is_primitive_subtype(p2, p1) {
        Some(p2.clone())
    } else {
        None
    }
}

pub fn union_types<S: Clone + Eq + std::hash::Hash>(
    typedefs: &TypeDefs<S>,
    span: &Span,
    type1: &Type<S>,
    type2: &Type<S>,
) -> Result<Type<S>, TypeError<S>> {
    if type1.is_definitely_assignable_to(type2, typedefs)? {
        return Ok(type2.clone());
    }
    if type2.is_definitely_assignable_to(type1, typedefs)? {
        return Ok(type1.clone());
    }

    union_types_structural(typedefs, span, type1, type2)
}

pub fn intersect_types<S: Clone + Eq + std::hash::Hash>(
    typedefs: &TypeDefs<S>,
    span: &Span,
    type1: &Type<S>,
    type2: &Type<S>,
) -> Result<Type<S>, TypeError<S>> {
    if type1.is_definitely_assignable_to(type2, typedefs)? {
        return Ok(type1.clone());
    }
    if type2.is_definitely_assignable_to(type1, typedefs)? {
        return Ok(type2.clone());
    }

    intersect_types_structural(typedefs, span, type1, type2)
}

fn union_types_structural<S: Clone + Eq + std::hash::Hash>(
    typedefs: &TypeDefs<S>,
    span: &Span,
    type1: &Type<S>,
    type2: &Type<S>,
) -> Result<Type<S>, TypeError<S>> {
    match (type1, type2) {
        (Type::Either(_, branches), t2) if branches.is_empty() => Ok(t2.clone()),
        (t1, Type::Either(_, branches)) if branches.is_empty() => Ok(t1.clone()),
        (t1 @ Type::Choice(_, branches), _t2) if branches.is_empty() => Ok(t1.clone()),
        (_t1, t2 @ Type::Choice(_, branches)) if branches.is_empty() => Ok(t2.clone()),
        (Type::Name(span1, name1, args1), t2) => {
            union_types(typedefs, span, &typedefs.get(span1, name1, args1)?, t2)
        }
        (t1, Type::Name(span2, name2, args2)) => {
            union_types(typedefs, span, t1, &typedefs.get(span2, name2, args2)?)
        }
        (Type::DualName(span1, name1, args1), t2) => {
            union_types(typedefs, span, &typedefs.get_dual(span1, name1, args1)?, t2)
        }
        (t1, Type::DualName(span2, name2, args2)) => {
            union_types(typedefs, span, t1, &typedefs.get_dual(span2, name2, args2)?)
        }
        (t1, t2) => union_types_atoms(typedefs, span, t1, t2),
    }
}

fn union_types_atoms<S: Clone + Eq + std::hash::Hash>(
    typedefs: &TypeDefs<S>,
    span: &Span,
    type1: &Type<S>,
    type2: &Type<S>,
) -> Result<Type<S>, TypeError<S>> {
    match (type1, type2) {
        (Type::Var(_, name1), Type::Var(_, name2)) if name1 == name2 => {
            Ok(Type::Var(span.clone(), name1.clone()))
        }
        (Type::DualVar(_, name1), Type::DualVar(_, name2)) if name1 == name2 => {
            Ok(Type::DualVar(span.clone(), name1.clone()))
        }
        (Type::Box(_, inner1), Type::Box(_, inner2)) => Ok(Type::Box(
            span.clone(),
            Box::new(union_types(typedefs, span, inner1, inner2)?),
        )),
        (Type::DualBox(_, inner1), Type::DualBox(_, inner2)) => Ok(Type::DualBox(
            span.clone(),
            Box::new(intersect_types(typedefs, span, inner1, inner2)?),
        )),
        (Type::Break(_), Type::Break(_)) => Ok(Type::Break(span.clone())),
        (Type::Continue(_), Type::Continue(_)) => Ok(Type::Continue(span.clone())),
        (Type::Primitive(_, p1), Type::Primitive(_, p2)) if p1 == p2 => {
            let Some(p) = union_primitives::<S>(p1, p2) else {
                return Err(TypeError::TypesCannotBeUnified(
                    span.clone(),
                    type1.clone(),
                    type2.clone(),
                ));
            };
            Ok(Type::Primitive(span.clone(), p))
        }
        (Type::DualPrimitive(_, p1), Type::DualPrimitive(_, p2)) if p1 == p2 => {
            let Some(p) = intersect_primitives::<S>(p1, p2) else {
                return Err(TypeError::TypesCannotBeUnified(
                    span.clone(),
                    type1.clone(),
                    type2.clone(),
                ));
            };
            Ok(Type::DualPrimitive(span.clone(), p))
        }
        (t1, t2) => union_types_compound(typedefs, span, t1, t2),
    }
}

fn union_types_compound<S: Clone + Eq + std::hash::Hash>(
    typedefs: &TypeDefs<S>,
    span: &Span,
    type1: &Type<S>,
    type2: &Type<S>,
) -> Result<Type<S>, TypeError<S>> {
    match (type1, type2) {
        (Type::Pair(_, left1, right1, vars1), Type::Pair(_, left2, right2, vars2))
            if vars1.is_empty() && vars2.is_empty() =>
        {
            Ok(Type::Pair(
                span.clone(),
                Box::new(union_types(typedefs, span, left1, left2)?),
                Box::new(union_types(typedefs, span, right1, right2)?),
                vec![],
            ))
        }
        (Type::Function(_, arg1, ret1, vars1), Type::Function(_, arg2, ret2, vars2))
            if vars1.is_empty() && vars2.is_empty() =>
        {
            Ok(Type::Function(
                span.clone(),
                Box::new(intersect_types(typedefs, span, arg1, arg2)?),
                Box::new(union_types(typedefs, span, ret1, ret2)?),
                vec![],
            ))
        }
        (t1, t2) => union_types_branching(typedefs, span, t1, t2),
    }
}

fn union_types_branching<S: Clone + Eq + std::hash::Hash>(
    typedefs: &TypeDefs<S>,
    span: &Span,
    type1: &Type<S>,
    type2: &Type<S>,
) -> Result<Type<S>, TypeError<S>> {
    match (type1, type2) {
        (Type::Either(_, branches1), Type::Either(_, branches2)) => {
            let mut new_branches = branches1.clone();
            for (name, typ2) in branches2 {
                if let Some(typ1) = new_branches.get(name) {
                    new_branches.insert(name.clone(), union_types(typedefs, span, typ1, typ2)?);
                } else {
                    new_branches.insert(name.clone(), typ2.clone());
                }
            }
            Ok(Type::Either(span.clone(), new_branches))
        }
        (Type::Choice(_, branches1), Type::Choice(_, branches2)) => {
            let mut new_branches = BTreeMap::new();
            for (name, typ1) in branches1 {
                if let Some(typ2) = branches2.get(name) {
                    new_branches.insert(name.clone(), union_types(typedefs, span, typ1, typ2)?);
                }
            }
            Ok(Type::Choice(span.clone(), new_branches))
        }
        (Type::Forall(_, name1, body1), Type::Forall(_, name2, body2)) => Ok(Type::Forall(
            span.clone(),
            crate::frontend_impl::language::TypeParameter {
                name: name1.name.clone(),
                constraint: name1.constraint.narrower(name2.constraint),
            },
            Box::new(union_types(
                typedefs,
                span,
                body1,
                &body2.clone().substitute(BTreeMap::from([(
                    &name2.name,
                    &Type::Var(Span::None, name1.name.clone()),
                )]))?,
            )?),
        )),
        (Type::Exists(_, name1, body1), Type::Exists(_, name2, body2)) => Ok(Type::Exists(
            span.clone(),
            crate::frontend_impl::language::TypeParameter {
                name: name1.name.clone(),
                // Covariant in the constraint, unlike `Forall`.
                constraint: name1.constraint.broader(name2.constraint),
            },
            Box::new(union_types(
                typedefs,
                span,
                body1,
                &body2.clone().substitute(BTreeMap::from([(
                    &name2.name,
                    &Type::Var(Span::None, name1.name.clone()),
                )]))?,
            )?),
        )),
        (Type::Box(_, t1), t2) => union_types(typedefs, span, t1, t2),
        (t1, Type::Box(_, t2)) => union_types(typedefs, span, t1, t2),
        (t1, t2) => Err(TypeError::TypesCannotBeUnified(
            span.clone(),
            t1.clone(),
            t2.clone(),
        )),
    }
}

fn intersect_types_structural<S: Clone + Eq + std::hash::Hash>(
    typedefs: &TypeDefs<S>,
    span: &Span,
    type1: &Type<S>,
    type2: &Type<S>,
) -> Result<Type<S>, TypeError<S>> {
    match (type1, type2) {
        (Type::Choice(_, branches), t2) if branches.is_empty() => Ok(t2.clone()),
        (t1, Type::Choice(_, branches)) if branches.is_empty() => Ok(t1.clone()),
        (t1 @ Type::Either(_, branches), _t2) if branches.is_empty() => Ok(t1.clone()),
        (_t1, t2 @ Type::Either(_, branches)) if branches.is_empty() => Ok(t2.clone()),
        (Type::Name(span1, name1, args1), t2) => {
            intersect_types(typedefs, span, &typedefs.get(span1, name1, args1)?, t2)
        }
        (t1, Type::Name(span2, name2, args2)) => {
            intersect_types(typedefs, span, t1, &typedefs.get(span2, name2, args2)?)
        }
        (Type::DualName(span1, name1, args1), t2) => {
            intersect_types(typedefs, span, &typedefs.get_dual(span1, name1, args1)?, t2)
        }
        (t1, Type::DualName(span2, name2, args2)) => {
            intersect_types(typedefs, span, t1, &typedefs.get_dual(span2, name2, args2)?)
        }
        (t1, t2) => intersect_types_atoms(typedefs, span, t1, t2),
    }
}

fn intersect_types_atoms<S: Clone + Eq + std::hash::Hash>(
    typedefs: &TypeDefs<S>,
    span: &Span,
    type1: &Type<S>,
    type2: &Type<S>,
) -> Result<Type<S>, TypeError<S>> {
    match (type1, type2) {
        (Type::Var(_, name1), Type::Var(_, name2)) if name1 == name2 => {
            Ok(Type::Var(span.clone(), name1.clone()))
        }
        (Type::DualVar(_, name1), Type::DualVar(_, name2)) if name1 == name2 => {
            Ok(Type::DualVar(span.clone(), name1.clone()))
        }
        (Type::Box(_, inner1), Type::Box(_, inner2)) => Ok(Type::Box(
            span.clone(),
            Box::new(intersect_types(typedefs, span, inner1, inner2)?),
        )),
        (Type::DualBox(_, inner1), Type::DualBox(_, inner2)) => Ok(Type::DualBox(
            span.clone(),
            Box::new(union_types(typedefs, span, inner1, inner2)?),
        )),
        (Type::Break(_), Type::Break(_)) => Ok(Type::Break(span.clone())),
        (Type::Continue(_), Type::Continue(_)) => Ok(Type::Continue(span.clone())),
        (Type::Primitive(_, p1), Type::Primitive(_, p2)) if p1 == p2 => {
            let Some(p) = intersect_primitives::<S>(p1, p2) else {
                return Err(TypeError::TypesCannotBeUnified(
                    span.clone(),
                    type1.clone(),
                    type2.clone(),
                ));
            };
            Ok(Type::Primitive(span.clone(), p))
        }
        (Type::DualPrimitive(_, p1), Type::DualPrimitive(_, p2)) if p1 == p2 => {
            let Some(p) = union_primitives::<S>(p1, p2) else {
                return Err(TypeError::TypesCannotBeUnified(
                    span.clone(),
                    type1.clone(),
                    type2.clone(),
                ));
            };
            Ok(Type::DualPrimitive(span.clone(), p))
        }
        (t1, t2) => intersect_types_compound(typedefs, span, t1, t2),
    }
}

fn intersect_types_compound<S: Clone + Eq + std::hash::Hash>(
    typedefs: &TypeDefs<S>,
    span: &Span,
    type1: &Type<S>,
    type2: &Type<S>,
) -> Result<Type<S>, TypeError<S>> {
    match (type1, type2) {
        (Type::Pair(_, left1, right1, vars1), Type::Pair(_, left2, right2, vars2))
            if vars1.is_empty() && vars2.is_empty() =>
        {
            Ok(Type::Pair(
                span.clone(),
                Box::new(intersect_types(typedefs, span, left1, left2)?),
                Box::new(intersect_types(typedefs, span, right1, right2)?),
                vec![],
            ))
        }
        (Type::Function(_, arg1, ret1, vars1), Type::Function(_, arg2, ret2, vars2))
            if vars1.is_empty() && vars2.is_empty() =>
        {
            Ok(Type::Function(
                span.clone(),
                Box::new(union_types(typedefs, span, arg1, arg2)?),
                Box::new(intersect_types(typedefs, span, ret1, ret2)?),
                vec![],
            ))
        }
        (t1, t2) => intersect_types_branching(typedefs, span, t1, t2),
    }
}

fn intersect_types_branching<S: Clone + Eq + std::hash::Hash>(
    typedefs: &TypeDefs<S>,
    span: &Span,
    type1: &Type<S>,
    type2: &Type<S>,
) -> Result<Type<S>, TypeError<S>> {
    match (type1, type2) {
        (Type::Either(_, branches1), Type::Either(_, branches2)) => {
            let mut new_branches = BTreeMap::new();
            for (name, typ1) in branches1 {
                if let Some(typ2) = branches2.get(name) {
                    new_branches.insert(name.clone(), intersect_types(typedefs, span, typ1, typ2)?);
                }
            }
            Ok(Type::Either(span.clone(), new_branches))
        }
        (Type::Choice(_, branches1), Type::Choice(_, branches2)) => {
            let mut new_branches = branches1.clone();
            for (name, typ2) in branches2 {
                if let Some(typ1) = new_branches.get(name) {
                    new_branches.insert(name.clone(), intersect_types(typedefs, span, typ1, typ2)?);
                } else {
                    new_branches.insert(name.clone(), typ2.clone());
                }
            }
            Ok(Type::Choice(span.clone(), new_branches))
        }
        (Type::Forall(_, name1, body1), Type::Forall(_, name2, body2)) => Ok(Type::Forall(
            span.clone(),
            crate::frontend_impl::language::TypeParameter {
                name: name1.name.clone(),
                constraint: name1.constraint.broader(name2.constraint),
            },
            Box::new(intersect_types(
                typedefs,
                span,
                body1,
                &body2.clone().substitute(BTreeMap::from([(
                    &name2.name,
                    &Type::Var(Span::None, name1.name.clone()),
                )]))?,
            )?),
        )),
        (Type::Exists(_, name1, body1), Type::Exists(_, name2, body2)) => Ok(Type::Exists(
            span.clone(),
            crate::frontend_impl::language::TypeParameter {
                name: name1.name.clone(),
                // Covariant in the constraint, unlike `Forall`.
                constraint: name1.constraint.narrower(name2.constraint),
            },
            Box::new(intersect_types(
                typedefs,
                span,
                body1,
                &body2.clone().substitute(BTreeMap::from([(
                    &name2.name,
                    &Type::Var(Span::None, name1.name.clone()),
                )]))?,
            )?),
        )),
        (Type::Box(_, t1), t2) => intersect_types(typedefs, span, t1, t2),
        (t1, Type::Box(_, t2)) => intersect_types(typedefs, span, t1, t2),
        (t1, t2) => Err(TypeError::TypesCannotBeUnified(
            span.clone(),
            t1.clone(),
            t2.clone(),
        )),
    }
}
