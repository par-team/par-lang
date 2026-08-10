use crate::frontend_impl::language::LocalName;
use crate::frontend_impl::types::core::{Ignored, NamedTypeDisplay};
use crate::frontend_impl::types::visit;
use crate::frontend_impl::types::{Size, Type, TypeDefs, TypeError};
use crate::location::Span;
use im::HashSet;

impl<S: Clone> Type<S> {
    pub fn expand_definition(&self, type_defs: &TypeDefs<S>) -> Result<Self, TypeError<S>>
    where
        S: Eq + std::hash::Hash,
    {
        match self {
            Self::Name(span, name, args) => type_defs.get(span, name, args),
            Self::DualName(span, name, args) => type_defs.get_dual(span, name, args),
            _ => Ok(self.clone()),
        }
    }

    pub(crate) fn expand_recursive(
        size: &HashSet<Size>,
        label: &Option<LocalName>,
        body: &Self,
        display_hint: Option<&NamedTypeDisplay<S>>,
    ) -> Result<Self, TypeError<S>> {
        let mut typ = body.clone();
        fn inner<S: Clone>(
            typ: &mut Type<S>,
            target_label: &Option<LocalName>,
            size: &HashSet<Size>,
            body: &Type<S>,
            display_hint: Option<&NamedTypeDisplay<S>>,
        ) -> Result<(), TypeError<S>> {
            match typ {
                Type::Self_(span, label) if label == target_label => {
                    *typ = Type::Recursive {
                        span: span.clone(),
                        size: size.iter().map(|s| s.dec()).collect(),
                        label: label.clone(),
                        body: Box::new(body.clone()),
                        display_hint: Ignored(display_hint.cloned()),
                    };
                }
                Type::DualSelf(span, label) if label == target_label => {
                    *typ = Type::Recursive {
                        span: span.clone(),
                        size: size.iter().map(|s| s.dec()).collect(),
                        label: label.clone(),
                        body: Box::new(body.clone()),
                        display_hint: Ignored(display_hint.cloned()),
                    }
                    .dual(Span::None);
                }
                Type::Recursive { label, .. } | Type::Iterative { label, .. }
                    if label == target_label => {}
                _ => {
                    visit::continue_mut(typ, |child: &mut Type<S>| {
                        inner(child, target_label, size, body, display_hint)
                    })?;
                }
            }
            Ok(())
        }

        inner(&mut typ, label, size, body, display_hint)?;
        Ok(typ)
    }

    pub(crate) fn expand_iterative(
        span: &Span,
        size: &HashSet<Size>,
        label: &Option<LocalName>,
        body: &Self,
        display_hint: Option<&NamedTypeDisplay<S>>,
    ) -> Result<Self, TypeError<S>> {
        if size.iter().any(|s| matches!(s, Size::LT(_))) {
            return Err(TypeError::CannotUnrollAscendantIterative(
                span.clone(),
                label.clone(),
            ));
        }

        Type::expand_iterative_unsafe(span, size, label, body, display_hint)
    }

    // This variant does not make sure size isn't empty
    pub(crate) fn expand_iterative_unsafe(
        _span: &Span,
        size: &HashSet<Size>,
        label: &Option<LocalName>,
        body: &Self,
        display_hint: Option<&NamedTypeDisplay<S>>,
    ) -> Result<Self, TypeError<S>> {
        let mut typ = body.clone();
        fn inner<S: Clone>(
            typ: &mut Type<S>,
            target_label: &Option<LocalName>,
            size: &HashSet<Size>,
            body: &Type<S>,
            display_hint: Option<&NamedTypeDisplay<S>>,
        ) -> Result<(), TypeError<S>> {
            match typ {
                Type::Self_(span, label) if label == target_label => {
                    *typ = Type::Iterative {
                        span: span.clone(),
                        size: size.iter().map(|s| s.dec()).collect(),
                        label: label.clone(),
                        body: Box::new(body.clone()),
                        display_hint: Ignored(display_hint.cloned()),
                    };
                }
                Type::DualSelf(span, label) if label == target_label => {
                    *typ = Type::Iterative {
                        span: span.clone(),
                        size: size.iter().map(|s| s.dec()).collect(),
                        label: label.clone(),
                        body: Box::new(body.clone()),
                        display_hint: Ignored(display_hint.cloned()),
                    }
                    .dual(Span::None);
                }
                Type::Recursive { label, .. } | Type::Iterative { label, .. }
                    if label == target_label =>
                {
                    // label is shadowed
                }
                _ => {
                    visit::continue_mut(typ, |child: &mut Type<S>| {
                        inner(child, target_label, size, body, display_hint)
                    })?;
                }
            }
            Ok(())
        }

        inner(&mut typ, label, size, body, display_hint)?;
        Ok(typ)
    }

    #[allow(dead_code)]
    pub fn expand_fixpoint(&self) -> Result<Self, TypeError<S>> {
        match self {
            Type::Recursive {
                size,
                label,
                body,
                display_hint,
                ..
            } => Self::expand_recursive(size, label, body, display_hint.0.as_ref()),
            Type::Iterative {
                span,
                size,
                label,
                body,
                display_hint,
            } => Self::expand_iterative(span, size, label, body, display_hint.0.as_ref()),
            _ => Ok(self.clone()),
        }
    }

    // This variant does not make sure iterative's size isn't empty
    pub fn expand_fixpoint_unfounded(&self) -> Result<Self, TypeError<S>> {
        match self {
            Type::Recursive {
                size,
                label,
                body,
                display_hint,
                ..
            } => Self::expand_recursive(size, label, body, display_hint.0.as_ref()),
            Type::Iterative {
                span,
                size,
                label,
                body,
                display_hint,
            } => Self::expand_iterative_unsafe(span, size, label, body, display_hint.0.as_ref()),
            _ => Ok(self.clone()),
        }
    }
}
