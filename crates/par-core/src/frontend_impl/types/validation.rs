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
        self.satisfies_constraint_with(constraint, false, defs, &mut Vec::new())
    }

    /// Whether a `box<constraint>` can capture a value of this type: it must satisfy
    /// `constraint`, except that it may also be built from boxes whose own captures do.
    /// So `box<data>` values compose, while still never reaching a plain `box`.
    pub fn can_be_captured_by_box(
        &self,
        constraint: TypeConstraint,
        defs: &TypeDefs<S>,
    ) -> Result<bool, TypeError<S>> {
        self.satisfies_constraint_with(constraint, true, defs, &mut Vec::new())
    }

    fn satisfies_constraint_with(
        &self,
        constraint: TypeConstraint,
        capture: bool,
        defs: &TypeDefs<S>,
        fixpoints: &mut Vec<(Option<LocalName>, FixpointKind)>,
    ) -> Result<bool, TypeError<S>> {
        if constraint == TypeConstraint::Any {
            return Ok(true);
        }

        let satisfies_at_least = |minimum| -> bool { constraint.is_broader_or_equal_than(minimum) };
        // Values of types with binders can't be compared, so they're at most `share`. That
        // doesn't matter for captures, which are never compared, so there they count like
        // any other structure.
        let binders_minimum = if capture {
            TypeConstraint::Data
        } else {
            TypeConstraint::Share
        };

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
            Type::DualName(_, name, args) => {
                defs.get_dual(&self.span(), name, args).and_then(|typ| {
                    typ.satisfies_constraint_with(constraint, capture, defs, fixpoints)
                })
            }
            Type::Name(_, name, args) => defs.get(&self.span(), name, args).and_then(|typ| {
                typ.satisfies_constraint_with(constraint, capture, defs, fixpoints)
            }),
            // A box is as reusable as its captures, but never more than `share`:
            // its code can't be compared or displayed.
            Type::Box(_, captures, _)
                if capture && constraint.is_broader_or_equal_than(*captures) =>
            {
                Ok(true)
            }
            Type::Box(_, captures, _) => {
                Ok(satisfies_at_least(captures.broader(TypeConstraint::Share)))
            }
            Type::Pair(_, left, right, vars) => {
                let minimum = if vars.is_empty() {
                    TypeConstraint::Data
                } else {
                    binders_minimum
                };
                if !satisfies_at_least(minimum) {
                    return Ok(false);
                }
                Self::with_type_parameters(defs, vars, |defs| {
                    Ok(
                        left.satisfies_constraint_with(constraint, capture, defs, fixpoints)?
                            && right
                                .satisfies_constraint_with(constraint, capture, defs, fixpoints)?,
                    )
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
                            .satisfies_constraint_with(constraint, capture, defs, fixpoints)?)
                })
            }
            Type::Choice(_, branches) if constraint == TypeConstraint::Drop => {
                match branches.values().find(|branch| branch.cleanup) {
                    Some(continuation) => continuation
                        .typ
                        .satisfies_constraint_with(constraint, capture, defs, fixpoints),
                    None => Ok(false),
                }
            }
            Type::Recursive { label, body, .. } => {
                fixpoints.push((label.clone(), FixpointKind::Recursive));
                let result = if satisfies_at_least(TypeConstraint::Data) {
                    body.satisfies_constraint_with(constraint, capture, defs, fixpoints)
                } else {
                    Ok(false)
                };
                fixpoints.pop();
                result
            }
            Type::Iterative { label, body, .. } => {
                fixpoints.push((label.clone(), FixpointKind::Iterative));
                let result = if satisfies_at_least(TypeConstraint::Data) {
                    body.satisfies_constraint_with(constraint, capture, defs, fixpoints)
                } else {
                    Ok(false)
                };
                fixpoints.pop();
                result
            }
            Type::Exists(_, param, body) | Type::Forall(_, param, body) => {
                if !satisfies_at_least(binders_minimum) {
                    return Ok(false);
                }
                Self::with_type_parameter(defs, param, |defs| {
                    body.satisfies_constraint_with(constraint, capture, defs, fixpoints)
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
