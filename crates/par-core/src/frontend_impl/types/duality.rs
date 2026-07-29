use super::super::language::LocalName;
use super::core::Ignored;
use super::core::Type;
use crate::frontend_impl::types::visit;
use crate::location::Span;

impl<S> Type<S> {
    pub fn dual(self, span0: Span) -> Self {
        match self {
            Self::Primitive(span, p) => Self::DualPrimitive(span0.join(span), p),
            Self::DualPrimitive(span, p) => Self::Primitive(span0.join(span), p),

            Self::Var(span, name) => Self::DualVar(span0.join(span), name),
            Self::DualVar(span, name) => Self::Var(span0.join(span), name),

            Self::Name(span, name, args) => Self::DualName(span0.join(span), name, args),
            Self::DualName(span, name, args) => Self::Name(span0.join(span), name, args),

            Self::Box(span, body) => Self::DualBox(span0.join(span), body),
            Self::DualBox(span, body) => Self::Box(span0.join(span), body),

            Self::Pair(span, t, u, vars) => {
                Self::Function(span0.join(span), t, Box::new(u.dual(Span::None)), vars)
            }
            Self::Function(span, t, u, vars) => {
                Self::Pair(span0.join(span), t, Box::new(u.dual(Span::None)), vars)
            }
            Self::Either(span, branches) => Self::Choice(
                span0.join(span),
                branches
                    .into_iter()
                    .map(|(branch, t)| (branch, t.dual(Span::None)))
                    .collect(),
            ),
            Self::Choice(span, branches) => Self::Either(
                span0.join(span),
                branches
                    .into_iter()
                    .map(|(branch, t)| (branch.clone(), t.dual(Span::None)))
                    .collect(),
            ),
            Self::Break(span) => Self::Continue(span0.join(span)),
            Self::Continue(span) => Self::Break(span0.join(span)),

            Self::Recursive {
                span,
                asc,
                label,
                body: t,
                display_hint,
            } => {
                let body = Box::new(t.dual(Span::None).dualize_self(&label));
                Self::Iterative {
                    span: span0.join(span),
                    asc,
                    label,
                    body,
                    display_hint: Ignored(display_hint.0.map(|display_hint| display_hint.dual())),
                }
            }
            Self::Iterative {
                span,
                asc,
                label,
                body: t,
                display_hint,
            } => {
                let body = Box::new(t.dual(Span::None).dualize_self(&label));
                Self::Recursive {
                    span: span0.join(span),
                    asc,
                    label,
                    body,
                    display_hint: Ignored(display_hint.0.map(|display_hint| display_hint.dual())),
                }
            }
            Self::Self_(span, label) => Self::DualSelf(span0.join(span), label),
            Self::DualSelf(span, label) => Self::Self_(span0.join(span), label),

            Self::Exists(span, param, t) => {
                Self::Forall(span0.join(span), param, Box::new(t.dual(Span::None)))
            }
            Self::Forall(span, param, t) => {
                Self::Exists(span0.join(span), param, Box::new(t.dual(Span::None)))
            }

            Type::Hole(span, name, hole) => Type::DualHole(span0.join(span), name, hole),
            Type::DualHole(span, name, hole) => Type::Hole(span0.join(span), name, hole),

            Type::Fail(span) => Type::Fail(span0.join(span)),
        }
    }

    fn dualize_self(mut self, label: &Option<LocalName>) -> Self {
        fn inner<S>(typ: &mut Type<S>, target_label: &Option<LocalName>) -> Result<(), ()> {
            match typ {
                Type::Self_(span, label) if label == target_label => {
                    *typ = Type::DualSelf(span.clone(), label.clone());
                }
                Type::DualSelf(span, label) if label == target_label => {
                    *typ = Type::Self_(span.clone(), label.clone());
                }
                Type::Recursive { label, .. } | Type::Iterative { label, .. }
                    if label == target_label =>
                {
                    // our label is shadowed
                }
                _ => {
                    visit::continue_mut(typ, |child: &mut Type<S>| inner(child, target_label))?;
                }
            }
            Ok(())
        }
        inner(&mut self, label).unwrap();
        self
    }
}
