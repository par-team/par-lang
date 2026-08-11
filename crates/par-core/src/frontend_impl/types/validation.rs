use super::TypeDefs;
use super::core::Type;
use super::error::TypeError;
use crate::frontend::PrimitiveType;
use crate::frontend_impl::language::TypeConstraint;
use crate::location::Spanning;

impl<S: Clone + Eq + std::hash::Hash> Type<S> {
    pub fn is_linear(&self, type_defs: &TypeDefs<S>) -> Result<bool, TypeError<S>> {
        Ok(!self.satisfies_constraint(TypeConstraint::Box, type_defs)?)
    }

    pub fn satisfies_constraint(
        &self,
        constraint: TypeConstraint,
        defs: &TypeDefs<S>,
    ) -> Result<bool, TypeError<S>>
    where
        S: Clone + Eq + std::hash::Hash,
    {
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
            Type::Primitive(..) | Type::Break(_) | Type::Self_(..) | Type::DualSelf(..) => {
                Ok(satisfies_at_least(TypeConstraint::Data))
            }
            Type::Var(_, name) => Ok(defs
                .var_constraint(name)
                .is_some_and(|actual| constraint.is_broader_or_equal_than(actual))),
            Type::DualName(_, name, args) => defs
                .get_dual(&self.span(), name, args)
                .and_then(|typ| typ.satisfies_constraint(constraint, defs)),
            Type::Name(_, name, args) => defs
                .get(&self.span(), name, args)
                .and_then(|typ| typ.satisfies_constraint(constraint, defs)),
            Type::SizedName(_, sizes, name, args) => defs
                .get_sized(&self.span(), sizes, name, args)
                .and_then(|typ| typ.satisfies_constraint(constraint, defs)),
            Type::SizedDualName(_, sizes, name, args) => defs
                .get_sized_dual(&self.span(), sizes, name, args)
                .and_then(|typ| typ.satisfies_constraint(constraint, defs)),
            Type::Box(_, typ) => Ok(satisfies_at_least(TypeConstraint::Box)
                || typ.satisfies_constraint(constraint, defs)?),
            Type::Pair(_, left, right, vars) => {
                let minimum = if vars.is_empty() {
                    TypeConstraint::Data
                } else {
                    TypeConstraint::Box
                };
                if !satisfies_at_least(minimum) {
                    return Ok(false);
                }
                Self::with_implicit_parameters(defs, vars, |defs| {
                    Ok(left.satisfies_constraint(constraint, defs)?
                        && right.satisfies_constraint(constraint, defs)?)
                })
            }
            Type::Either(_, branches) => {
                if !satisfies_at_least(TypeConstraint::Data) {
                    return Ok(false);
                }
                branches.values().try_fold(true, |acc, branch| {
                    Ok(acc && branch.satisfies_constraint(constraint, defs)?)
                })
            }
            Type::Recursive { body, .. } | Type::Iterative { body, .. } => {
                if !satisfies_at_least(TypeConstraint::Data) {
                    return Ok(false);
                }
                body.satisfies_constraint(constraint, defs)
            }
            Type::Exists(_, param, body) | Type::Forall(_, param, body) => {
                if !satisfies_at_least(TypeConstraint::Box) {
                    return Ok(false);
                }
                Self::with_type_parameter(defs, param, |defs| {
                    body.satisfies_constraint(constraint, defs)
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

    pub fn validate_sized(&self, defs: &TypeDefs<S>) -> Result<(), TypeError<S>>
    where
        S: Clone + Eq + std::hash::Hash,
    {
        match self {
            Type::SizedName(span, _, name, args) => {
                for arg in args {
                    arg.validate_sized(defs)?;
                }
                let mut expanded = defs.get(span, name, args)?;
                while matches!(
                    expanded,
                    Type::Name(..)
                        | Type::DualName(..)
                        | Type::SizedName(..)
                        | Type::SizedDualName(..)
                ) {
                    if let Ok(next) = expanded.clone().expand_definition(defs) {
                        if next == expanded {
                            break;
                        }
                        expanded = next;
                    } else {
                        break;
                    }
                }
                if !expanded.is_fixpoint() {
                    return Err(TypeError::CannotApplySized(span.clone(), expanded));
                }
                expanded.validate_sized(defs)?;
            }
            Type::SizedDualName(span, _, name, args) => {
                for arg in args {
                    arg.validate_sized(defs)?;
                }
                let mut expanded = defs.get_dual(span, name, args)?;
                while matches!(
                    expanded,
                    Type::Name(..)
                        | Type::DualName(..)
                        | Type::SizedName(..)
                        | Type::SizedDualName(..)
                ) {
                    if let Ok(next) = expanded.clone().expand_definition(defs) {
                        if next == expanded {
                            break;
                        }
                        expanded = next;
                    } else {
                        break;
                    }
                }
                if !expanded.is_fixpoint() {
                    return Err(TypeError::CannotApplySized(span.clone(), expanded));
                }
                expanded.validate_sized(defs)?;
            }
            Type::Name(_, _, args) | Type::DualName(_, _, args) => {
                for arg in args {
                    arg.validate_sized(defs)?;
                }
            }
            Type::Box(_, body) | Type::DualBox(_, body) => {
                body.validate_sized(defs)?;
            }
            Type::Pair(_, left, right, vars) | Type::Function(_, left, right, vars) => {
                Self::with_implicit_parameters(defs, vars, |defs| {
                    left.validate_sized(defs)?;
                    right.validate_sized(defs)?;
                    Ok(())
                })?;
            }
            Type::Either(_, branches) | Type::Choice(_, branches) => {
                for branch in branches.values() {
                    branch.validate_sized(defs)?;
                }
            }
            Type::Recursive { body, .. } | Type::Iterative { body, .. } => {
                body.validate_sized(defs)?;
            }
            Type::Exists(_, param, body) | Type::Forall(_, param, body) => {
                Self::with_type_parameter(defs, param, |defs| body.validate_sized(defs))?;
            }
            Type::Hole(..) | Type::DualHole(..) => {}
            Type::Primitive(..)
            | Type::DualPrimitive(..)
            | Type::Var(..)
            | Type::DualVar(..)
            | Type::Break(..)
            | Type::Continue(..)
            | Type::Self_(..)
            | Type::DualSelf(..)
            | Type::Fail(..) => {}
        }
        Ok(())
    }
}
