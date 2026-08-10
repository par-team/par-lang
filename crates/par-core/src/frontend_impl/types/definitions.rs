use crate::frontend_impl::language::{
    GlobalName, ImplicitParameter, LocalName, TypeConstraint, TypeParameter,
};
use crate::frontend_impl::types::core::NamedTypeDisplay;
use crate::frontend_impl::types::{Type, TypeError, visit};
use crate::location::Span;
use indexmap::{IndexMap, IndexSet};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct TypeDefs<S> {
    pub globals: Arc<IndexMap<GlobalName<S>, (Span, Vec<TypeParameter>, Type<S>)>>,
    pub vars: IndexMap<LocalName, TypeConstraint>,
    pub size_vars: IndexSet<LocalName>,
}

impl<S: Clone + Eq + std::hash::Hash> Default for TypeDefs<S> {
    fn default() -> Self {
        Self {
            globals: Default::default(),
            vars: Default::default(),
            size_vars: Default::default(),
        }
    }
}

impl<S: Clone + Eq + std::hash::Hash> TypeDefs<S> {
    pub fn new_with_validation<'a>(
        globals: impl Iterator<
            Item = (
                &'a Span,
                &'a GlobalName<S>,
                &'a Vec<TypeParameter>,
                &'a Type<S>,
            ),
        >,
    ) -> (Self, IndexSet<TypeError<S>>)
    where
        S: 'a,
    {
        let mut globals_map = IndexMap::new();
        let mut errors = IndexSet::new();
        for (span, name, params, typ) in globals {
            if let Some((span1, _, _)) =
                globals_map.insert(name.clone(), (span.clone(), params.clone(), typ.clone()))
            {
                errors.insert(TypeError::TypeNameAlreadyDefined(
                    span.clone(),
                    span1.clone(),
                    name.clone(),
                ));
            }
        }

        let type_defs = Self {
            globals: Arc::new(globals_map),
            vars: Default::default(),
            size_vars: Default::default(),
        };

        let mut deps_map: IndexMap<GlobalName<S>, Vec<GlobalName<S>>> = Default::default();
        for (name, (_, _, typ)) in type_defs.globals.iter() {
            deps_map.insert(name.clone(), typ.get_dependencies());
        }

        for (name, _) in type_defs.globals.iter() {
            if let Err(e) = type_defs.validate_acyclic(name, &Default::default(), &deps_map) {
                errors.insert(e);
            }
        }

        for (_, (_, params, typ)) in type_defs.globals.iter() {
            let mut type_defs = type_defs.clone();
            type_defs.extend_implicit_vars(params.iter().cloned().map(ImplicitParameter::Type));
            if let Err(e) = type_defs.validate_type(typ) {
                errors.insert(e);
            }
        }

        (type_defs, errors)
    }

    pub fn get(
        &self,
        span: &Span,
        name: &GlobalName<S>,
        args: &[Type<S>],
    ) -> Result<Type<S>, TypeError<S>> {
        self.get_with_span(span, name, args).map(|(_, typ)| typ)
    }

    pub fn get_with_span(
        &self,
        span: &Span,
        name: &GlobalName<S>,
        args: &[Type<S>],
    ) -> Result<(&Span, Type<S>), TypeError<S>> {
        match self.globals.get(name) {
            Some((definition_span, params, typ)) => {
                if params.len() != args.len() {
                    return Err(TypeError::WrongNumberOfTypeArgs(
                        span.clone(),
                        name.clone(),
                        params.len(),
                        args.len(),
                    ));
                }
                let typ = typ
                    .clone()
                    .substitute(params.iter().map(|param| &param.name).zip(args).collect())?
                    .with_display_hint(NamedTypeDisplay::new(name.clone(), args.to_vec(), false));
                Ok((definition_span, typ))
            }
            None => Err(TypeError::TypeNameNotDefined(span.clone(), name.clone())),
        }
    }

    pub fn get_dual(
        &self,
        span: &Span,
        name: &GlobalName<S>,
        args: &[Type<S>],
    ) -> Result<Type<S>, TypeError<S>> {
        self.get_dual_with_span(span, name, args)
            .map(|(_, typ)| typ)
    }

    pub fn get_dual_with_span(
        &self,
        span: &Span,
        name: &GlobalName<S>,
        args: &[Type<S>],
    ) -> Result<(&Span, Type<S>), TypeError<S>> {
        match self.globals.get(name) {
            Some((definition_span, params, typ)) => {
                if params.len() != args.len() {
                    return Err(TypeError::WrongNumberOfTypeArgs(
                        span.clone(),
                        name.clone(),
                        params.len(),
                        args.len(),
                    ));
                }
                let typ = typ
                    .clone()
                    .dual(Span::None)
                    .substitute(params.iter().map(|param| &param.name).zip(args).collect())?
                    .with_display_hint(NamedTypeDisplay::new(name.clone(), args.to_vec(), true));
                Ok((definition_span, typ))
            }
            None => Err(TypeError::TypeNameNotDefined(span.clone(), name.clone())),
        }
    }

    pub fn insert_var(&mut self, param: TypeParameter) {
        self.vars.insert(param.name.clone(), param.constraint);
    }

    pub fn insert_implicit_var(&mut self, param: ImplicitParameter) {
        match param {
            ImplicitParameter::Type(param) => self.insert_var(param),
            ImplicitParameter::Size(param) => {
                self.size_vars.insert(param.name);
            }
        }
    }

    pub fn extend_implicit_vars(&mut self, params: impl IntoIterator<Item = ImplicitParameter>) {
        for param in params {
            self.insert_implicit_var(param);
        }
    }

    pub fn contains_size_var(&self, name: &LocalName) -> bool {
        self.size_vars.contains(name)
    }

    pub fn contains_var(&self, name: &LocalName) -> bool {
        self.vars.contains_key(name)
    }

    pub fn var_constraint(&self, name: &LocalName) -> Option<TypeConstraint> {
        self.vars.get(name).copied()
    }

    pub fn validate_acyclic(
        &self,
        name: &GlobalName<S>,
        deps_stack: &IndexSet<GlobalName<S>>,
        deps_map: &IndexMap<GlobalName<S>, Vec<GlobalName<S>>>,
    ) -> Result<(), TypeError<S>> {
        let mut deps_stack = deps_stack.clone();
        if !deps_stack.insert(name.clone()) {
            return Err(TypeError::DependencyCycle(
                self.globals[name].0.clone(),
                deps_stack
                    .clone()
                    .into_iter()
                    .skip_while(|dep| dep != name)
                    .collect(),
            ));
        }
        if let Some(deps) = deps_map.get(name) {
            for dep in deps {
                self.validate_acyclic(dep, &deps_stack, deps_map)?;
            }
        }
        Ok(())
    }

    pub fn validate_type(&self, typ: &Type<S>) -> Result<(), TypeError<S>> {
        #[derive(Clone)]
        struct Ctx<S> {
            defs: TypeDefs<S>,
            check_self: bool,
            self_polarity: IndexMap<Option<LocalName>, bool>,
            unguarded_self_rec: IndexSet<Option<LocalName>>,
            unguarded_self_iter: IndexSet<Option<LocalName>>,
        }
        fn inner<S: Clone + Eq + std::hash::Hash>(
            typ: &Type<S>,
            positive: bool,
            mut ctx: Ctx<S>,
        ) -> Result<(), TypeError<S>> {
            match typ {
                Type::Name(_span, _name, args) | Type::DualName(_span, _name, args) => {
                    for arg in args {
                        inner(
                            arg,
                            positive,
                            Ctx {
                                defs: ctx.defs.clone(),
                                check_self: false,
                                self_polarity: IndexMap::new(),
                                unguarded_self_rec: IndexSet::new(),
                                unguarded_self_iter: IndexSet::new(),
                            },
                        )?;
                    }
                    visit::continue_deref_polarized(typ, positive, &ctx.defs, |typ, positive| {
                        inner(typ, positive, ctx.clone())
                    })?;
                }
                Type::Function(span, arg, res, vars) if !vars.is_empty() => {
                    ctx.defs.extend_implicit_vars(vars.iter().cloned());
                    visit::continue_deref_polarized(typ, positive, &ctx.defs, |_typ, positive| {
                        inner(
                            &Type::Function(span.clone(), arg.clone(), res.clone(), vec![]),
                            positive,
                            ctx.clone(),
                        )
                    })?;
                }
                Type::Pair(span, arg, res, vars) if !vars.is_empty() => {
                    ctx.defs.extend_implicit_vars(vars.iter().cloned());
                    visit::continue_deref_polarized(typ, positive, &ctx.defs, |_typ, positive| {
                        inner(
                            &Type::Pair(span.clone(), arg.clone(), res.clone(), vec![]),
                            positive,
                            ctx.clone(),
                        )
                    })?;
                }
                Type::Exists(_span, param, _body) | Type::Forall(_span, param, _body) => {
                    ctx.defs.insert_var(param.clone());
                    visit::continue_deref_polarized(typ, positive, &ctx.defs, |typ, positive| {
                        inner(typ, positive, ctx.clone())
                    })?;
                }
                Type::Var(span, name) | Type::DualVar(span, name) => {
                    if ctx.defs.contains_var(name) {
                        ()
                    } else {
                        return Err(TypeError::TypeVariableNotDefined(
                            span.clone(),
                            name.clone(),
                        ));
                    }
                }
                Type::Recursive { label, .. } if ctx.check_self => {
                    ctx.unguarded_self_rec.insert(label.clone());
                    ctx.unguarded_self_iter.shift_remove(label);
                    ctx.self_polarity.insert(label.clone(), positive);
                    visit::continue_deref_polarized(typ, positive, &ctx.defs, |typ, positive| {
                        inner(typ, positive, ctx.clone())
                    })?;
                }
                Type::Iterative { label, .. } if ctx.check_self => {
                    ctx.unguarded_self_iter.insert(label.clone());
                    ctx.unguarded_self_rec.shift_remove(label);
                    ctx.self_polarity.insert(label.clone(), positive);
                    visit::continue_deref_polarized(typ, positive, &ctx.defs, |typ, positive| {
                        inner(typ, positive, ctx.clone())
                    })?;
                }
                Type::Either(..) if ctx.check_self => {
                    ctx.unguarded_self_rec = IndexSet::new();
                    visit::continue_deref_polarized(typ, positive, &ctx.defs, |typ, positive| {
                        inner(typ, positive, ctx.clone())
                    })?;
                }
                Type::Choice(..) if ctx.check_self => {
                    ctx.unguarded_self_iter = IndexSet::new();
                    visit::continue_deref_polarized(typ, positive, &ctx.defs, |typ, positive| {
                        inner(typ, positive, ctx.clone())
                    })?;
                }
                Type::Self_(span, label) if ctx.check_self => {
                    if let Some(is_positive) = ctx.self_polarity.get(label) {
                        if *is_positive != positive {
                            return Err(TypeError::SelfUsedInNegativePosition(span.clone()));
                        }
                    } else {
                        return Err(TypeError::NoMatchingRecursiveOrIterative(span.clone()));
                    }
                    if ctx.unguarded_self_rec.contains(label) {
                        return Err(TypeError::UnguardedRecursiveSelf(span.clone()));
                    }
                    if ctx.unguarded_self_iter.contains(label) {
                        return Err(TypeError::UnguardedIterativeSelf(span.clone()));
                    }
                }
                Type::DualSelf(span, label) if ctx.check_self => {
                    if let Some(is_positive) = ctx.self_polarity.get(label) {
                        if *is_positive == positive {
                            return Err(TypeError::SelfUsedInNegativePosition(span.clone()));
                        }
                    } else {
                        return Err(TypeError::NoMatchingRecursiveOrIterative(span.clone()));
                    }
                    if ctx.unguarded_self_rec.contains(label) {
                        return Err(TypeError::UnguardedRecursiveSelf(span.clone()));
                    }
                    if ctx.unguarded_self_iter.contains(label) {
                        return Err(TypeError::UnguardedIterativeSelf(span.clone()));
                    }
                }
                _ => visit::continue_deref_polarized(typ, positive, &ctx.defs, |typ, positive| {
                    inner(typ, positive, ctx.clone())
                })?,
            }
            Ok(())
        }
        inner(
            typ,
            true,
            Ctx {
                defs: self.clone(),
                check_self: true,
                self_polarity: IndexMap::new(),
                unguarded_self_rec: IndexSet::new(),
                unguarded_self_iter: IndexSet::new(),
            },
        )
    }
}
