use super::super::language::LocalName;
use super::core::Type;
use super::error::TypeError;
use crate::frontend_impl::types::visit;
use crate::location::Span;
use std::collections::BTreeMap;

impl<S: Clone> Type<S> {
    pub fn substitute(self, map: BTreeMap<&LocalName, &Type<S>>) -> Result<Self, TypeError<S>> {
        fn inner<S: Clone>(
            typ: &mut Type<S>,
            map: &BTreeMap<&LocalName, &Type<S>>,
        ) -> Result<(), TypeError<S>> {
            match typ {
                Type::Var(_span, name) if map.contains_key(name) => {
                    *typ = map.get(name).cloned().cloned().unwrap();
                }
                Type::DualVar(_span, name) if map.contains_key(name) => {
                    *typ = map.get(name).cloned().cloned().unwrap().dual(Span::None);
                }
                Type::Exists(_span, param, body) | Type::Forall(_span, param, body) => {
                    let old_name = param.name.clone();
                    while map.values().any(|t| t.contains_var(&param.name)) {
                        param.name.string = format!("{}'", param.name.string).into();
                    }
                    if old_name != param.name {
                        inner(
                            body,
                            &BTreeMap::from([(
                                &old_name,
                                &Type::Var(param.name.span.clone(), param.name.clone()),
                            )]),
                        )?;
                    }
                    let mut map = map.clone();
                    map.remove(&old_name);
                    inner(body, &map)?
                }
                Type::Recursive {
                    body, display_hint, ..
                }
                | Type::Iterative {
                    body, display_hint, ..
                } => {
                    inner(body, map)?;
                    if let Some(display_hint) = display_hint.0.as_mut() {
                        for arg in &mut display_hint.args {
                            inner(arg, map)?;
                        }
                    }
                }
                _ => {
                    visit::continue_mut(typ, |child: &mut Type<S>| inner(child, map))?;
                }
            }
            Ok(())
        }

        let mut typ = self;
        inner(&mut typ, &map)?;
        Ok(typ)
    }

    pub fn contains_var(&self, var: &LocalName) -> bool {
        fn inner<S>(result: &mut bool, typ: &Type<S>, target_name: &LocalName) -> Result<(), ()> {
            match typ {
                Type::Var(_span, name) | Type::DualVar(_span, name) if name == target_name => {
                    *result = true;
                }
                Type::Forall(_, param, _) | Type::Exists(_, param, _)
                    if &param.name == target_name =>
                {
                    // var is shadowed
                }
                _ => {
                    visit::continue_(typ, |child| inner(result, child, target_name))?;
                }
            }
            Ok(())
        }
        let mut result = false;
        inner(&mut result, self, var).unwrap();
        result
    }

    pub fn substitute_inferred_holes(self, map: &BTreeMap<LocalName, Type<S>>) -> Self {
        fn inner<S: Clone>(typ: &mut Type<S>, map: &BTreeMap<LocalName, Type<S>>) {
            match typ {
                Type::Hole(_span, name, _) => {
                    if let Some(replacement) = map.get(name) {
                        *typ = replacement.clone();
                    }
                }
                Type::DualHole(_span, name, _) => {
                    if let Some(replacement) = map.get(name) {
                        *typ = replacement.clone().dual(Span::None);
                    }
                }
                Type::Recursive {
                    body, display_hint, ..
                }
                | Type::Iterative {
                    body, display_hint, ..
                } => {
                    inner(body, map);
                    if let Some(display_hint) = display_hint.0.as_mut() {
                        for arg in &mut display_hint.args {
                            inner(arg, map);
                        }
                    }
                }
                _ => {
                    visit::continue_mut(typ, |child| {
                        inner(child, map);
                        Ok::<_, ()>(())
                    })
                    .unwrap();
                }
            }
        }

        let mut typ = self;
        inner(&mut typ, map);
        typ
    }
}
