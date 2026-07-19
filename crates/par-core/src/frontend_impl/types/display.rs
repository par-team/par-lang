use crate::frontend_impl::language::{GlobalName, LocalName};
use crate::frontend_impl::process::HoverInfo;
use crate::frontend_impl::program::Docs;
use crate::frontend_impl::types::core::NamedTypeDisplay;
use crate::frontend_impl::types::{PrimitiveType, Type, TypeDefs};
use crate::location::{Span, Spanning};
use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write;

pub trait GlobalNameWriter<S> {
    fn write_global_name<W: Write>(&self, f: &mut W, name: &GlobalName<S>) -> fmt::Result;
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TypeRenderOptions {
    indent: usize,
    compact: bool,
    prefer_display_hints: bool,
}

impl TypeRenderOptions {
    pub(crate) const fn pretty(indent: usize) -> Self {
        Self {
            indent,
            compact: false,
            prefer_display_hints: true,
        }
    }

    pub(crate) const fn pretty_compact() -> Self {
        Self {
            indent: 0,
            compact: true,
            prefer_display_hints: true,
        }
    }

    pub(crate) const fn with_prefer_display_hints(self, prefer_display_hints: bool) -> Self {
        Self {
            prefer_display_hints,
            ..self
        }
    }

    fn next_indent(self) -> Self {
        Self {
            indent: self.indent + 1,
            ..self
        }
    }

    fn write_indentation(self, f: &mut impl Write) -> fmt::Result {
        if !self.compact {
            write!(f, "\n")?;
            for _ in 0..self.indent {
                write!(f, "  ")?;
            }
        }
        Ok(())
    }
}

impl<S: Clone> Type<S> {
    pub fn pretty<N: GlobalNameWriter<S>>(
        &self,
        f: &mut impl Write,
        names: &N,
        indent: usize,
    ) -> fmt::Result {
        self.pretty_with_options(f, names, TypeRenderOptions::pretty(indent))
    }

    pub fn pretty_compact<N: GlobalNameWriter<S>>(
        &self,
        f: &mut impl Write,
        names: &N,
    ) -> fmt::Result {
        self.pretty_with_options(f, names, TypeRenderOptions::pretty_compact())
    }

    pub(crate) fn pretty_with_options<N: GlobalNameWriter<S>>(
        &self,
        f: &mut impl Write,
        names: &N,
        options: TypeRenderOptions,
    ) -> fmt::Result {
        write_type_with_options(f, names, self, options)
    }

    pub fn types_at_spans(
        &self,
        type_defs: &TypeDefs<S>,
        docs: &Docs<S>,
        consume: &mut impl FnMut(Span, HoverInfo<S>),
    ) where
        S: Eq + std::hash::Hash,
    {
        match self {
            Self::Primitive(_, _) | Self::DualPrimitive(_, _) => {}
            Self::Var(_, _) | Self::DualVar(_, _) => {}
            Self::Name(span, name, args) => {
                let (def_span, typ) = type_defs
                    .get_with_span(span, name, args)
                    .unwrap_or_else(|_| (&Span::None, self.clone()));
                consume(
                    span.clone(),
                    HoverInfo::type_instantiation(
                        name.clone(),
                        args.clone(),
                        typ,
                        docs.type_doc(name).cloned(),
                        def_span.clone(),
                    ),
                );
                for arg in args {
                    arg.types_at_spans(type_defs, docs, consume);
                }
            }
            Self::DualName(span, name, args) => {
                let (def_span, typ) =
                    type_defs
                        .get_with_span(span, name, args)
                        .unwrap_or_else(|_| {
                            (
                                &Span::None,
                                Type::Name(span.clone(), name.clone(), args.clone()),
                            )
                        });
                consume(
                    dual_name_hover_span(span, name),
                    HoverInfo::type_instantiation(
                        name.clone(),
                        args.clone(),
                        typ,
                        docs.type_doc(name).cloned(),
                        def_span.clone(),
                    ),
                );

                let (_dual_def_span, dual_typ) = type_defs
                    .get_dual_with_span(span, name, args)
                    .unwrap_or_else(|_| (&Span::None, self.clone()));
                consume(
                    dual_keyword_hover_span(span, name),
                    HoverInfo::unnamed(dual_typ),
                );

                for arg in args {
                    arg.types_at_spans(type_defs, docs, consume);
                }
            }
            Self::Box(_, body) | Self::DualBox(_, body) => {
                body.types_at_spans(type_defs, docs, consume)
            }
            Self::Pair(_, t, u, _) => {
                t.types_at_spans(type_defs, docs, consume);
                u.types_at_spans(type_defs, docs, consume);
            }
            Self::Function(_, t, u, _) => {
                t.types_at_spans(type_defs, docs, consume);
                u.types_at_spans(type_defs, docs, consume);
            }
            Self::Either(_, branches) => {
                for (_, t) in branches.iter() {
                    t.types_at_spans(type_defs, docs, consume);
                }
            }
            Self::Choice(_, branches) => {
                for (_, t) in branches.iter() {
                    t.types_at_spans(type_defs, docs, consume);
                }
            }
            Self::Break(_) => {}
            Self::Continue(_) => {}
            Self::Recursive { body, .. } => {
                body.types_at_spans(type_defs, docs, consume);
            }
            Self::Iterative { body, .. } => {
                body.types_at_spans(type_defs, docs, consume);
            }
            Self::Self_(_, _) | Self::DualSelf(_, _) => {}
            Self::Exists(_, _, body) => {
                body.types_at_spans(type_defs, docs, consume);
            }
            Self::Forall(_, _, body) => {
                body.types_at_spans(type_defs, docs, consume);
            }
            Type::Hole(_, _, _) => {}
            Type::DualHole(_, _, _) => {}
            Type::Fail(_) => {}
        }
    }
}

fn write_type_with_options<S: Clone, N: GlobalNameWriter<S>>(
    f: &mut impl Write,
    names: &N,
    typ: &Type<S>,
    options: TypeRenderOptions,
) -> fmt::Result {
    if options.prefer_display_hints {
        if let Some(display_hint) = typ.display_hint() {
            return write_named_type_display(f, names, display_hint, options);
        }
    }

    match typ {
        Type::Primitive(_, primitive) => write_primitive_type(f, primitive),
        Type::DualPrimitive(_, primitive) => {
            write!(f, "dual ")?;
            write_primitive_type(f, primitive)
        }
        Type::Var(_, name) => write!(f, "{name}"),
        Type::DualVar(_, name) => write!(f, "dual {name}"),
        Type::Name(_, name, args) => {
            names.write_global_name(f, name)?;
            write_type_args(f, names, args, options)
        }
        Type::DualName(_, name, args) => {
            write!(f, "dual ")?;
            names.write_global_name(f, name)?;
            write_type_args(f, names, args, options)
        }
        Type::Box(_, body) => {
            write!(f, "box ")?;
            write_type_with_options(f, names, body, options)
        }
        Type::DualBox(_, body) => {
            write!(f, "dual box ")?;
            write_type_with_options(f, names, body, options)
        }
        Type::Pair(_, _, _, _) => write_pair_like(f, names, "(", ")", typ, false, options),
        Type::Function(_, _, _, _) => write_pair_like(f, names, "[", "]", typ, true, options),
        Type::Either(_, branches) => {
            write_braced_branches(f, names, "either", branches, false, options)
        }
        Type::Choice(_, branches) => {
            write_braced_branches(f, names, "choice", branches, true, options)
        }
        Type::Break(_) => write!(f, "!"),
        Type::Continue(_) => write!(f, "?"),
        Type::Recursive { label, body, .. } => {
            write!(f, "recursive")?;
            if !options.compact || !matches!(body.as_ref(), Type::Either(..)) {
                if let Some(label) = label {
                    write!(f, "@{label}")?;
                }
            }
            write!(f, " ")?;
            write_type_with_options(f, names, body, options)
        }
        Type::Iterative { label, body, .. } => {
            write!(f, "iterative")?;
            if !options.compact || !matches!(body.as_ref(), Type::Choice(..)) {
                if let Some(label) = label {
                    write!(f, "@{label}")?;
                }
            }
            write!(f, " ")?;
            write_type_with_options(f, names, body, options)
        }
        Type::Self_(_, label) => {
            write!(f, "self")?;
            if let Some(label) = label {
                write!(f, "@{label}")?;
            }
            Ok(())
        }
        Type::DualSelf(_, label) => {
            write!(f, "dual self")?;
            if let Some(label) = label {
                write!(f, "@{label}")?;
            }
            Ok(())
        }
        Type::Exists(_, _, _) => write_pair_like(f, names, "(", ")", typ, false, options),
        Type::Forall(_, _, _) => write_pair_like(f, names, "[", "]", typ, true, options),
        Type::Hole(_, name, _) => write!(f, "%{name}"),
        Type::DualHole(_, name, _) => write!(f, "dual %{name}"),
        Type::Fail(_) => write!(f, "<error>"),
    }
}

fn write_primitive_type(f: &mut impl Write, primitive: &PrimitiveType) -> fmt::Result {
    let text = match primitive {
        PrimitiveType::Nat => "Nat",
        PrimitiveType::Int => "Int",
        PrimitiveType::Float => "Float",
        PrimitiveType::String => "String",
        PrimitiveType::Char => "Char",
        PrimitiveType::Byte => "Byte",
        PrimitiveType::Bytes => "Bytes",
    };
    write!(f, "{text}")
}

fn write_type_args<S: Clone, N: GlobalNameWriter<S>>(
    f: &mut impl Write,
    names: &N,
    args: &[Type<S>],
    options: TypeRenderOptions,
) -> fmt::Result {
    if args.is_empty() {
        return Ok(());
    }

    write!(f, "<")?;
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        write_type_with_options(f, names, arg, options)?;
    }
    write!(f, ">")
}

fn write_pair_like<S: Clone, N: GlobalNameWriter<S>>(
    f: &mut impl Write,
    names: &N,
    open: &str,
    close: &str,
    typ: &Type<S>,
    function: bool,
    options: TypeRenderOptions,
) -> fmt::Result {
    let mut then = typ;
    let mut wrote_prefix_item = false;

    write!(f, "{open}")?;
    loop {
        match then {
            Type::Forall(_, name, next_then) if function => {
                if wrote_prefix_item {
                    write!(f, ", ")?;
                }
                write!(f, "type {name}")?;
                then = next_then;
            }
            Type::Exists(_, name, next_then) if !function => {
                if wrote_prefix_item {
                    write!(f, ", ")?;
                }
                write!(f, "type {name}")?;
                then = next_then;
            }
            Type::Function(_, arg, next_then, vars) if function => {
                if wrote_prefix_item {
                    write!(f, ", ")?;
                }
                if !vars.is_empty() {
                    write!(f, "<{}", vars[0])?;
                    for var in vars.iter().skip(1) {
                        write!(f, ", {var}")?;
                    }
                    write!(f, "> ")?;
                }
                write_type_with_options(f, names, arg, options)?;
                then = next_then;
            }
            Type::Pair(_, arg, next_then, vars) if !function => {
                if wrote_prefix_item {
                    write!(f, ", ")?;
                }
                if !vars.is_empty() {
                    write!(f, "<{}", vars[0])?;
                    for var in vars.iter().skip(1) {
                        write!(f, ", {var}")?;
                    }
                    write!(f, "> ")?;
                }
                write_type_with_options(f, names, arg, options)?;
                then = next_then;
            }
            _ => break,
        }
        wrote_prefix_item = true;
    }

    let is_terminal = if function {
        matches!(then, Type::Continue(_))
    } else {
        matches!(then, Type::Break(_))
    };
    if is_terminal {
        if function {
            write!(f, "{close}?")
        } else {
            write!(f, "{close}!")
        }
    } else {
        write!(f, "{close} ")?;
        write_type_with_options(f, names, then, options)
    }
}

fn write_braced_branches<S: Clone, N: GlobalNameWriter<S>>(
    f: &mut impl Write,
    names: &N,
    prefix: &str,
    branches: &BTreeMap<LocalName, Type<S>>,
    choice: bool,
    options: TypeRenderOptions,
) -> fmt::Result {
    if branches.is_empty() {
        return write!(f, "{prefix} {{}}");
    }

    write!(f, "{prefix} {{")?;

    for (branch, branch_type) in branches {
        let options = options.next_indent();
        options.write_indentation(f)?;
        write!(f, ".{branch}")?;
        if choice {
            if matches!(branch_type, Type::Function(..)) || matches!(branch_type, Type::Forall(..))
            {
                write_pair_like(f, names, "(", ") =>", branch_type, true, options)?;
            } else {
                write!(f, " => ")?;
                write_type_with_options(f, names, branch_type, options)?;
            }
        } else {
            if matches!(
                branch_type,
                Type::Break(_) | Type::Exists(..) | Type::Pair(..)
            ) {
                // no space between `.foo` and `!`/`(`
            } else {
                write!(f, " ")?;
            }
            write_type_with_options(f, names, branch_type, options)?;
        }
        write!(f, ",")?;
    }
    options.write_indentation(f)?;
    write!(f, "}}")
}

fn write_named_type_display<S: Clone, N: GlobalNameWriter<S>>(
    f: &mut impl Write,
    names: &N,
    display_hint: &NamedTypeDisplay<S>,
    options: TypeRenderOptions,
) -> fmt::Result {
    if display_hint.dual {
        write!(f, "dual ")?;
    }
    names.write_global_name(f, &display_hint.name)?;
    write_type_args(f, names, &display_hint.args, options)
}

fn dual_name_hover_span<S>(full_span: &Span, name: &GlobalName<S>) -> Span {
    match (name.span().start(), full_span.end(), full_span.file()) {
        (Some(start), Some(end), Some(file)) => Span::At { start, end, file },
        _ => name.span(),
    }
}

fn dual_keyword_hover_span<S>(full_span: &Span, name: &GlobalName<S>) -> Span {
    match (full_span.start(), name.span().start(), full_span.file()) {
        (Some(start), Some(end), Some(file)) if start.offset < end.offset => {
            Span::At { start, end, file }
        }
        _ => Span::None,
    }
}
