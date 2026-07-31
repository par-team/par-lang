use crate::frontend::TypeError::CannotAssignFromTo;
use crate::frontend_impl::language::{LocalName, TypeConstraint, TypeParameter};
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

pub(crate) fn substitute_holes<S: Clone + Eq + std::hash::Hash>(
    pattern: &Type<S>,
    names: &[TypeParameter],
) -> Result<(Type<S>, HashMap<LocalName, Hole<S>>), TypeError<S>> {
    let mut holed_pattern = pattern.clone();
    let mut holes_map: HashMap<LocalName, Hole<S>> = HashMap::new();
    for name in names.iter() {
        let (hole_typ, hole) = Type::hole(name.name.clone());
        holes_map.insert(name.name.clone(), hole);
        holed_pattern = holed_pattern
            .clone()
            .substitute(BTreeMap::from([(&name.name, &hole_typ)]))?;
    }
    Ok((holed_pattern, holes_map))
}

pub(crate) fn resolve_holes<S: Clone + Eq + std::hash::Hash>(
    span: &Span,
    names: &[TypeParameter],
    type_defs: &TypeDefs<S>,
    holes_map: HashMap<LocalName, Hole<S>>,
) -> Result<BTreeMap<LocalName, Type<S>>, TypeError<S>> {
    let mut res = BTreeMap::new();
    for name in names {
        let hole = holes_map.get(&name.name).unwrap();
        let solved_type = solve_constraints(hole, name.constraint, type_defs, span)?;
        if !solved_type.satisfies_constraint(name.constraint, type_defs)? {
            return Err(TypeError::TypeDoesNotSatisfyConstraint(
                span.clone(),
                name.name.clone(),
                solved_type,
                name.constraint,
            ));
        }
        res.insert(name.name.clone(), solved_type);
    }
    Ok(res)
}
