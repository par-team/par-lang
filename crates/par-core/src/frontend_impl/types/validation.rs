use super::TypeDefs;
use super::core::Type;
use super::error::TypeError;
use crate::frontend::PrimitiveType;
use crate::frontend_impl::language::{LocalName, TypeConstraint};
use crate::location::Spanning;

#[derive(Clone, Copy)]
enum FixpointKind {
    Recursive,
    Iterative,
}

impl<S: Clone + Eq + std::hash::Hash> Type<S> {
    pub fn is_linear(&self, type_defs: &TypeDefs<S>) -> Result<bool, TypeError<S>> {
        Ok(!self.satisfies_constraint(TypeConstraint::Share, type_defs)?)
    }

    pub fn is_drop(&self, type_defs: &TypeDefs<S>) -> Result<bool, TypeError<S>> {
        self.satisfies_constraint(TypeConstraint::Drop, type_defs)
    }

    pub fn satisfies_constraint(
        &self,
        constraint: TypeConstraint,
        defs: &TypeDefs<S>,
    ) -> Result<bool, TypeError<S>>
    where
        S: Clone + Eq + std::hash::Hash,
    {
        self.satisfies_constraint_with(constraint, defs, &mut Vec::new())
    }

    fn satisfies_constraint_with(
        &self,
        constraint: TypeConstraint,
        defs: &TypeDefs<S>,
        fixpoints: &mut Vec<(Option<LocalName>, FixpointKind)>,
    ) -> Result<bool, TypeError<S>> {
        if constraint == TypeConstraint::Any {
            return Ok(true);
        }

        let satisfies_at_least = |minimum| -> bool { constraint.is_broader_or_equal_than(minimum) };

        match self {
            Type::Primitive(_, PrimitiveType::Nat) => {
                Ok(satisfies_at_least(TypeConstraint::Number))
            }
            Type::Primitive(_, PrimitiveType::Int | PrimitiveType::Float) => {
                Ok(satisfies_at_least(TypeConstraint::Signed))
            }
            Type::Primitive(..) | Type::Break(_) => Ok(satisfies_at_least(TypeConstraint::Data)),
            Type::DualSelf(..) if constraint == TypeConstraint::Drop => Ok(false),
            Type::DualSelf(..) => Ok(satisfies_at_least(TypeConstraint::Data)),
            Type::Self_(_, label) if constraint == TypeConstraint::Drop => Ok(fixpoints
                .iter()
                .rev()
                .find(|(bound, _)| bound == label)
                .is_some_and(|(_, kind)| matches!(kind, FixpointKind::Recursive))),
            Type::Self_(..) => Ok(satisfies_at_least(TypeConstraint::Data)),
            Type::Var(_, name) => Ok(defs
                .var_constraint(name)
                .is_some_and(|actual| constraint.is_broader_or_equal_than(actual))),
            Type::DualName(_, name, args) => defs
                .get_dual(&self.span(), name, args)
                .and_then(|typ| typ.satisfies_constraint_with(constraint, defs, fixpoints)),
            Type::Name(_, name, args) => defs
                .get(&self.span(), name, args)
                .and_then(|typ| typ.satisfies_constraint_with(constraint, defs, fixpoints)),
            Type::Box(_, typ) => Ok(satisfies_at_least(TypeConstraint::Share)
                || typ.satisfies_constraint_with(constraint, defs, fixpoints)?),
            Type::Pair(_, left, right, vars) => {
                let minimum = if vars.is_empty() {
                    TypeConstraint::Data
                } else {
                    TypeConstraint::Share
                };
                if !satisfies_at_least(minimum) {
                    return Ok(false);
                }
                Self::with_type_parameters(defs, vars, |defs| {
                    Ok(left.satisfies_constraint_with(constraint, defs, fixpoints)?
                        && right.satisfies_constraint_with(constraint, defs, fixpoints)?)
                })
            }
            Type::Either(_, branches) => {
                if !satisfies_at_least(TypeConstraint::Data) {
                    return Ok(false);
                }
                branches.values().try_fold(true, |acc, branch| {
                    Ok(acc
                        && branch
                            .typ
                            .satisfies_constraint_with(constraint, defs, fixpoints)?)
                })
            }
            Type::Choice(_, branches) if constraint == TypeConstraint::Drop => {
                match branches.values().find(|branch| branch.cleanup) {
                    Some(continuation) => continuation
                        .typ
                        .satisfies_constraint_with(constraint, defs, fixpoints),
                    None => Ok(false),
                }
            }
            Type::Recursive { label, body, .. } => {
                fixpoints.push((label.clone(), FixpointKind::Recursive));
                let result = if satisfies_at_least(TypeConstraint::Data) {
                    body.satisfies_constraint_with(constraint, defs, fixpoints)
                } else {
                    Ok(false)
                };
                fixpoints.pop();
                result
            }
            Type::Iterative { label, body, .. } => {
                fixpoints.push((label.clone(), FixpointKind::Iterative));
                let result = if satisfies_at_least(TypeConstraint::Data) {
                    body.satisfies_constraint_with(constraint, defs, fixpoints)
                } else {
                    Ok(false)
                };
                fixpoints.pop();
                result
            }
            Type::Exists(_, param, body) | Type::Forall(_, param, body) => {
                if !satisfies_at_least(TypeConstraint::Share) {
                    return Ok(false);
                }
                Self::with_type_parameter(defs, param, |defs| {
                    body.satisfies_constraint_with(constraint, defs, fixpoints)
                })
            }
            Type::Fail(_) => Ok(true),
            Type::DualPrimitive(..)
            | Type::DualVar(..)
            | Type::DualBox(..)
            | Type::Function(..)
            | Type::Choice(..)
            | Type::Continue(_)
            | Type::Hole(..)
            | Type::DualHole(..) => Ok(false),
        }
    }
}
