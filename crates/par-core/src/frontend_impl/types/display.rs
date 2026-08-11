use crate::frontend_impl::language::{GlobalName, LocalName};
use crate::frontend_impl::process::HoverInfo;
use crate::frontend_impl::program::Docs;
use crate::frontend_impl::types::core::{
    NamedTypeDisplay, Size, SizeAnchor, TypePath, TypePathSegment,
};
use crate::frontend_impl::types::{PrimitiveType, Type, TypeDefs};
use crate::location::{Span, Spanning};
use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write;

pub trait GlobalNameWriter<S> {
    fn write_global_name<W: Write>(&self, f: &mut W, name: &GlobalName<S>) -> fmt::Result;
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TypeRenderOptions<'a> {
    indent: usize,
    compact: bool,
    prefer_display_hints: bool,
    highlight_path: Option<&'a TypePath>,
    is_underlined: bool,
}

impl<'a> TypeRenderOptions<'a> {
    pub(crate) const fn pretty(indent: usize) -> Self {
        Self {
            indent,
            compact: false,
            prefer_display_hints: true,
            highlight_path: None,
            is_underlined: false,
        }
    }

    pub(crate) const fn pretty_compact() -> Self {
        Self {
            indent: 0,
            compact: true,
            prefer_display_hints: true,
            highlight_path: None,
            is_underlined: false,
        }
    }

    pub(crate) fn with_highlight(mut self, path: &'a TypePath) -> Self {
        if !path.is_empty() {
            self.prefer_display_hints = false;
        }
        self.highlight_path = Some(path);
        self
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
            if self.is_underlined {
                write!(f, "\x1b[24m")?;
            }
            write!(f, "\n")?;
            for _ in 0..self.indent {
                write!(f, "  ")?;
            }
            if self.is_underlined {
                write!(f, "\x1b[4m")?;
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
        options: TypeRenderOptions<'_>,
    ) -> fmt::Result {
        let mut current_path = TypePath::new();
        write_type_with_options(f, names, self, options, &mut current_path)
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
            Self::Name(span, name, args) | Self::SizedName(span, _, name, args) => {
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
            Self::DualName(span, name, args) | Self::SizedDualName(span, _, name, args) => {
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
    options: TypeRenderOptions<'_>,
    current_path: &mut TypePath,
) -> fmt::Result {
    let is_this_node_target = options
        .highlight_path
        .map_or(false, |target| target == current_path);

    let start_underline = is_this_node_target && !options.is_underlined;

    if start_underline {
        write!(f, "\x1b[4m")?;
    }

    let options = TypeRenderOptions {
        is_underlined: options.is_underlined || is_this_node_target,
        ..options
    };

    if options.prefer_display_hints {
        if let Some(display_hint) = typ.display_hint() {
            let res = write_named_type_display(f, names, display_hint, options, current_path);
            if start_underline {
                write!(f, "\x1b[24m")?;
            }
            return res;
        }
    }

    let res = match typ {
        Type::Primitive(_, primitive) => write_primitive_type(f, primitive),
        Type::DualPrimitive(_, primitive) => {
            write!(f, "dual ")?;
            write_primitive_type(f, primitive)
        }
        Type::Var(_, name) => write!(f, "{name}"),
        Type::DualVar(_, name) => write!(f, "dual {name}"),
        Type::Name(_, name, args) => {
            names.write_global_name(f, name)?;
            write_type_args(f, names, args, options, current_path)
        }
        Type::DualName(_, name, args) => {
            write!(f, "dual ")?;
            names.write_global_name(f, name)?;
            write_type_args(f, names, args, options, current_path)
        }
        Type::SizedName(_, sizes, name, args) => {
            for size in sizes {
                match size {
                    Size::LE(SizeAnchor::Var(anchor)) => write!(f, "sized({anchor}) ")?,
                    Size::LT(SizeAnchor::Var(anchor)) => write!(f, "sized(<{anchor}) ")?,
                    _ => {}
                }
            }
            names.write_global_name(f, name)?;
            write_type_args(f, names, args, options, current_path)
        }
        Type::SizedDualName(_, sizes, name, args) => {
            for size in sizes {
                match size {
                    Size::LE(SizeAnchor::Var(anchor)) => write!(f, "sized({anchor}) ")?,
                    Size::LT(SizeAnchor::Var(anchor)) => write!(f, "sized(<{anchor}) ")?,
                    _ => {}
                }
            }
            write!(f, "dual ")?;
            names.write_global_name(f, name)?;
            write_type_args(f, names, args, options, current_path)
        }
        Type::Box(_, body) => {
            write!(f, "box ")?;
            current_path.push(TypePathSegment::BoxBody);
            let r = write_type_with_options(f, names, body, options, current_path);
            current_path.pop();
            r
        }
        Type::DualBox(_, body) => {
            write!(f, "dual box ")?;
            current_path.push(TypePathSegment::BoxBody);
            let r = write_type_with_options(f, names, body, options, current_path);
            current_path.pop();
            r
        }
        Type::Pair(_, _, _, _) => {
            write_pair_like(f, names, "(", ")", typ, false, options, current_path)
        }
        Type::Function(_, _, _, _) => {
            write_pair_like(f, names, "[", "]", typ, true, options, current_path)
        }
        Type::Either(_, branches) => {
            write_braced_branches(f, names, "either", branches, false, options, current_path)
        }
        Type::Choice(_, branches) => {
            write_braced_branches(f, names, "choice", branches, true, options, current_path)
        }
        Type::Break(_) => write!(f, "!"),
        Type::Continue(_) => write!(f, "?"),
        Type::Recursive { size, label, body, .. } => {
            let mut sizes: Vec<_> = size.iter().collect();
            sizes.sort_by_key(|s| match s {
                Size::LE(SizeAnchor::Var(a) | SizeAnchor::Hole(a, _)) => (0, a.string.as_str()),
                Size::LT(SizeAnchor::Var(a) | SizeAnchor::Hole(a, _)) => (1, a.string.as_str()),
                Size::LE(SizeAnchor::LoopId(_)) => (2, ""),
                Size::LT(SizeAnchor::LoopId(_)) => (3, ""),
            });
            for s in sizes {
                match s {
                    Size::LE(SizeAnchor::Var(anchor) | SizeAnchor::Hole(anchor, _)) => write!(f, "sized({anchor}) ")?,
                    Size::LT(SizeAnchor::Var(anchor) | SizeAnchor::Hole(anchor, _)) => write!(f, "sized(<{anchor}) ")?,
                    _ => {}
                }
            }
            write!(f, "recursive")?;
            if !options.compact || !matches!(body.as_ref(), Type::Either(..)) {
                if let Some(label) = label {
                    write!(f, "@{label}")?;
                }
            }
            write!(f, " ")?;
            current_path.push(TypePathSegment::RecursiveBody);
            let r = write_type_with_options(f, names, body, options, current_path);
            current_path.pop();
            r
        }
        Type::Iterative { size, label, body, .. } => {
            let mut sizes: Vec<_> = size.iter().collect();
            sizes.sort_by_key(|s| match s {
                Size::LE(SizeAnchor::Var(a) | SizeAnchor::Hole(a, _)) => (0, a.string.as_str()),
                Size::LT(SizeAnchor::Var(a) | SizeAnchor::Hole(a, _)) => (1, a.string.as_str()),
                Size::LE(SizeAnchor::LoopId(_)) => (2, ""),
                Size::LT(SizeAnchor::LoopId(_)) => (3, ""),
            });
            for s in sizes {
                match s {
                    Size::LE(SizeAnchor::Var(anchor) | SizeAnchor::Hole(anchor, _)) => write!(f, "sized({anchor}) ")?,
                    Size::LT(SizeAnchor::Var(anchor) | SizeAnchor::Hole(anchor, _)) => write!(f, "sized(<{anchor}) ")?,
                    _ => {}
                }
            }
            write!(f, "iterative")?;
            if !options.compact || !matches!(body.as_ref(), Type::Choice(..)) {
                if let Some(label) = label {
                    write!(f, "@{label}")?;
                }
            }
            write!(f, " ")?;
            current_path.push(TypePathSegment::IterativeBody);
            let r = write_type_with_options(f, names, body, options, current_path);
            current_path.pop();
            r
        }
        Type::Self_(_, label) => {
            current_path.push(TypePathSegment::Self_);
            let is_target = options
                .highlight_path
                .map_or(false, |target| target == current_path);
            let start_underline = is_target && !options.is_underlined;
            if start_underline {
                write!(f, "\x1b[4m")?;
            }
            write!(f, "self")?;
            if let Some(label) = label {
                write!(f, "@{label}")?;
            }
            if start_underline {
                write!(f, "\x1b[24m")?;
            }
            current_path.pop();
            Ok(())
        }
        Type::DualSelf(_, label) => {
            current_path.push(TypePathSegment::Self_);
            let is_target = options
                .highlight_path
                .map_or(false, |target| target == current_path);
            let start_underline = is_target && !options.is_underlined;
            if start_underline {
                write!(f, "\x1b[4m")?;
            }
            write!(f, "dual self")?;
            if let Some(label) = label {
                write!(f, "@{label}")?;
            }
            if start_underline {
                write!(f, "\x1b[24m")?;
            }
            current_path.pop();
            Ok(())
        }
        Type::Exists(_, _, _) => {
            write_pair_like(f, names, "(", ")", typ, false, options, current_path)
        }
        Type::Forall(_, _, _) => {
            write_pair_like(f, names, "[", "]", typ, true, options, current_path)
        }
        Type::Hole(_, name, _) => write!(f, "%{name}"),
        Type::DualHole(_, name, _) => write!(f, "dual %{name}"),
        Type::Fail(_) => write!(f, "<error>"),
    };

    if start_underline {
        write!(f, "\x1b[24m")?;
    }

    res
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
    options: TypeRenderOptions<'_>,
    current_path: &mut TypePath,
) -> fmt::Result {
    if args.is_empty() {
        return Ok(());
    }

    write!(f, "<")?;
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        current_path.push(TypePathSegment::NameArg(i));
        write_type_with_options(f, names, arg, options, current_path)?;
        current_path.pop();
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
    options: TypeRenderOptions<'_>,
    current_path: &mut TypePath,
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
                current_path.push(TypePathSegment::TypeParameter(name.name.clone()));
                let is_param_target = options
                    .highlight_path
                    .map_or(false, |target| target == current_path);
                let start_underline = is_param_target && !options.is_underlined;
                if start_underline {
                    write!(f, "\x1b[4m")?;
                }
                write!(f, "type {name}")?;
                if start_underline {
                    write!(f, "\x1b[24m")?;
                }
                current_path.pop();
                then = next_then;
            }
            Type::Exists(_, name, next_then) if !function => {
                if wrote_prefix_item {
                    write!(f, ", ")?;
                }
                current_path.push(TypePathSegment::TypeParameter(name.name.clone()));
                let is_param_target = options
                    .highlight_path
                    .map_or(false, |target| target == current_path);
                let start_underline = is_param_target && !options.is_underlined;
                if start_underline {
                    write!(f, "\x1b[4m")?;
                }
                write!(f, "type {name}")?;
                if start_underline {
                    write!(f, "\x1b[24m")?;
                }
                current_path.pop();
                then = next_then;
            }
            Type::Function(_, arg, next_then, vars) if function => {
                if wrote_prefix_item {
                    write!(f, ", ")?;
                }
                current_path.push(TypePathSegment::ImplicitGenerics);
                let is_vars_target = options
                    .highlight_path
                    .map_or(false, |target| target == current_path);
                let start_underline = is_vars_target && !options.is_underlined;
                if start_underline {
                    write!(f, "\x1b[4m")?;
                }
                if !vars.is_empty() {
                    write!(f, "<")?;
                    for (i, var) in vars.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        current_path.push(TypePathSegment::TypeParameter(var.name().clone()));
                        let is_param_target = options
                            .highlight_path
                            .map_or(false, |target| target == current_path);
                        let start_param_underline =
                            is_param_target && !options.is_underlined && !start_underline;
                        if start_param_underline {
                            write!(f, "\x1b[4m")?;
                        }
                        write!(f, "{var}")?;
                        if start_param_underline {
                            write!(f, "\x1b[24m")?;
                        }
                        current_path.pop();
                    }
                    write!(f, "> ")?;
                } else if is_vars_target {
                    write!(f, "<> ")?;
                }
                if start_underline {
                    write!(f, "\x1b[24m")?;
                }
                current_path.pop();

                current_path.push(TypePathSegment::FunctionParam);
                write_type_with_options(f, names, arg, options, current_path)?;
                current_path.pop();
                then = next_then;
                current_path.push(TypePathSegment::FunctionReturn);
            }
            Type::Pair(_, arg, next_then, vars) if !function => {
                if wrote_prefix_item {
                    write!(f, ", ")?;
                }
                current_path.push(TypePathSegment::ImplicitGenerics);
                let is_vars_target = options
                    .highlight_path
                    .map_or(false, |target| target == current_path);
                let start_underline = is_vars_target && !options.is_underlined;
                if start_underline {
                    write!(f, "\x1b[4m")?;
                }
                if !vars.is_empty() {
                    write!(f, "<")?;
                    for (i, var) in vars.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        current_path.push(TypePathSegment::TypeParameter(var.name().clone()));
                        let is_param_target = options
                            .highlight_path
                            .map_or(false, |target| target == current_path);
                        let start_param_underline =
                            is_param_target && !options.is_underlined && !start_underline;
                        if start_param_underline {
                            write!(f, "\x1b[4m")?;
                        }
                        write!(f, "{var}")?;
                        if start_param_underline {
                            write!(f, "\x1b[24m")?;
                        }
                        current_path.pop();
                    }
                    write!(f, "> ")?;
                } else if is_vars_target {
                    write!(f, "<> ")?;
                }
                if start_underline {
                    write!(f, "\x1b[24m")?;
                }
                current_path.pop();

                current_path.push(TypePathSegment::PairLeft);
                write_type_with_options(f, names, arg, options, current_path)?;
                current_path.pop();
                then = next_then;
                current_path.push(TypePathSegment::PairRight);
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
            write!(f, "{close}?")?;
        } else {
            write!(f, "{close}!")?;
        }
    } else {
        write!(f, "{close} ")?;
        write_type_with_options(f, names, then, options, current_path)?;
    }

    while current_path.last() == Some(&TypePathSegment::FunctionReturn)
        || current_path.last() == Some(&TypePathSegment::PairRight)
    {
        current_path.pop();
    }

    Ok(())
}

fn write_braced_branches<S: Clone, N: GlobalNameWriter<S>>(
    f: &mut impl Write,
    names: &N,
    prefix: &str,
    branches: &BTreeMap<LocalName, Type<S>>,
    choice: bool,
    options: TypeRenderOptions<'_>,
    current_path: &mut TypePath,
) -> fmt::Result {
    if branches.is_empty() {
        return write!(f, "{prefix} {{}}");
    }

    write!(f, "{prefix} {{")?;

    for (branch, branch_type) in branches {
        let options = options.next_indent();
        options.write_indentation(f)?;

        let label_seg = if choice {
            TypePathSegment::ChoiceBranchLabel(branch.clone())
        } else {
            TypePathSegment::EitherBranchLabel(branch.clone())
        };

        current_path.push(label_seg);
        let is_label_target = options
            .highlight_path
            .map_or(false, |target| target == current_path);
        current_path.pop();

        if is_label_target {
            write!(f, "\x1b[4m.{branch}\x1b[24m")?;
        } else {
            write!(f, ".{branch}")?;
        }

        let seg = if choice {
            TypePathSegment::ChoiceBranch(branch.clone())
        } else {
            TypePathSegment::EitherBranch(branch.clone())
        };
        current_path.push(seg);

        if choice {
            if matches!(branch_type, Type::Function(..)) || matches!(branch_type, Type::Forall(..))
            {
                write_pair_like(
                    f,
                    names,
                    "(",
                    ") =>",
                    branch_type,
                    true,
                    options,
                    current_path,
                )?;
            } else {
                write!(f, " => ")?;
                write_type_with_options(f, names, branch_type, options, current_path)?;
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
            write_type_with_options(f, names, branch_type, options, current_path)?;
        }
        current_path.pop();
        write!(f, ",")?;
    }
    options.write_indentation(f)?;
    write!(f, "}}")
}

fn write_named_type_display<S: Clone, N: GlobalNameWriter<S>>(
    f: &mut impl Write,
    names: &N,
    display_hint: &NamedTypeDisplay<S>,
    options: TypeRenderOptions<'_>,
    current_path: &mut TypePath,
) -> fmt::Result {
    if display_hint.dual {
        write!(f, "dual ")?;
    }
    names.write_global_name(f, &display_hint.name)?;
    write_type_args(f, names, &display_hint.args, options, current_path)
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
