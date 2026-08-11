use crate::frontend::TypeError::CannotAssignFromTo;
use crate::frontend_impl::language::{ImplicitParameter, LocalName, TypeConstraint};
use crate::frontend_impl::types::assignability::Assignability;
use crate::frontend_impl::types::core::Hole;
use crate::frontend_impl::types::lattice::{intersect_types, union_types};
use crate::frontend_impl::types::{Type, TypeDefs, TypeError};
use crate::location::Span;
use im::HashMap;
use std::collections::BTreeMap;

fn solve_constraints<S: Clone + Eq + std::hash::Hash>(
    hole: &Hole<S>,
    constraint: TypeConstraint,
    type_defs: &TypeDefs<S>,
    span: &Span,
) -> Result<Type<S>, TypeError<S>> {
    let (lower_bounds, upper_bounds) = hole.get_constraints();
    let mut lower = Type::Either(Span::None, BTreeMap::new());
    let mut upper = Type::Choice(Span::None, BTreeMap::new());
    for typ in lower_bounds {
        lower = union_types(type_defs, span, &lower, &typ)?;
    }
    for typ in upper_bounds {
        upper = intersect_types(type_defs, span, &upper, &typ)?;
    }
    if let Assignability::Incompatible(cause) = lower.require_assignable_to(&upper, type_defs)? {
        let from_type = Type::unroll_type_along_path(lower, &cause.from_path, type_defs);
        let to_type = Type::unroll_type_along_path(upper, &cause.to_path, type_defs);
        return Err(CannotAssignFromTo(span.clone(), from_type, to_type, cause));
    }

    if matches!(constraint, TypeConstraint::Signed)
        && lower
            .is_definitely_assignable_to(&Type::nat(), type_defs)?
            .is_assignable()
        && Type::nat()
            .is_definitely_assignable_to(&lower, type_defs)?
            .is_assignable()
    {
        let promoted = Type::int();
        if promoted
            .is_definitely_assignable_to(&upper, type_defs)?
            .is_assignable()
        {
            return Ok(promoted);
        }
    }

    if let Type::Choice(_, branches) = &upper {
        if branches.is_empty() {
            return Ok(lower);
        }
    }

    if let Type::Either(_, branches) = &lower {
        if branches.is_empty() {
            return Ok(upper);
        }
    }

    Ok(lower)
}

use crate::frontend_impl::types::core::{Size, SizeAnchor, SizeHole};

pub(crate) fn substitute_holes<S: Clone + Eq + std::hash::Hash>(
    pattern: &Type<S>,
    names: &[ImplicitParameter],
) -> Result<
    (
        Type<S>,
        HashMap<LocalName, Hole<S>>,
        HashMap<LocalName, SizeHole>,
    ),
    TypeError<S>,
> {
    let mut holed_pattern = pattern.clone();
    let mut type_holes: HashMap<LocalName, Hole<S>> = HashMap::new();
    let mut size_holes: HashMap<LocalName, SizeHole> = HashMap::new();
    let mut type_subst = BTreeMap::new();
    let mut size_subst = BTreeMap::new();

    for name in names.iter() {
        match name {
            ImplicitParameter::Type(t) => {
                let (hole_typ, hole) = Type::hole(t.name.clone());
                type_holes.insert(t.name.clone(), hole);
                type_subst.insert(&t.name, hole_typ);
            }
            ImplicitParameter::Size(s) => {
                let size_hole = SizeHole::new();
                size_holes.insert(s.name.clone(), size_hole.clone());
                let replacement = vec![Size::LE(SizeAnchor::Hole(s.name.clone(), size_hole))];
                size_subst.insert(&s.name, replacement);
            }
        }
    }

    let size_sub_refs: BTreeMap<_, _> =
        size_subst.iter().map(|(k, v)| (*k, v.as_slice())).collect();
    holed_pattern = holed_pattern
        .substitute_size(&size_sub_refs)
        .substitute(type_subst.iter().map(|(k, v)| (*k, v)).collect())?;

    Ok((holed_pattern, type_holes, size_holes))
}

pub(crate) fn resolve_holes<S: Clone + Eq + std::hash::Hash>(
    span: &Span,
    names: &[ImplicitParameter],
    type_defs: &TypeDefs<S>,
    type_holes: HashMap<LocalName, Hole<S>>,
    size_holes: HashMap<LocalName, SizeHole>,
) -> Result<(BTreeMap<LocalName, Type<S>>, BTreeMap<LocalName, Vec<Size>>), TypeError<S>> {
    let mut res_types = BTreeMap::new();
    let mut res_sizes = BTreeMap::new();
    for name in names {
        match name {
            ImplicitParameter::Type(name) => {
                let hole = type_holes.get(&name.name).unwrap();
                let solved_type = solve_constraints(hole, name.constraint, type_defs, span)?;
                if !solved_type.satisfies_constraint(name.constraint, type_defs)? {
                    return Err(TypeError::TypeDoesNotSatisfyConstraint(
                        span.clone(),
                        name.name.clone(),
                        solved_type,
                        name.constraint,
                    ));
                }
                res_types.insert(name.name.clone(), solved_type);
            }
            ImplicitParameter::Size(name) => {
                let hole = size_holes.get(&name.name).unwrap();
                let bounds = hole.get_bounds();
                let solved_sizes = if bounds.is_empty() {
                    vec![Size::LE(SizeAnchor::Var(name.name.clone()))]
                } else {
                    bounds
                };
                res_sizes.insert(name.name.clone(), solved_sizes);
            }
        }
    }
    Ok((res_types, res_sizes))
}
