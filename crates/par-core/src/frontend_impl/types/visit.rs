use super::{Type, TypeDefs, TypeError};
use crate::frontend_impl::language::{GlobalName, LocalName};
use crate::location::Span;
use indexmap::IndexSet;

pub(crate) fn continue_<S, E, F>(typ: &Type<S>, mut visit: F) -> Result<(), E>
where
    F: FnMut(&Type<S>) -> Result<(), E>,
{
    match typ {
        Type::Name(_, _, args) => {
            for arg in args {
                visit(arg)?;
            }
        }
        Type::DualName(_, _, args) => {
            for arg in args {
                visit(arg)?;
            }
        }
        Type::Box(_, inner) | Type::DualBox(_, inner) => {
            visit(inner)?;
        }
        Type::Pair(_, left, right, _) | Type::Function(_, left, right, _) => {
            visit(left)?;
            visit(right)?;
        }
        Type::Either(_, branches) | Type::Choice(_, branches) => {
            for (_name, typ) in branches {
                visit(typ)?;
            }
        }
        Type::Recursive { body, .. } | Type::Iterative { body, .. } => {
            visit(body)?;
        }
        Type::Exists(_, _, body) | Type::Forall(_, _, body) => {
            visit(body)?;
        }
        Type::Primitive(..)
        | Type::DualPrimitive(..)
        | Type::Var(..)
        | Type::DualVar(..)
        | Type::Hole(..)
        | Type::DualHole(..)
        | Type::Break(..)
        | Type::Continue(..)
        | Type::Self_(..)
        | Type::DualSelf(..)
        | Type::Fail(..) => {}
    }
    Ok(())
}

pub(crate) fn continue_mut<S, E, F>(typ: &mut Type<S>, mut visit: F) -> Result<(), E>
where
    F: FnMut(&mut Type<S>) -> Result<(), E>,
{
    match typ {
        Type::Name(_, _, args) => {
            for arg in args {
                visit(arg)?;
            }
        }
        Type::DualName(_, _, args) => {
            for arg in args {
                visit(arg)?;
            }
        }
        Type::Box(_, inner) | Type::DualBox(_, inner) => {
            visit(inner)?;
        }
        Type::Pair(_, left, right, _) | Type::Function(_, left, right, _) => {
            visit(left)?;
            visit(right)?;
        }
        Type::Either(_, branches) | Type::Choice(_, branches) => {
            for (_name, typ) in branches {
                visit(typ)?;
            }
        }
        Type::Recursive { body, .. } | Type::Iterative { body, .. } => {
            visit(body)?;
        }
        Type::Exists(_, _, body) | Type::Forall(_, _, body) => {
            visit(body)?;
        }
        Type::Primitive(..)
        | Type::DualPrimitive(..)
        | Type::Var(..)
        | Type::DualVar(..)
        | Type::Hole(..)
        | Type::DualHole(..)
        | Type::Break(..)
        | Type::Continue(..)
        | Type::Self_(..)
        | Type::DualSelf(..)
        | Type::Fail(..) => {}
    }
    Ok(())
}

pub(crate) fn continue_deref<S: Clone + Eq + std::hash::Hash, F>(
    typ: &Type<S>,
    defs: &TypeDefs<S>,
    mut visit: F,
) -> Result<(), TypeError<S>>
where
    F: FnMut(&Type<S>) -> Result<(), TypeError<S>>,
{
    match typ {
        Type::Name(span, name, args) => {
            let typ = defs.get(span, name, args)?;
            visit(&typ)?;
        }
        Type::DualName(span, name, args) => {
            let typ = defs.get_dual(span, name, args)?;
            visit(&typ)?;
        }
        _ => {
            continue_(typ, visit)?;
        }
    }
    Ok(())
}

pub(crate) fn continue_deref_polarized<S: Clone + Eq + std::hash::Hash, F>(
    typ: &Type<S>,
    is_positive: bool,
    defs: &TypeDefs<S>,
    mut visit: F,
) -> Result<(), TypeError<S>>
where
    F: FnMut(&Type<S>, bool) -> Result<(), TypeError<S>>,
{
    match typ {
        Type::DualName(..) => {
            continue_deref(typ, defs, |child| visit(child, is_positive))?;
        }
        Type::DualBox(_, inner) => {
            visit(inner, !is_positive)?;
        }
        Type::Function(_, left, right, _) => {
            visit(left, !is_positive)?;
            visit(right, is_positive)?;
        }
        _ => {
            continue_deref(typ, defs, |child| visit(child, is_positive))?;
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) enum Polarity {
    Positive,
    Negative,
    Both,
    Neither,
}

impl Polarity {
    pub(crate) fn dual(self) -> Self {
        match self {
            Polarity::Positive => Polarity::Negative,
            Polarity::Negative => Polarity::Positive,
            Polarity::Both => Polarity::Both,
            Polarity::Neither => Polarity::Neither,
        }
    }

    pub(crate) fn xor(self, other: Self) -> Self {
        match self {
            Polarity::Positive => other,
            Polarity::Negative => other.dual(),
            Polarity::Both | Polarity::Neither => self,
        }
    }
}

fn get_args_polarity<S: Clone + Eq + std::hash::Hash>(
    span: &Span,
    name: &GlobalName<S>,
    defs: &TypeDefs<S>,
) -> Result<Vec<Polarity>, TypeError<S>> {
    let Some((_span, vars, body)) = defs.globals.get(name) else {
        return Err(TypeError::GlobalNameNotDefined(span.clone(), name.clone()));
    };

    let mut positive_vars: IndexSet<LocalName> = IndexSet::new();
    let mut negative_vars: IndexSet<LocalName> = IndexSet::new();

    fn inner<S: Clone + Eq + std::hash::Hash>(
        typ: &Type<S>,
        is_positive: bool,
        positive_vars: &mut IndexSet<LocalName>,
        negative_vars: &mut IndexSet<LocalName>,
        names: &IndexSet<LocalName>,
        defs: &TypeDefs<S>,
    ) -> Result<(), TypeError<S>> {
        match typ {
            Type::Var(_, name) if names.contains(name) => {
                if is_positive {
                    positive_vars.insert(name.clone());
                } else {
                    negative_vars.insert(name.clone());
                }
            }
            Type::DualVar(_, name) if names.contains(name) => {
                if is_positive {
                    negative_vars.insert(name.clone());
                } else {
                    positive_vars.insert(name.clone());
                }
            }
            Type::Exists(_, param, body) | Type::Forall(_, param, body) => {
                let mut names = names.clone();
                names.shift_remove(&param.name);
                inner(
                    body,
                    is_positive,
                    positive_vars,
                    negative_vars,
                    &names,
                    defs,
                )?;
            }
            _ => {
                continue_deref_polarized(typ, is_positive, defs, |child, is_positive| {
                    inner(
                        child,
                        is_positive,
                        positive_vars,
                        negative_vars,
                        names,
                        defs,
                    )
                })?;
            }
        }
        Ok(())
    }

    inner(
        body,
        true,
        &mut positive_vars,
        &mut negative_vars,
        &vars.iter().map(|var| var.name.clone()).collect(),
        defs,
    )?;

    Ok(vars
        .iter()
        .map(|var| {
            match (
                positive_vars.contains(&var.name),
                negative_vars.contains(&var.name),
            ) {
                (true, false) => Polarity::Positive,
                (false, true) => Polarity::Negative,
                (true, true) => Polarity::Both,
                (false, false) => Polarity::Neither,
            }
        })
        .collect())
}

pub(crate) fn continue_mut_polarized<S: Clone + Eq + std::hash::Hash, F>(
    typ: &mut Type<S>,
    polarity: Polarity,
    defs: &TypeDefs<S>,
    mut visit: F,
) -> Result<(), TypeError<S>>
where
    F: FnMut(&mut Type<S>, Polarity) -> Result<(), TypeError<S>>,
{
    match typ {
        Type::Name(span, name, args) => {
            for (arg, polarity) in args.iter_mut().zip(get_args_polarity(span, name, defs)?) {
                visit(arg, polarity.xor(polarity))?;
            }
        }
        Type::DualName(span, name, args) => {
            for (arg, polarity) in args.iter_mut().zip(get_args_polarity(span, name, defs)?) {
                visit(arg, polarity.xor(polarity).dual())?;
            }
        }
        Type::DualBox(_, inner) => {
            visit(inner, polarity.dual())?;
        }
        Type::Function(_, left, right, _) => {
            visit(left, polarity.dual())?;
            visit(right, polarity)?;
        }
        _ => {
            continue_mut(typ, |child| visit(child, polarity))?;
        }
    }
    Ok(())
}
