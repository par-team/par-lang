use super::{
    language::{
        Apply, ApplyBranch, ApplyBranches, ArithmeticOperator, Command, CommandBranch,
        CommandBranches, ComparisonOperator, ComparisonStep, Condition, Construct, ConstructBranch,
        ConstructBranches, Expression, GlobalName, Pattern, Process, TemplatePart, TypeConstraint,
        TypeParameter, Unresolved,
    },
    lexer::{
        Comment, CommentKind, Input, Token, TokenKind, lex, lex_with_comments,
        unescape_template_text,
    },
};
use crate::frontend_impl::program::DefinitionBody;
use crate::frontend_impl::{
    language::LocalName,
    program::{
        Declaration, Definition, DocComment, ImportDecl, ImportPath, Module, ModuleDecl,
        SourceFile, TypeDef,
    },
    types::Type,
};
use crate::location::{FileName, Point, Span, Spanning};
use bytes::Bytes;
use core::fmt::Display;
use miette::{SourceOffset, SourceSpan};
use num_bigint::BigInt;
use par_runtime::{
    atom::{Atom, sym},
    primitive::{ParString, Primitive},
    readback::Number,
};
use std::collections::BTreeMap;
use winnow::token::literal;
use winnow::{
    Parser,
    combinator::{alt, cut_err, not, opt, peek, preceded, repeat, separated, terminated, trace},
    error::{
        AddContext, ContextError, ErrMode, ModalError, ParserError, StrContext, StrContextValue,
    },
    stream::{Accumulate, Stream},
};

#[derive(Debug, Clone, Default, PartialEq)]
struct ParseContextError<C = StrContext> {
    context: Vec<(usize, ContextError<C>)>,
}

type Error = ErrMode<ParseContextError>;
impl<I: Stream, C: core::fmt::Debug> ParserError<I> for ParseContextError<C> {
    type Inner = Self;

    fn from_input(input: &I) -> Self {
        Self {
            context: vec![(input.eof_offset(), ContextError::from_input(input))],
        }
    }
    fn into_inner(self) -> winnow::Result<Self::Inner, Self> {
        Ok(self)
    }
    fn append(self, _input: &I, _token_start: &<I as Stream>::Checkpoint) -> Self {
        self
    }
    fn or(mut self, other: Self) -> Self {
        self.context.extend(other.context);
        self
    }
}
impl<I: Stream, C> AddContext<I, C> for ParseContextError<C> {
    fn add_context(
        mut self,
        input: &I,
        token_start: &<I as Stream>::Checkpoint,
        context: C,
    ) -> Self {
        let new_context = |context| {
            (
                input.eof_offset(),
                ContextError::new().add_context(input, token_start, context),
            )
        };
        if self.context.is_empty() {
            self.context.push(new_context(context));
            return self;
        }
        let last = self.context.pop().unwrap();
        if last.0 != input.eof_offset() {
            self.context.push(new_context(context));
            return self;
        }
        let last = (
            last.0.min(input.eof_offset()),
            last.1.add_context(input, token_start, context),
        );
        self.context.push(last);
        self
    }
}

type Result<O, E = ParseContextError> = core::result::Result<O, ErrMode<E>>;

/// Token with additional context of expecting the `token` value
fn t<'i, E>(kind: TokenKind) -> impl Parser<Input<'i>, &'i Token<'i>, E>
where
    E: AddContext<Input<'i>, StrContext> + ParserError<Input<'i>>,
{
    literal(kind)
        .context(StrContext::Expected(StrContextValue::StringLiteral(
            kind.expected(),
        )))
        .map(|t: &[Token]| &t[0])
}

fn list0<'i, P, O>(item: P) -> impl Parser<Input<'i>, Vec<O>, Error> + use<'i, P, O>
where
    P: Parser<Input<'i>, O, Error>,
    Vec<O>: Accumulate<O>,
{
    terminated(
        separated(0.., item, t(TokenKind::Comma)),
        opt(t(TokenKind::Comma)),
    )
}

fn list1<'i, P, O>(item: P) -> impl Parser<Input<'i>, Vec<O>, Error> + use<'i, P, O>
where
    P: Parser<Input<'i>, O, Error>,
    Vec<O>: Accumulate<O>,
{
    terminated(
        separated(1.., item, t(TokenKind::Comma)),
        opt(t(TokenKind::Comma)),
    )
}

fn commit_after<Input, Prefix, Output, Error, PrefixParser, ParseNext>(
    prefix: PrefixParser,
    parser: ParseNext,
) -> impl Parser<Input, (Prefix, Output), Error>
where
    Input: Stream,
    Error: ParserError<Input> + ModalError,
    PrefixParser: Parser<Input, Prefix, Error>,
    ParseNext: Parser<Input, Output, Error>,
{
    trace("commit_after", (prefix, cut_err(parser)))
}

fn lowercase_identifier(input: &mut Input) -> Result<(Span, String)> {
    alt((
        literal(TokenKind::LowercaseIdentifier),
        literal(TokenKind::And),
        literal(TokenKind::Neg),
        literal(TokenKind::Not),
        literal(TokenKind::Or),
    ))
    .context(StrContext::Expected(StrContextValue::CharLiteral('_')))
    .context(StrContext::Expected(StrContextValue::Description(
        "lower-case alphabetic",
    )))
    .map(|token: &[Token]| (token[0].span(), token[0].raw.to_owned()))
    .parse_next(input)
}

fn uppercase_identifier(input: &mut Input) -> Result<(Span, String)> {
    literal(TokenKind::UppercaseIdentifier)
        .context(StrContext::Expected(StrContextValue::Description(
            "upper-case alphabetic",
        )))
        .map(|token: &[Token]| (token[0].span(), token[0].raw.to_owned()))
        .parse_next(input)
}

fn local_name(input: &mut Input) -> Result<LocalName> {
    lowercase_identifier
        .map(|(span, string)| LocalName {
            span,
            string: Atom::from(string),
        })
        .parse_next(input)
}

fn global_name(input: &mut Input) -> Result<GlobalName<Unresolved>> {
    (
        uppercase_identifier,
        opt((t(TokenKind::Dot), uppercase_identifier)),
    )
        .map(|((first_span, first), opt_second)| {
            let (span, module, primary) = match opt_second {
                Some((_, (second_span, second))) => {
                    (first_span.join(second_span), Some(first), second)
                }
                None => (first_span, None, first),
            };
            GlobalName::new(span, Unresolved::Path { qualifier: module }, primary)
        })
        .parse_next(input)
}

fn global_binding_name(input: &mut Input) -> Result<GlobalName<Unresolved>> {
    uppercase_identifier
        .map(|(span, primary)| GlobalName::new(span, Unresolved::Path { qualifier: None }, primary))
        .parse_next(input)
}

fn module_decl(input: &mut Input) -> Result<ModuleDecl> {
    (
        opt(t(TokenKind::Export)),
        commit_after(t(TokenKind::Module), uppercase_identifier),
    )
        .map(|(export_kw, (module_kw, (name_span, name)))| ModuleDecl {
            span: export_kw
                .map(|export_kw| export_kw.span.join(name_span.clone()))
                .unwrap_or_else(|| module_kw.span.join(name_span)),
            exported: export_kw.is_some(),
            doc: None,
            name,
        })
        .parse_next(input)
}

fn any_identifier(input: &mut Input) -> Result<(Span, String)> {
    alt((lowercase_identifier, uppercase_identifier)).parse_next(input)
}

fn import_path(input: &mut Input) -> Result<(Span, ImportPath)> {
    alt((
        (
            preceded(
                t(TokenKind::At),
                lowercase_identifier.map(|(_, string)| string),
            ),
            t(TokenKind::Slash),
            any_identifier,
            repeat(0.., preceded(t(TokenKind::Slash), any_identifier)),
        )
            .map(
                |(dependency, _slash, first, mut rest): (
                    String,
                    &Token,
                    (Span, String),
                    Vec<(Span, String)>,
                )| {
                    let mut directories = Vec::new();
                    let mut end_span = first.0.clone();
                    let mut module = first.1;
                    for (span, segment) in rest.drain(..) {
                        directories.push(module);
                        module = segment;
                        end_span = span;
                    }
                    (
                        first.0.join(end_span),
                        ImportPath {
                            dependency: Some(dependency),
                            directories,
                            module,
                        },
                    )
                },
            ),
        (
            any_identifier,
            repeat(0.., preceded(t(TokenKind::Slash), any_identifier)),
        )
            .map(|(first, mut rest): ((Span, String), Vec<(Span, String)>)| {
                let mut directories = Vec::new();
                let mut end_span = first.0.clone();
                let mut module = first.1;
                for (span, segment) in rest.drain(..) {
                    directories.push(module);
                    module = segment;
                    end_span = span;
                }
                (
                    first.0.join(end_span),
                    ImportPath {
                        dependency: None,
                        directories,
                        module,
                    },
                )
            }),
    ))
    .parse_next(input)
}

fn import_entry(input: &mut Input) -> Result<ImportDecl> {
    (
        import_path,
        opt(preceded(
            t(TokenKind::As),
            uppercase_identifier.map(|(_, string)| string),
        )),
    )
        .map(
            |((path_span, path), alias): ((Span, ImportPath), Option<String>)| {
                let span = path_span;
                ImportDecl { span, path, alias }
            },
        )
        .parse_next(input)
}

fn import_statement(input: &mut Input) -> Result<Vec<ImportDecl>> {
    commit_after(
        t(TokenKind::Import),
        alt((
            commit_after(
                t(TokenKind::LCurly),
                (
                    repeat(0.., terminated(import_entry, opt(t(TokenKind::Comma)))),
                    t(TokenKind::RCurly),
                ),
            )
            .map(|(_lcurly, (items, _rcurly))| items),
            import_entry.map(|item| vec![item]),
        )),
    )
    .map(|(_import, entries)| entries)
    .parse_next(input)
}

#[derive(Clone)]
enum ModuleItem<Expr> {
    TypeDef(TypeDef<Unresolved>),
    Declaration(Declaration<Unresolved>),
    Definition(Definition<Expr, Unresolved>, Option<Type<Unresolved>>),
}

fn mark_exported_type_def(
    mut type_def: TypeDef<Unresolved>,
    export_span: Option<Span>,
) -> TypeDef<Unresolved> {
    type_def.exported = true;
    if let Some(export_span) = export_span {
        type_def.span = export_span.join(type_def.span);
    }
    type_def
}

fn mark_exported_declaration(
    mut declaration: Declaration<Unresolved>,
    export_span: Option<Span>,
) -> Declaration<Unresolved> {
    declaration.exported = true;
    if let Some(export_span) = export_span {
        declaration.span = export_span.join(declaration.span);
    }
    declaration
}

fn export_block_item(input: &mut Input) -> Result<ModuleItem<Expression<Unresolved>>> {
    alt((
        type_def.map(|type_def| ModuleItem::TypeDef(mark_exported_type_def(type_def, None))),
        declaration.map(|declaration| {
            ModuleItem::Declaration(mark_exported_declaration(declaration, None))
        }),
    ))
    .context(StrContext::Label("exported item"))
    .parse_next(input)
}

fn export_statement(input: &mut Input) -> Result<Vec<ModuleItem<Expression<Unresolved>>>> {
    enum ExportStatement {
        Block(Vec<ModuleItem<Expression<Unresolved>>>),
        TypeDef(TypeDef<Unresolved>),
        Declaration(Declaration<Unresolved>),
    }

    commit_after(
        t(TokenKind::Export),
        alt((
            commit_after(
                t(TokenKind::LCurly),
                (repeat(0.., export_block_item), t(TokenKind::RCurly)),
            )
            .map(|(_lcurly, (items, _rcurly))| ExportStatement::Block(items)),
            type_def.map(ExportStatement::TypeDef),
            declaration.map(ExportStatement::Declaration),
        )),
    )
    .map(|(export_kw, statement)| match statement {
        ExportStatement::Block(items) => items,
        ExportStatement::TypeDef(type_def) => vec![ModuleItem::TypeDef(mark_exported_type_def(
            type_def,
            Some(export_kw.span.clone()),
        ))],
        ExportStatement::Declaration(declaration) => {
            vec![ModuleItem::Declaration(mark_exported_declaration(
                declaration,
                Some(export_kw.span.clone()),
            ))]
        }
    })
    .context(StrContext::Label("export statement"))
    .parse_next(input)
}

fn module_item_statement(input: &mut Input) -> Result<Vec<ModuleItem<Expression<Unresolved>>>> {
    alt((
        export_statement,
        type_def.map(|type_def| vec![ModuleItem::TypeDef(type_def)]),
        declaration.map(|declaration| vec![ModuleItem::Declaration(declaration)]),
        definition.map(|(definition, typ)| vec![ModuleItem::Definition(definition, typ)]),
    ))
    .parse_next(input)
}

struct ProgramParseError {
    offset: usize,
    error: ParseContextError,
}
impl ProgramParseError {
    fn offset(&self) -> usize {
        self.offset
    }
    fn inner(&self) -> &ParseContextError {
        &self.error
    }
}

fn source_file(
    mut input: Input,
) -> std::result::Result<SourceFile<Expression<Unresolved>>, ProgramParseError> {
    let parser = repeat(
        0..,
        module_item_statement.context(StrContext::Label("item")),
    )
    .fold(Module::default, |mut acc, item| {
        for item in item {
            match item {
                ModuleItem::TypeDef(type_def) => {
                    acc.type_defs.push(type_def);
                }
                ModuleItem::Declaration(dec) => {
                    acc.declarations.push(dec);
                }
                ModuleItem::Definition(Definition { span, name, body }, annotation) => {
                    if let Some(typ) = annotation {
                        acc.declarations.push(Declaration {
                            span: span.clone(),
                            exported: false,
                            doc: None,
                            name: name.clone(),
                            typ,
                        });
                    }
                    acc.definitions.push(Definition { span, name, body });
                }
            }
        }
        acc
    });

    let start = input.checkpoint();
    (
        (
            opt(module_decl),
            repeat(0.., import_statement).map(|groups: Vec<Vec<ImportDecl>>| {
                groups.into_iter().flatten().collect::<Vec<ImportDecl>>()
            }),
            parser,
        ),
        winnow::combinator::eof
            .context(StrContext::Expected(StrContextValue::StringLiteral(
                "module",
            )))
            .context(StrContext::Expected(StrContextValue::StringLiteral(
                "export",
            )))
            .context(StrContext::Expected(StrContextValue::StringLiteral(
                "import",
            )))
            .context(StrContext::Expected(StrContextValue::StringLiteral("type")))
            .context(StrContext::Expected(StrContextValue::StringLiteral("dec")))
            .context(StrContext::Expected(StrContextValue::StringLiteral("def")))
            .context(StrContext::Expected(StrContextValue::Description(
                "end of file",
            ))),
    )
        .parse_next(&mut input)
        .map(|((module_decl, imports, body), _eof)| SourceFile {
            module_decl,
            imports,
            body,
        })
        .map_err(|e: Error| {
            let e = e.into_inner().unwrap_or_else(|_err| {
                panic!("complete parsers should not report `ErrMode::Incomplete(_)`")
            });

            ProgramParseError {
                offset: winnow::stream::Offset::offset_from(&input, &start),
                error: ParserError::append(e, &input, &start),
            }
        })
}

#[derive(Debug, Clone, miette::Diagnostic)]
#[diagnostic(severity(Error))]
pub struct SyntaxError {
    #[label]
    source_span: SourceSpan,
    // Generate these with the miette! macro.
    // #[related]
    // related: Arc<[miette::ErrReport]>,
    #[help]
    help: String,

    span: Span,
}

impl Display for SyntaxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Syntax error")
    }
}
impl core::error::Error for SyntaxError {}

impl Spanning for SyntaxError {
    fn span(&self) -> Span {
        self.span.clone()
    }
}

pub fn set_miette_hook() {
    _ = miette::set_hook(Box::new(|_| {
        Box::new(
            miette::MietteHandlerOpts::new()
                .terminal_links(true)
                .unicode(true)
                .color(false)
                // .context_lines(1)
                // .with_cause_chain()
                .build(),
        )
    }));
}

pub(crate) fn parse_module(
    input: &str,
    file: FileName,
) -> std::result::Result<Module<Expression<Unresolved>, Unresolved>, SyntaxError> {
    parse_source_file(input, file).map(|source_file| source_file.body)
}

pub(crate) fn parse_source_file(
    input: &str,
    file: FileName,
) -> std::result::Result<SourceFile<Expression<Unresolved>>, SyntaxError> {
    let lexed = lex_with_comments(&input, &file);
    let comments = lexed.comments;
    let tokens = lexed.tokens;
    let e = match source_file(Input::new(&tokens)) {
        Ok(mut x) => {
            attach_doc_comments(input, &comments, &mut x);
            return Ok(x);
        }
        Err(e) => e,
    };
    // Empty input doesn't error so this won't panic.
    let error_tok = tokens
        .get(e.offset())
        .unwrap_or(tokens.last().unwrap())
        .clone();
    let error_tok_span = error_tok.span();
    Err(SyntaxError {
        span: error_tok_span.clone(),
        source_span: match error_tok_span {
            Span::None => SourceSpan::new(SourceOffset::from(0), input.len()),
            span @ Span::At { start, .. } => SourceSpan::new(
                SourceOffset::from(start.offset as usize),
                if span.len() == 1 {
                    // miette unicode format for 1 length span is a hard-to-notice line, so don't set length to 1.
                    0
                } else {
                    span.len() as usize
                },
            ),
        },
        help: e
            .inner()
            .context
            .iter()
            .map(|x| x.1.to_string().chars().chain(['\n']).collect::<String>())
            .collect::<String>(),
    })
}

#[derive(Clone, Copy)]
enum DocTarget {
    ModuleDecl,
    TypeDef(usize),
    Declaration(usize),
}

struct TopLevelBarrier {
    start: Point,
    end: Point,
    target: Option<DocTarget>,
}

fn attach_doc_comments(
    source: &str,
    comments: &[Comment<'_>],
    source_file: &mut SourceFile<Expression<Unresolved>>,
) {
    let mut barriers = Vec::new();

    if let Some(module_decl) = source_file.module_decl.as_ref() {
        if let Some((start, end)) = module_decl.span.points() {
            barriers.push(TopLevelBarrier {
                start,
                end,
                target: Some(DocTarget::ModuleDecl),
            });
        }
    }

    for import in &source_file.imports {
        if let Some((start, end)) = import.span.points() {
            barriers.push(TopLevelBarrier {
                start,
                end,
                target: None,
            });
        }
    }

    for (index, type_def) in source_file.body.type_defs.iter().enumerate() {
        if let Some((start, end)) = type_def.span.points() {
            barriers.push(TopLevelBarrier {
                start,
                end,
                target: Some(DocTarget::TypeDef(index)),
            });
        }
    }

    for (index, declaration) in source_file.body.declarations.iter().enumerate() {
        if !is_explicit_declaration(source, declaration) {
            continue;
        }

        if let Some((start, end)) = declaration.span.points() {
            barriers.push(TopLevelBarrier {
                start,
                end,
                target: Some(DocTarget::Declaration(index)),
            });
        }
    }

    for definition in &source_file.body.definitions {
        if let Some((start, end)) = definition.span.points() {
            barriers.push(TopLevelBarrier {
                start,
                end,
                target: None,
            });
        }
    }

    barriers.sort_by_key(|barrier| barrier.start.offset);

    let mut previous_barrier_end = 0;
    let mut comment_start = 0;
    for barrier in barriers {
        while comment_start < comments.len()
            && comments[comment_start]
                .span
                .end()
                .is_some_and(|end| end.offset <= previous_barrier_end)
        {
            comment_start += 1;
        }

        let mut comment_end = comment_start;
        while comment_end < comments.len()
            && comments[comment_end]
                .span
                .end()
                .is_some_and(|end| end.offset <= barrier.start.offset)
        {
            comment_end += 1;
        }

        if let Some(doc) =
            doc_comment_before_item(&comments[comment_start..comment_end], barrier.start)
        {
            match barrier.target {
                Some(DocTarget::ModuleDecl) => {
                    if let Some(module_decl) = source_file.module_decl.as_mut() {
                        module_decl.doc = Some(doc);
                    }
                }
                Some(DocTarget::TypeDef(index)) => {
                    source_file.body.type_defs[index].doc = Some(doc)
                }
                Some(DocTarget::Declaration(index)) => {
                    source_file.body.declarations[index].doc = Some(doc)
                }
                None => {}
            }
        }

        previous_barrier_end = barrier.end.offset;
        comment_start = comment_end;
    }
}

fn is_explicit_declaration(source: &str, declaration: &Declaration<Unresolved>) -> bool {
    if declaration.exported {
        return true;
    }
    declaration
        .span
        .start()
        .and_then(|start| source.get(start.offset as usize..))
        .is_some_and(|rest| rest.starts_with("dec"))
}

fn doc_comment_before_item(comments: &[Comment<'_>], item_start: Point) -> Option<DocComment> {
    let last = comments.last()?;
    let last_end = last.span.end()?;
    if has_blank_line_between(last_end.row, item_start.row) {
        return None;
    }

    match last.kind {
        CommentKind::Block => {
            if let Some(previous) = comments.iter().rev().nth(1) {
                let previous_end = previous.span.end()?;
                let last_start = last.span.start()?;
                if !has_blank_line_between(previous_end.row, last_start.row) {
                    return None;
                }
            }

            Some(DocComment {
                span: last.span.clone(),
                markdown: normalize_doc_newlines(strip_block_comment(last.raw)).into(),
            })
        }
        CommentKind::Line => {
            let mut run_start = comments.len() - 1;
            while run_start > 0 {
                let previous = &comments[run_start - 1];
                let current = &comments[run_start];
                let previous_end = previous.span.end()?;
                let current_start = current.span.start()?;
                if previous.kind == CommentKind::Line
                    && !has_blank_line_between(previous_end.row, current_start.row)
                {
                    run_start -= 1;
                } else {
                    break;
                }
            }

            let run = &comments[run_start..];
            let markdown = run
                .iter()
                .map(|comment| strip_line_comment(comment.raw))
                .collect::<Vec<_>>()
                .join("\n");

            Some(DocComment {
                span: run
                    .first()
                    .map(|first| first.span.join(run.last().unwrap().span.clone()))
                    .unwrap_or_else(|| last.span.clone()),
                markdown: markdown.into(),
            })
        }
    }
}

fn has_blank_line_between(previous_end_row: u32, next_start_row: u32) -> bool {
    next_start_row > previous_end_row + 1
}

fn strip_line_comment(raw: &str) -> String {
    raw.strip_prefix("//")
        .unwrap_or(raw)
        .strip_prefix(' ')
        .unwrap_or_else(|| raw.strip_prefix("//").unwrap_or(raw))
        .trim_end_matches('\r')
        .to_string()
}

fn strip_block_comment(raw: &str) -> &str {
    raw.strip_prefix("/*")
        .and_then(|raw| raw.strip_suffix("*/"))
        .unwrap_or(raw)
}

fn normalize_doc_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

pub fn parse_bytes(input: &str, file: &FileName) -> Option<Vec<u8>> {
    (literal_bytes_inner, winnow::combinator::eof)
        .parse_next(&mut Input::new(&lex(input, file)))
        .map(|(b, _)| b)
        .ok()
}

fn type_def(input: &mut Input) -> Result<TypeDef<Unresolved>> {
    commit_after(
        t(TokenKind::Type),
        (global_binding_name, type_params, t(TokenKind::Eq), typ),
    )
    .map(|(pre, (name, type_params, _, typ))| TypeDef {
        span: pre.span.join(typ.span()),
        exported: false,
        doc: None,
        name,
        params: type_params.map_or_else(Vec::new, |(_, params)| params),
        typ,
    })
    .context(StrContext::Label("type definition"))
    .parse_next(input)
}

fn declaration(input: &mut Input) -> Result<Declaration<Unresolved>> {
    commit_after(
        t(TokenKind::Dec),
        (global_binding_name, t(TokenKind::Colon), typ),
    )
    .map(|(pre, (name, _, typ))| Declaration {
        span: pre.span.join(typ.span()),
        exported: false,
        doc: None,
        name,
        typ,
    })
    .context(StrContext::Label("declaration"))
    .parse_next(input)
}

fn definition(
    input: &mut Input,
) -> Result<(
    Definition<Expression<Unresolved>, Unresolved>,
    Option<Type<Unresolved>>,
)> {
    commit_after(
        t(TokenKind::Def),
        (
            global_binding_name,
            annotation,
            t(TokenKind::Eq),
            alt((
                t(TokenKind::External).map(|t| DefinitionBody::External(t.span.clone())),
                expression.map(|expr| DefinitionBody::Par(expr)),
            )),
        ),
    )
    .map(|(pre, (name, annotation, _, body))| {
        (
            Definition {
                span: pre.span.join(body.span()),
                name,
                body,
            },
            annotation,
        )
    })
    .context(StrContext::Label("definition"))
    .parse_next(input)
}

fn branches_body<'i, P, O>(
    branch: P,
) -> impl Parser<Input<'i>, (Span, BTreeMap<LocalName, O>, Option<Box<O>>), Error> + use<'i, P, O>
where
    P: Clone + Parser<Input<'i>, O, Error>,
{
    commit_after(
        t(TokenKind::LCurly),
        (
            repeat(
                0..,
                (
                    t(TokenKind::Dot),
                    local_name,
                    cut_err(branch.clone()),
                    opt(t(TokenKind::Comma)),
                ),
            )
            .fold(
                || BTreeMap::new(),
                |mut branches, (_, name, branch, _)| {
                    branches.insert(name, branch);
                    branches
                },
            ),
            opt((
                t(TokenKind::Else),
                cut_err(branch),
                opt(t(TokenKind::Comma)),
            )),
            t(TokenKind::RCurly),
        ),
    )
    .map(|(open, (branches, else_branch, close))| {
        (
            open.span.join(close.span()),
            branches,
            else_branch.map(|(_, b, _)| Box::new(b)),
        )
    })
    .context(StrContext::Label("either/choice branches"))
}

fn branches_without_else_body<'i, P, O>(
    branch: P,
) -> impl Parser<Input<'i>, (Span, BTreeMap<LocalName, O>), Error> + use<'i, P, O>
where
    P: Clone + Parser<Input<'i>, O, Error>,
{
    commit_after(
        t(TokenKind::LCurly),
        (
            repeat(
                0..,
                (
                    t(TokenKind::Dot),
                    local_name,
                    cut_err(branch.clone()),
                    opt(t(TokenKind::Comma)),
                ),
            )
            .fold(
                || BTreeMap::new(),
                |mut branches, (_, name, branch, _)| {
                    branches.insert(name, branch);
                    branches
                },
            ),
            t(TokenKind::RCurly),
        ),
    )
    .map(|(open, (branches, close))| (open.span.join(close.span()), branches))
    .context(StrContext::Label("either/choice branches"))
}

fn typ(input: &mut Input) -> Result<Type<Unresolved>> {
    alt((
        typ_var,
        typ_name,
        typ_box,
        typ_chan,
        typ_either,
        typ_choice,
        typ_break,
        typ_continue,
        typ_recursive,
        typ_iterative,
        typ_self,
        typ_send,
        typ_receive,
        typ_generic,
    ))
    .context(StrContext::Label("type"))
    .parse_next(input)
}

fn typ_var(input: &mut Input) -> Result<Type<Unresolved>> {
    trace(
        "typ_var",
        local_name.map(|name| Type::Var(name.span(), name)),
    )
    .parse_next(input)
}

fn typ_name(input: &mut Input) -> Result<Type<Unresolved>> {
    trace(
        "typ_name",
        (global_name, type_args).map(|(name, type_args)| match type_args {
            Some((type_args_span, type_args)) => {
                Type::Name(name.span.join(type_args_span), name, type_args)
            }
            None => Type::Name(name.span(), name, vec![]),
        }),
    )
    .parse_next(input)
}

fn typ_box(input: &mut Input) -> Result<Type<Unresolved>> {
    commit_after(
        t(TokenKind::Box),
        typ.context(StrContext::Label("box type")),
    )
    .map(|(pre, typ)| Type::Box(pre.span.join(typ.span()), Box::new(typ)))
    .parse_next(input)
}

fn typ_chan(input: &mut Input) -> Result<Type<Unresolved>> {
    commit_after(
        t(TokenKind::Dual),
        typ.context(StrContext::Label("dual type")),
    )
    .map(|(pre, typ)| typ.dual(pre.span()))
    .parse_next(input)
}

fn typ_send(input: &mut Input) -> Result<Type<Unresolved>> {
    commit_after(
        t(TokenKind::LParen),
        (list1(type_prefix_item), t(TokenKind::RParen), typ),
    )
    .map(|(open, (items, close, then))| {
        let span = open.span.join(close.span());
        fold_type_prefix(
            items,
            then,
            |name, then| Type::Exists(span.clone(), name, Box::new(then)),
            |arg, then| Type::Pair(span.clone(), Box::new(arg), Box::new(then), vec![]),
        )
    })
    .parse_next(input)
}

fn typ_receive(input: &mut Input) -> Result<Type<Unresolved>> {
    commit_after(
        t(TokenKind::LBrack),
        (list1(type_prefix_item), t(TokenKind::RBrack), typ),
    )
    .map(|(open, (items, close, then))| {
        let span = open.span.join(close.span());
        fold_type_prefix(
            items,
            then,
            |name, then| Type::Forall(span.clone(), name, Box::new(then)),
            |arg, then| Type::Function(span.clone(), Box::new(arg), Box::new(then), vec![]),
        )
    })
    .parse_next(input)
}

enum SendOrReceive {
    Send,
    Receive,
}

enum TypePrefixItem {
    Explicit(TypeParameter),
    Value(Type<Unresolved>),
}

enum PatternPrefixItem {
    Explicit(TypeParameter),
    Value(Pattern<Unresolved>),
}

enum SendPrefixItem {
    Explicit(Type<Unresolved>),
    Value(Expression<Unresolved>),
}

fn fold_type_prefix<T>(
    items: Vec<TypePrefixItem>,
    rest: T,
    mut explicit: impl FnMut(TypeParameter, T) -> T,
    mut value: impl FnMut(Type<Unresolved>, T) -> T,
) -> T {
    items.into_iter().rfold(rest, |rest, item| match item {
        TypePrefixItem::Explicit(name) => explicit(name, rest),
        TypePrefixItem::Value(typ) => value(typ, rest),
    })
}

fn fold_pattern_prefix<T>(
    items: Vec<PatternPrefixItem>,
    rest: T,
    mut explicit: impl FnMut(TypeParameter, T) -> T,
    mut value: impl FnMut(Pattern<Unresolved>, T) -> T,
) -> T {
    items.into_iter().rfold(rest, |rest, item| match item {
        PatternPrefixItem::Explicit(name) => explicit(name, rest),
        PatternPrefixItem::Value(pattern) => value(pattern, rest),
    })
}

fn fold_send_prefix<T>(
    items: Vec<SendPrefixItem>,
    rest: T,
    mut explicit: impl FnMut(Type<Unresolved>, T) -> T,
    mut value: impl FnMut(Expression<Unresolved>, T) -> T,
) -> T {
    items.into_iter().rfold(rest, |rest, item| match item {
        SendPrefixItem::Explicit(typ) => explicit(typ, rest),
        SendPrefixItem::Value(expression) => value(expression, rest),
    })
}

fn type_constraint(input: &mut Input) -> Result<TypeConstraint> {
    let checkpoint = input.checkpoint();
    if t::<Error>(TokenKind::Box).parse_next(input).is_ok() {
        return Ok(TypeConstraint::Box);
    }
    input.reset(&checkpoint);

    let name = local_name
        .context(StrContext::Label("type constraint"))
        .parse_next(input)?;
    match name.string {
        sym::data => Ok(TypeConstraint::Data),
        sym::number => Ok(TypeConstraint::Number),
        sym::signed => Ok(TypeConstraint::Signed),
        _ => Err(ErrMode::Backtrack(ParseContextError::from_input(input))),
    }
}

fn type_parameter(input: &mut Input) -> Result<TypeParameter> {
    (
        local_name,
        opt(commit_after(t(TokenKind::Colon), type_constraint)),
    )
        .map(|(name, constraint)| TypeParameter {
            name,
            constraint: constraint.map(|(_, c)| c).unwrap_or(TypeConstraint::Any),
        })
        .parse_next(input)
}

fn unconstrained_type_parameter(input: &mut Input) -> Result<TypeParameter> {
    local_name.map(TypeParameter::any).parse_next(input)
}

fn explicit_type_parameter(input: &mut Input) -> Result<TypeParameter> {
    commit_after(t(TokenKind::Type), type_parameter)
        .map(|(_, parameter)| parameter)
        .parse_next(input)
}

fn type_prefix_item(input: &mut Input) -> Result<TypePrefixItem> {
    alt((
        explicit_type_parameter.map(TypePrefixItem::Explicit),
        typ.map(TypePrefixItem::Value),
    ))
    .parse_next(input)
}

fn pattern_prefix_item(input: &mut Input) -> Result<PatternPrefixItem> {
    alt((
        explicit_type_parameter.map(PatternPrefixItem::Explicit),
        pattern.map(PatternPrefixItem::Value),
    ))
    .parse_next(input)
}

fn send_prefix_item(close: TokenKind, input: &mut Input) -> Result<SendPrefixItem> {
    let checkpoint = input.checkpoint();
    if t::<Error>(TokenKind::Type).parse_next(input).is_ok() {
        if let Ok(typ) = typ.parse_next(input) {
            if peek(alt((t::<Error>(TokenKind::Comma), t::<Error>(close))))
                .parse_next(input)
                .is_ok()
            {
                return Ok(SendPrefixItem::Explicit(typ));
            }
        }
        input.reset(&checkpoint);
    }

    expression.map(SendPrefixItem::Value).parse_next(input)
}

fn typ_simple_send(
    input: &mut Input,
) -> Result<(SendOrReceive, Type<Unresolved>, Type<Unresolved>, Span)> {
    commit_after(t(TokenKind::LParen), (typ, t(TokenKind::RParen), typ))
        .map(|(open, (arg, close, then))| {
            let span = open.span.join(close.span());
            (SendOrReceive::Send, arg, then, span)
        })
        .parse_next(input)
}

fn typ_simple_receive(
    input: &mut Input,
) -> Result<(SendOrReceive, Type<Unresolved>, Type<Unresolved>, Span)> {
    commit_after(t(TokenKind::LBrack), (typ, t(TokenKind::RBrack), typ))
        .map(|(open, (arg, close, then))| {
            let span = open.span.join(close.span());
            (SendOrReceive::Receive, arg, then, span)
        })
        .parse_next(input)
}

fn typ_generic(input: &mut Input) -> Result<Type<Unresolved>> {
    commit_after(
        t(TokenKind::Lt),
        (
            list1(type_parameter),
            t(TokenKind::Gt),
            alt((typ_simple_send, typ_simple_receive)),
        ),
    )
    .map(
        |(vars_open, (vars, _vars_close, (snd_or_recv, arg, then, span)))| {
            let span = vars_open.span.join(span);
            match snd_or_recv {
                SendOrReceive::Send => {
                    Type::Pair(span.clone(), Box::new(arg), Box::new(then), vars)
                }
                SendOrReceive::Receive => {
                    Type::Function(span.clone(), Box::new(arg), Box::new(then), vars)
                }
            }
        },
    )
    .parse_next(input)
}

fn typ_either(input: &mut Input) -> Result<Type<Unresolved>> {
    commit_after(t(TokenKind::Either), branches_without_else_body(typ))
        .map(|(pre, (branches_span, branches))| {
            Type::Either(pre.span.join(branches_span), branches)
        })
        .parse_next(input)
}

fn typ_choice(input: &mut Input) -> Result<Type<Unresolved>> {
    commit_after(t(TokenKind::Choice), branches_without_else_body(typ_branch))
        .map(|(pre, (branches_span, branches))| {
            Type::Choice(pre.span.join(branches_span), branches)
        })
        .parse_next(input)
}

fn typ_break(input: &mut Input) -> Result<Type<Unresolved>> {
    t(TokenKind::Bang)
        .map(|token| Type::Break(token.span()))
        .parse_next(input)
}

fn typ_continue(input: &mut Input) -> Result<Type<Unresolved>> {
    t(TokenKind::Quest)
        .map(|token| Type::Continue(token.span()))
        .parse_next(input)
}

fn typ_recursive(input: &mut Input) -> Result<Type<Unresolved>> {
    commit_after(t(TokenKind::Recursive), (label, typ))
        .map(|(pre, (label, typ))| Type::Recursive {
            span: pre.span.join(typ.span()),
            asc: Default::default(),
            label,
            body: Box::new(typ),
            display_hint: Default::default(),
        })
        .parse_next(input)
}

fn typ_iterative(input: &mut Input) -> Result<Type<Unresolved>> {
    commit_after(
        t(TokenKind::Iterative),
        (label, typ).context(StrContext::Label("iterative type body")),
    )
    .map(|(pre, (label, typ))| Type::Iterative {
        span: pre.span.join(typ.span()),
        asc: Default::default(),
        label,
        body: Box::new(typ),
        display_hint: Default::default(),
    })
    .parse_next(input)
}

fn typ_self(input: &mut Input) -> Result<Type<Unresolved>> {
    commit_after(
        t(TokenKind::Self_),
        label.context(StrContext::Label("self type loop label")),
    )
    .map(|(token, label)| {
        Type::Self_(
            match &label {
                Some(label) => token.span.join(label.span()),
                None => token.span(),
            },
            label,
        )
    })
    .parse_next(input)
}

fn type_params(input: &mut Input) -> Result<Option<(Span, Vec<TypeParameter>)>> {
    opt(commit_after(
        t(TokenKind::Lt),
        (list1(unconstrained_type_parameter), t(TokenKind::Gt)),
    ))
    .map(|opt| opt.map(|(open, (names, close))| (open.span.join(close.span()), names)))
    .parse_next(input)
}

fn type_args<'s>(input: &mut Input) -> Result<Option<(Span, Vec<Type<Unresolved>>)>> {
    opt(commit_after(
        t(TokenKind::Lt),
        (list1(typ), t(TokenKind::Gt)),
    ))
    .map(|opt| opt.map(|(open, (types, close))| (open.span.join(close.span()), types)))
    .parse_next(input)
}

fn typ_branch(input: &mut Input) -> Result<Type<Unresolved>> {
    alt((typ_branch_then, typ_branch_receive)).parse_next(input)
}

fn typ_branch_then(input: &mut Input) -> Result<Type<Unresolved>> {
    commit_after(t(TokenKind::FatArrow), typ)
        .map(|(_, typ)| typ)
        .parse_next(input)
}

fn typ_branch_receive(input: &mut Input) -> Result<Type<Unresolved>> {
    commit_after(
        t(TokenKind::LParen),
        (list1(type_prefix_item), t(TokenKind::RParen), typ_branch),
    )
    .map(|(open, (items, close, then))| {
        let span = open.span.join(close.span());
        fold_type_prefix(
            items,
            then,
            |name, then| Type::Forall(span.clone(), name, Box::new(then)),
            |arg, then| Type::Function(span.clone(), Box::new(arg), Box::new(then), vec![]),
        )
    })
    .parse_next(input)
}

fn annotation(input: &mut Input) -> Result<Option<Type<Unresolved>>> {
    opt(commit_after(t(TokenKind::Colon), typ))
        .map(|opt| opt.map(|(_, typ)| typ))
        .parse_next(input)
}

// pattern           = { pattern_name | pattern_receive | pattern_continue | pattern_recv_type }
fn pattern(input: &mut Input) -> Result<Pattern<Unresolved>> {
    alt((
        pattern_name,
        pattern_receive,
        pattern_generic_receive,
        pattern_continue,
        pattern_default,
        pattern_try,
    ))
    .parse_next(input)
}

fn pattern_name(input: &mut Input) -> Result<Pattern<Unresolved>> {
    (local_name, annotation)
        .map(|(name, annotation)| {
            Pattern::Name(
                match &annotation {
                    Some(typ) => name.span.join(typ.span()),
                    None => name.span(),
                },
                name,
                annotation,
            )
        })
        .parse_next(input)
}

fn pattern_receive(input: &mut Input) -> Result<Pattern<Unresolved>> {
    commit_after(
        t(TokenKind::LParen),
        (list1(pattern_prefix_item), t(TokenKind::RParen), pattern),
    )
    .map(|(open, (items, _close, rest))| {
        let span = open.span.join(rest.span());
        fold_pattern_prefix(
            items,
            rest,
            |name, rest| Pattern::ReceiveType(span.clone(), name, Box::new(rest)),
            |arg, rest| Pattern::Receive(span.clone(), Box::new(arg), Box::new(rest), vec![]),
        )
    })
    .parse_next(input)
}

fn pattern_generic_receive(input: &mut Input) -> Result<Pattern<Unresolved>> {
    commit_after(
        t(TokenKind::Lt),
        (
            list1(type_parameter),
            t(TokenKind::Gt),
            t(TokenKind::LParen),
            pattern,
            t(TokenKind::RParen),
            pattern,
        ),
    )
    .map(
        |(vars_open, (vars, _vars_close, _pattern_open, arg, _pattern_close, rest))| {
            let span = vars_open.span.join(rest.span());
            Pattern::Receive(span.clone(), Box::new(arg), Box::new(rest), vars)
        },
    )
    .parse_next(input)
}

fn pattern_continue(input: &mut Input) -> Result<Pattern<Unresolved>> {
    t(TokenKind::Bang)
        .map(|token| Pattern::Continue(token.span()))
        .parse_next(input)
}

fn pattern_try(input: &mut Input) -> Result<Pattern<Unresolved>> {
    commit_after(t(TokenKind::Try), (label, pattern))
        .map(|(pre, (label, rest))| Pattern::Try(pre.span.join(rest.span()), label, Box::new(rest)))
        .parse_next(input)
}

fn pattern_default(input: &mut Input) -> Result<Pattern<Unresolved>> {
    commit_after(
        (t(TokenKind::Default), t(TokenKind::LParen)),
        (expression, t(TokenKind::RParen), pattern),
    )
    .map(|((pre, _), (expr, _close, rest))| {
        Pattern::Default(pre.span.join(rest.span()), Box::new(expr), Box::new(rest))
    })
    .parse_next(input)
}

fn condition(input: &mut Input) -> Result<Condition<Unresolved>> {
    infix_or(input).map(expression_to_condition)
}

fn condition_payload_pattern(input: &mut Input) -> Result<Pattern<Unresolved>> {
    alt((pattern_payload_receive, pattern_continue, pattern_name)).parse_next(input)
}

fn pattern_payload_receive(input: &mut Input) -> Result<Pattern<Unresolved>> {
    commit_after(
        t(TokenKind::LParen),
        (list1(pattern_prefix_item), t(TokenKind::RParen), pattern),
    )
    .map(|(open, (items, _close, rest))| {
        let span = open.span.join(rest.span());
        fold_pattern_prefix(
            items,
            rest,
            |name, rest| Pattern::ReceiveType(span.clone(), name, Box::new(rest)),
            |arg, rest| Pattern::Receive(span.clone(), Box::new(arg), Box::new(rest), vec![]),
        )
    })
    .parse_next(input)
}

fn expression_to_condition(expr: Expression<Unresolved>) -> Condition<Unresolved> {
    match expr {
        Expression::Condition(_, cond) => *cond,
        other => Condition::Bool(other.span(), Box::new(other)),
    }
}

fn wrap_condition_expression(condition: Condition<Unresolved>) -> Expression<Unresolved> {
    let span = condition.span();
    Expression::Condition(span, Box::new(condition))
}

fn fold_condition_expression(
    left: Expression<Unresolved>,
    right: Expression<Unresolved>,
    build: impl FnOnce(Span, Condition<Unresolved>, Condition<Unresolved>) -> Condition<Unresolved>,
) -> Expression<Unresolved> {
    let left_span = left.span();
    let right_span = right.span();
    wrap_condition_expression(build(
        left_span.join(right_span.clone()),
        expression_to_condition(left),
        expression_to_condition(right),
    ))
}

fn comparison_operator(input: &mut Input) -> Result<(Span, ComparisonOperator)> {
    alt((
        t(TokenKind::LtEq).map(|token| (token.span(), ComparisonOperator::LessOrEqual)),
        t(TokenKind::GtEq).map(|token| (token.span(), ComparisonOperator::GreaterOrEqual)),
        t(TokenKind::EqEq).map(|token| (token.span(), ComparisonOperator::Equal)),
        t(TokenKind::BangEq).map(|token| (token.span(), ComparisonOperator::NotEqual)),
        t(TokenKind::Lt).map(|token| (token.span(), ComparisonOperator::Less)),
        t(TokenKind::Gt).map(|token| (token.span(), ComparisonOperator::Greater)),
    ))
    .parse_next(input)
}

fn additive_operator(input: &mut Input) -> Result<(Span, ArithmeticOperator)> {
    alt((
        t(TokenKind::Plus).map(|token| (token.span(), ArithmeticOperator::Add)),
        t(TokenKind::Minus).map(|token| (token.span(), ArithmeticOperator::Sub)),
    ))
    .parse_next(input)
}

fn multiplicative_operator(input: &mut Input) -> Result<(Span, ArithmeticOperator)> {
    alt((
        t(TokenKind::Star).map(|token| (token.span(), ArithmeticOperator::Mul)),
        t(TokenKind::Slash).map(|token| (token.span(), ArithmeticOperator::Div)),
    ))
    .parse_next(input)
}

fn infix_or(input: &mut Input) -> Result<Expression<Unresolved>> {
    (infix_and, repeat(0.., (t(TokenKind::Or), infix_and)))
        .map(
            |(first, rest): (Expression<Unresolved>, Vec<(_, Expression<Unresolved>)>)| {
                rest.into_iter().fold(first, |left, (_or_tok, right)| {
                    fold_condition_expression(left, right, |span, left, right| {
                        Condition::Or(span, Box::new(left), Box::new(right))
                    })
                })
            },
        )
        .parse_next(input)
}

fn infix_and(input: &mut Input) -> Result<Expression<Unresolved>> {
    (infix_not, repeat(0.., (t(TokenKind::And), infix_not)))
        .map(
            |(first, rest): (Expression<Unresolved>, Vec<(_, Expression<Unresolved>)>)| {
                rest.into_iter().fold(first, |left, (_and_tok, right)| {
                    fold_condition_expression(left, right, |span, left, right| {
                        Condition::And(span, Box::new(left), Box::new(right))
                    })
                })
            },
        )
        .parse_next(input)
}

fn infix_not(input: &mut Input) -> Result<Expression<Unresolved>> {
    alt((
        (t(TokenKind::Not), infix_not).map(|(not_tok, expr)| {
            wrap_condition_expression(Condition::Not(
                not_tok.span.join(expr.span()),
                Box::new(expression_to_condition(expr)),
            ))
        }),
        infix_is,
    ))
    .parse_next(input)
}

fn infix_is(input: &mut Input) -> Result<Expression<Unresolved>> {
    (
        condition_bool,
        opt((
            t(TokenKind::Is),
            t(TokenKind::Dot),
            local_name,
            condition_payload_pattern,
        )),
    )
        .map(|(value, suffix)| match suffix {
            Some((_is_tok, _, variant, pattern)) => {
                let end_span = pattern.span();
                wrap_condition_expression(Condition::Is {
                    span: value.span().join(end_span),
                    value,
                    variant,
                    pattern,
                })
            }
            None => value,
        })
        .parse_next(input)
}

fn condition_bool(input: &mut Input) -> Result<Expression<Unresolved>> {
    infix_comparison(input)
}

fn infix_comparison(input: &mut Input) -> Result<Expression<Unresolved>> {
    (
        infix_additive,
        repeat(0.., (comparison_operator, infix_additive)),
    )
        .map(
            |(first, rest): (
                Expression<Unresolved>,
                Vec<((Span, ComparisonOperator), Expression<Unresolved>)>,
            )| {
                if rest.is_empty() {
                    first
                } else {
                    let span = first.span().join(rest.last().unwrap().1.span());
                    Expression::ComparisonChain {
                        span,
                        first: Box::new(first),
                        rest: rest
                            .into_iter()
                            .map(|((op_span, op), expr)| ComparisonStep { op_span, op, expr })
                            .collect(),
                    }
                }
            },
        )
        .parse_next(input)
}

fn infix_additive(input: &mut Input) -> Result<Expression<Unresolved>> {
    (
        infix_multiplicative,
        repeat(0.., (additive_operator, infix_multiplicative)),
    )
        .map(
            |(first, rest): (
                Expression<Unresolved>,
                Vec<((Span, ArithmeticOperator), Expression<Unresolved>)>,
            )| {
                rest.into_iter()
                    .fold(first, |left, ((op_span, op), right)| {
                        let span = left.span().join(right.span());
                        Expression::Arithmetic {
                            span,
                            op_span,
                            op,
                            left: Box::new(left),
                            right: Box::new(right),
                        }
                    })
            },
        )
        .parse_next(input)
}

fn infix_multiplicative(input: &mut Input) -> Result<Expression<Unresolved>> {
    (
        infix_unary,
        repeat(0.., (multiplicative_operator, infix_unary)),
    )
        .map(
            |(first, rest): (
                Expression<Unresolved>,
                Vec<((Span, ArithmeticOperator), Expression<Unresolved>)>,
            )| {
                rest.into_iter()
                    .fold(first, |left, ((op_span, op), right)| {
                        let span = left.span().join(right.span());
                        Expression::Arithmetic {
                            span,
                            op_span,
                            op,
                            left: Box::new(left),
                            right: Box::new(right),
                        }
                    })
            },
        )
        .parse_next(input)
}

fn infix_unary(input: &mut Input) -> Result<Expression<Unresolved>> {
    alt((
        (t(TokenKind::Neg), infix_unary).map(|(neg_tok, expr)| Expression::Neg {
            span: neg_tok.span.join(expr.span()),
            op_span: neg_tok.span(),
            expr: Box::new(expr),
        }),
        data_expression,
    ))
    .parse_next(input)
}

fn starts_data_expression_token(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Float
            | TokenKind::Integer
            | TokenKind::String
            | TokenKind::TemplateStart
            | TokenKind::LParen
            | TokenKind::LCurly
            | TokenKind::LBrack
            | TokenKind::Dot
            | TokenKind::Bang
            | TokenKind::LowercaseIdentifier
            | TokenKind::UppercaseIdentifier
            | TokenKind::And
            | TokenKind::Or
            | TokenKind::Not
            | TokenKind::Neg
    )
}

fn ends_data_expression_token(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Float
            | TokenKind::Integer
            | TokenKind::String
            | TokenKind::TemplateEnd
            | TokenKind::RParen
            | TokenKind::RCurly
            | TokenKind::RBrack
            | TokenKind::Bang
            | TokenKind::LowercaseIdentifier
            | TokenKind::UppercaseIdentifier
            | TokenKind::And
            | TokenKind::Or
            | TokenKind::Not
            | TokenKind::Neg
    )
}

fn starts_condition_token(kind: TokenKind) -> bool {
    matches!(kind, TokenKind::Not) || starts_data_expression_token(kind)
}

fn looks_like_infix(input: &Input) -> bool {
    let tokens = input.iter().collect::<Vec<_>>();
    let mut depth = 0usize;

    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::TemplateStringStart | TokenKind::TemplateDataStart => {
                depth += 1;
                continue;
            }
            TokenKind::TemplateStart | TokenKind::TemplateEnd => {
                continue;
            }
            TokenKind::LParen | TokenKind::LCurly | TokenKind::LBrack => {
                depth += 1;
                continue;
            }
            TokenKind::RParen | TokenKind::RCurly | TokenKind::RBrack => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
                continue;
            }
            TokenKind::Comma | TokenKind::FatArrow if depth == 0 => {
                return false;
            }
            TokenKind::Dec
            | TokenKind::Def
            | TokenKind::Type
            | TokenKind::Module
            | TokenKind::Import
            | TokenKind::Export
                if depth == 0 && index > 0 =>
            {
                return false;
            }
            _ => {}
        }

        if depth != 0 {
            continue;
        }

        let prev = index.checked_sub(1).map(|i| tokens[i].kind);
        let next = tokens.get(index + 1).map(|token| token.kind);
        let is_infix = match token.kind {
            TokenKind::Or | TokenKind::And => {
                prev.is_some_and(ends_data_expression_token)
                    && next.is_some_and(starts_condition_token)
            }
            TokenKind::Not => next.is_some_and(starts_condition_token),
            TokenKind::Is => {
                prev.is_some_and(ends_data_expression_token) && matches!(next, Some(TokenKind::Dot))
            }
            TokenKind::Lt
            | TokenKind::Gt
            | TokenKind::LtEq
            | TokenKind::GtEq
            | TokenKind::EqEq
            | TokenKind::BangEq
            | TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash => {
                prev.is_some_and(ends_data_expression_token)
                    && next.is_some_and(starts_data_expression_token)
            }
            TokenKind::Neg => next.is_some_and(starts_data_expression_token),
            _ => false,
        };

        if is_infix {
            return true;
        }
    }

    false
}

fn expression(input: &mut Input) -> Result<Expression<Unresolved>> {
    if looks_like_infix(input) {
        let checkpoint = input.checkpoint();
        match expr_infix.parse_next(input) {
            Ok(expr) => return Ok(expr),
            Err(ErrMode::Backtrack(_)) => {
                input.reset(&checkpoint);
            }
            Err(e) => return Err(e),
        }
    }
    expression_no_condition
        .context(StrContext::Label("expression"))
        .parse_next(input)
}

fn expression_without_construction(input: &mut Input) -> Result<Expression<Unresolved>> {
    if looks_like_infix(input) {
        let checkpoint = input.checkpoint();
        match expr_infix.parse_next(input) {
            Ok(expr) => return Ok(expr),
            Err(ErrMode::Backtrack(_)) => {
                input.reset(&checkpoint);
            }
            Err(e) => return Err(e),
        }
    }
    expression_no_condition_without_construction
        .context(StrContext::Label("expression"))
        .parse_next(input)
}

fn expression_no_condition(input: &mut Input) -> Result<Expression<Unresolved>> {
    alt((
        expression_no_condition_without_construction,
        construction.map(|(span, construct)| Expression::Construction(span, construct)),
    ))
    .parse_next(input)
}

fn expression_no_condition_without_construction(
    input: &mut Input,
) -> Result<Expression<Unresolved>> {
    alt((
        expr_literal,
        expr_list,
        expr_let,
        expr_catch,
        expr_throw,
        expr_type_in,
        expr_poll,
        expr_repoll,
        expr_submit,
        expr_if,
        expr_do,
        expr_box,
        expr_chan,
        application,
        expr_grouped,
    ))
    .parse_next(input)
}

fn data_expression(input: &mut Input) -> Result<Expression<Unresolved>> {
    alt((
        data_expression_terminal,
        data_construction.map(|(span, construct)| Expression::Construction(span, construct)),
    ))
    .parse_next(input)
}

fn data_expression_terminal(input: &mut Input) -> Result<Expression<Unresolved>> {
    alt((expr_literal, expr_list, application, expr_grouped)).parse_next(input)
}

fn expr_grouped(input: &mut Input) -> Result<Expression<Unresolved>> {
    (t(TokenKind::LCurly), expression, t(TokenKind::RCurly))
        .map(|(open, expr, close)| {
            Expression::Grouped(open.span.join(close.span()), Box::new(expr))
        })
        .parse_next(input)
}

fn expr_infix(input: &mut Input) -> Result<Expression<Unresolved>> {
    infix_or(input)
}

fn expr_literal(input: &mut Input) -> Result<Expression<Unresolved>> {
    alt((
        expr_literal_float,
        expr_literal_int,
        expr_literal_string,
        expr_literal_template,
        expr_literal_bytes,
    ))
    .parse_next(input)
}

fn expr_literal_float(input: &mut Input) -> Result<Expression<Unresolved>> {
    literal_float
        .map(|(span, value)| Expression::Primitive(span, Primitive::Number(Number::Float(value))))
        .parse_next(input)
}

fn literal_float(input: &mut Input) -> Result<(Span, f64)> {
    t(TokenKind::Float)
        .map(|token| {
            let s: String = token.raw.chars().filter(|c| *c != '_').collect();
            (token.span(), s.parse::<f64>().unwrap())
        })
        .parse_next(input)
}

fn expr_list(input: &mut Input) -> Result<Expression<Unresolved>> {
    commit_after(
        t(TokenKind::Star),
        (
            t(TokenKind::LParen),
            list0(expression),
            t(TokenKind::RParen),
        ),
    )
    .map(|(pre, (_, items, post))| Expression::List(pre.span.join(post.span()), items))
    .parse_next(input)
}

fn expr_literal_int(input: &mut Input) -> Result<Expression<Unresolved>> {
    literal_int
        .map(|(span, i)| Expression::Primitive(span, Primitive::Number(Number::Int(i))))
        .parse_next(input)
}

fn literal_int(input: &mut Input) -> Result<(Span, BigInt)> {
    t(TokenKind::Integer)
        .map(|token| {
            let s: String = token.raw.chars().filter(|c| *c != '_').collect();
            (token.span(), BigInt::parse_bytes(s.as_bytes(), 10).unwrap())
        })
        .parse_next(input)
}

fn expr_literal_string(input: &mut Input) -> Result<Expression<Unresolved>> {
    t(TokenKind::String)
        .map(|token| {
            // validated in lexer
            let value = unescaper::unescape(token.raw).unwrap();
            Expression::Primitive(token.span(), Primitive::String(ParString::from(value)))
        })
        .parse_next(input)
}

fn expr_literal_template(input: &mut Input) -> Result<Expression<Unresolved>> {
    (
        t(TokenKind::TemplateStart),
        repeat(0.., template_part),
        t(TokenKind::TemplateEnd),
    )
        .map(|(open, parts, close)| Expression::Template {
            span: open.span().join(close.span()),
            parts,
        })
        .parse_next(input)
}

fn template_part(input: &mut Input) -> Result<TemplatePart<Unresolved>> {
    alt((
        t(TokenKind::TemplateText).map(|token| {
            // validated in lexer
            TemplatePart::Literal(ParString::from(unescape_template_text(token.raw).unwrap()))
        }),
        (
            t(TokenKind::TemplateStringStart),
            expression,
            t(TokenKind::RCurly),
        )
            .map(|(_, expr, _)| TemplatePart::StringExpr(expr)),
        (
            t(TokenKind::TemplateDataStart),
            expression,
            t(TokenKind::RCurly),
        )
            .map(|(_, expr, _)| TemplatePart::DataExpr(expr)),
    ))
    .parse_next(input)
}

fn expr_literal_bytes(input: &mut Input) -> Result<Expression<Unresolved>> {
    alt((expr_literal_bytes_empty, expr_literal_bytes_nonempty)).parse_next(input)
}

fn expr_literal_bytes_empty(input: &mut Input) -> Result<Expression<Unresolved>> {
    commit_after((t(TokenKind::Lt), t(TokenKind::Link)), t(TokenKind::Gt))
        .map(|((pre, _), post)| {
            Expression::Primitive(pre.span.join(post.span()), Primitive::Bytes(Bytes::new()))
        })
        .parse_next(input)
}

fn expr_literal_bytes_nonempty(input: &mut Input) -> Result<Expression<Unresolved>> {
    commit_after(
        (t(TokenKind::Lt), t(TokenKind::Lt)),
        (literal_bytes_inner, t(TokenKind::Gt), t(TokenKind::Gt)),
    )
    .map(|((pre, _), (bytes, _, post))| {
        Expression::Primitive(
            pre.span.join(post.span()),
            Primitive::Bytes(Bytes::from(bytes)),
        )
    })
    .parse_next(input)
}

fn literal_bytes_inner(input: &mut Input) -> Result<Vec<u8>> {
    repeat(0.., literal_byte).parse_next(input)
}

fn literal_byte(input: &mut Input) -> Result<u8> {
    literal_int
        .map(|(_, i)| {
            if i < BigInt::ZERO {
                let rem: BigInt = i % 256;
                if rem == BigInt::ZERO {
                    0
                } else {
                    (256 - rem.iter_u32_digits().next().unwrap_or(0)) as u8
                }
            } else {
                (i.iter_u32_digits().next().unwrap_or(0) % 256) as u8
            }
        })
        .parse_next(input)
}

fn expr_let(input: &mut Input) -> Result<Expression<Unresolved>> {
    commit_after(
        t(TokenKind::Let),
        (
            pattern,
            t(TokenKind::Eq),
            expression,
            t(TokenKind::In),
            expression,
        ),
    )
    .map(
        |(pre, (pattern, _, expression, _in_tok, body))| Expression::Let {
            span: pre.span.join(body.span()),
            pattern,
            expression: Box::new(expression),
            then: Box::new(body),
        },
    )
    .parse_next(input)
}

fn expr_catch(input: &mut Input) -> Result<Expression<Unresolved>> {
    commit_after(
        t(TokenKind::Catch),
        (
            label,
            pattern,
            t(TokenKind::FatArrow),
            expression,
            t(TokenKind::In),
            expression,
        ),
    )
    .map(
        |(pre, (label, pattern, _, block, _in_tok, then))| Expression::Catch {
            span: pre.span.join(then.span()),
            label,
            pattern,
            block: Box::new(block),
            then: Box::new(then),
        },
    )
    .parse_next(input)
}

fn expr_throw(input: &mut Input) -> Result<Expression<Unresolved>> {
    commit_after(t(TokenKind::Throw), (label, expression))
        .map(|(pre, (label, expression))| {
            Expression::Throw(
                pre.span.join(expression.span()),
                label,
                Box::new(expression),
            )
        })
        .parse_next(input)
}

fn expr_type_in(input: &mut Input) -> Result<Expression<Unresolved>> {
    commit_after(t(TokenKind::Type), (typ, t(TokenKind::In), expression))
        .map(|(pre, (typ, _in_tok, expr))| Expression::TypeIn {
            span: pre.span.join(expr.span()),
            typ,
            expr: Box::new(expr),
        })
        .parse_next(input)
}

fn expr_if(input: &mut Input) -> Result<Expression<Unresolved>> {
    commit_after(
        t(TokenKind::If),
        (
            t(TokenKind::LCurly),
            repeat(1.., expr_if_branch),
            opt((
                t(TokenKind::Else),
                cut_err((t(TokenKind::FatArrow), expression)),
                opt(t(TokenKind::Comma)),
            )),
            t(TokenKind::RCurly),
        ),
    )
    .map(|(kw, (_open, branches, else_part, close))| Expression::If {
        span: kw.span.join(close.span()),
        branches,
        else_: else_part.map(|(_, (_, else_expr), _)| Box::new(else_expr)),
    })
    .parse_next(input)
}

fn expr_poll(input: &mut Input) -> Result<Expression<Unresolved>> {
    commit_after(
        t(TokenKind::Poll),
        (
            label,
            t(TokenKind::LParen),
            list0(expression),
            t(TokenKind::RParen),
            t(TokenKind::LCurly),
            local_name,
            t(TokenKind::FatArrow),
            expression,
            opt(t(TokenKind::Comma)),
            t(TokenKind::Else),
            cut_err((t(TokenKind::FatArrow), expression)),
            opt(t(TokenKind::Comma)),
            t(TokenKind::RCurly),
        ),
    )
    .map(
        |(
            kw,
            (
                label,
                _open,
                clients,
                _close,
                _open2,
                name,
                _arrow,
                then,
                _comma1,
                _else_kw,
                (_, else_),
                _comma2,
                close,
            ),
        )| {
            Expression::Poll {
                span: kw.span.join(close.span()),
                label,
                clients,
                name,
                then: Box::new(then),
                else_: Box::new(else_),
            }
        },
    )
    .parse_next(input)
}

fn expr_repoll(input: &mut Input) -> Result<Expression<Unresolved>> {
    commit_after(
        t(TokenKind::Repoll),
        (
            label,
            t(TokenKind::LParen),
            list0(expression),
            t(TokenKind::RParen),
            t(TokenKind::LCurly),
            local_name,
            t(TokenKind::FatArrow),
            expression,
            opt(t(TokenKind::Comma)),
            t(TokenKind::Else),
            cut_err((t(TokenKind::FatArrow), expression)),
            opt(t(TokenKind::Comma)),
            t(TokenKind::RCurly),
        ),
    )
    .map(
        |(
            kw,
            (
                label,
                _open,
                clients,
                _close,
                _open2,
                name,
                _arrow,
                then,
                _comma1,
                _else_kw,
                (_, else_),
                _comma2,
                close,
            ),
        )| {
            Expression::Repoll {
                span: kw.span.join(close.span()),
                label,
                clients,
                name,
                then: Box::new(then),
                else_: Box::new(else_),
            }
        },
    )
    .parse_next(input)
}

fn expr_submit(input: &mut Input) -> Result<Expression<Unresolved>> {
    commit_after(
        t(TokenKind::Submit),
        (
            label,
            t(TokenKind::LParen),
            list0(expression),
            t(TokenKind::RParen),
        ),
    )
    .map(|(kw, (label, _open, values, close))| Expression::Submit {
        span: kw.span.join(close.span()),
        label,
        values,
    })
    .parse_next(input)
}

fn expr_if_branch(input: &mut Input) -> Result<(Condition<Unresolved>, Expression<Unresolved>)> {
    (
        condition,
        t(TokenKind::FatArrow),
        expression,
        opt(t(TokenKind::Comma)),
    )
        .map(|(condition, _, body, _)| (condition, body))
        .parse_next(input)
}

fn expr_do(input: &mut Input) -> Result<Expression<Unresolved>> {
    commit_after(
        t(TokenKind::Do),
        (
            t(TokenKind::LCurly),
            opt(process),
            (t(TokenKind::RCurly), t(TokenKind::In)),
            expression,
        ),
    )
    .map(
        |(pre, (open, process, (_close, _in_tok), expression))| Expression::Do {
            span: pre.span.join(expression.span()),
            process: match process {
                Some((_, process)) => Box::new(process),
                None => Box::new(Process::Fallthrough(open.span.only_end())),
            },
            then: Box::new(expression),
        },
    )
    .parse_next(input)
}

fn expr_box(input: &mut Input) -> Result<Expression<Unresolved>> {
    commit_after(t(TokenKind::Box), expression)
        .map(|(pre, expression)| {
            Expression::Box(pre.span.join(expression.span()), Box::new(expression))
        })
        .parse_next(input)
}

fn expr_chan(input: &mut Input) -> Result<Expression<Unresolved>> {
    commit_after(
        t(TokenKind::Chan),
        (
            pattern,
            t(TokenKind::LCurly),
            opt(process),
            t(TokenKind::RCurly),
        ),
    )
    .map(|(pre, (pattern, open, process, close))| Expression::Chan {
        span: pre.span.join(close.span.clone()),
        pattern,
        process: match process {
            Some((_, process)) => Box::new(process),
            None => Box::new(Process::Fallthrough(open.span.only_end())),
        },
    })
    .parse_next(input)
}

fn construction(input: &mut Input) -> Result<(Span, Construct<Unresolved>)> {
    alt((
        cons_then,
        cons_begin,
        cons_unfounded,
        cons_loop,
        cons_signal,
        cons_case,
        cons_break,
        cons_send,
        cons_receive,
        cons_generic_receive,
    ))
    .context(StrContext::Label("construction"))
    .parse_next(input)
}

fn data_construction(input: &mut Input) -> Result<(Span, Construct<Unresolved>)> {
    alt((
        data_cons_signal,
        data_cons_break,
        data_cons_send,
        data_cons_then,
    ))
    .context(StrContext::Label("data construction"))
    .parse_next(input)
}

fn cons_then(input: &mut Input) -> Result<(Span, Construct<Unresolved>)> {
    expression_without_construction
        .map(|expr| {
            let span = expr.span();
            (span, Construct::Then(Box::new(expr)))
        })
        .parse_next(input)
}

fn data_cons_then(input: &mut Input) -> Result<(Span, Construct<Unresolved>)> {
    data_expression_terminal
        .map(|expr| {
            let span = expr.span();
            (span, Construct::Then(Box::new(expr)))
        })
        .parse_next(input)
}

fn cons_send(input: &mut Input) -> Result<(Span, Construct<Unresolved>)> {
    commit_after(
        t(TokenKind::LParen),
        (
            list1(|input: &mut Input| send_prefix_item(TokenKind::RParen, input)),
            t(TokenKind::RParen),
            construction,
        ),
    )
    .map(|(open, (items, close, (then_full_span, then)))| {
        let short_span = open.span.join(close.span());
        let full_span = open.span.join(then_full_span);
        let construct = fold_send_prefix(
            items,
            then,
            |typ, then| Construct::SendType(short_span.clone(), typ, Box::new(then)),
            |arg, then| Construct::Send(short_span.clone(), Box::new(arg), Box::new(then)),
        );
        (full_span, construct)
    })
    .parse_next(input)
}

fn data_cons_send(input: &mut Input) -> Result<(Span, Construct<Unresolved>)> {
    commit_after(
        t(TokenKind::LParen),
        (
            list1(|input: &mut Input| send_prefix_item(TokenKind::RParen, input)),
            t(TokenKind::RParen),
            data_construction,
        ),
    )
    .map(|(open, (items, close, (then_full_span, then)))| {
        let short_span = open.span.join(close.span());
        let full_span = open.span.join(then_full_span);
        let construct = fold_send_prefix(
            items,
            then,
            |typ, then| Construct::SendType(short_span.clone(), typ, Box::new(then)),
            |arg, then| Construct::Send(short_span.clone(), Box::new(arg), Box::new(then)),
        );
        (full_span, construct)
    })
    .parse_next(input)
}

fn cons_receive(input: &mut Input) -> Result<(Span, Construct<Unresolved>)> {
    commit_after(
        t(TokenKind::LBrack),
        (
            list1(pattern_prefix_item),
            t(TokenKind::RBrack),
            construction,
        ),
    )
    .map(|(open, (items, close, (then_full_span, then)))| {
        let short_span = open.span.join(close.span());
        let full_span = open.span.join(then_full_span);
        let construct = fold_pattern_prefix(
            items,
            then,
            |name, then| Construct::ReceiveType(short_span.clone(), name, Box::new(then)),
            |pattern, then| Construct::Receive(short_span.clone(), pattern, Box::new(then), vec![]),
        );
        (full_span, construct)
    })
    .parse_next(input)
}

fn data_cons_signal(input: &mut Input) -> Result<(Span, Construct<Unresolved>)> {
    (t(TokenKind::Dot), (local_name, data_construction))
        .map(|(pre, (chosen, (then_full_span, construct)))| {
            let short_span = pre.span.join(chosen.span());
            let full_span = pre.span.join(then_full_span);
            (
                full_span,
                Construct::Signal(short_span, chosen, Box::new(construct)),
            )
        })
        .parse_next(input)
}

fn data_cons_break(input: &mut Input) -> Result<(Span, Construct<Unresolved>)> {
    t(TokenKind::Bang)
        .map(|token| {
            let span = token.span();
            (span.clone(), Construct::Break(span))
        })
        .parse_next(input)
}

fn cons_generic_receive(input: &mut Input) -> Result<(Span, Construct<Unresolved>)> {
    commit_after(
        t(TokenKind::Lt),
        (
            list1(type_parameter),
            t(TokenKind::Gt),
            t(TokenKind::LBrack),
            pattern,
            t(TokenKind::RBrack),
            construction,
        ),
    )
    .map(
        |(
            vars_open,
            (vars, _vars_close, _pattern_open, arg, pattern_close, (then_full_span, rest)),
        )| {
            let short_span = vars_open.span.join(pattern_close.span());
            let full_span = vars_open.span.join(then_full_span);
            (
                full_span,
                Construct::Receive(short_span, arg, Box::new(rest), vars),
            )
        },
    )
    .parse_next(input)
}

fn cons_signal(input: &mut Input) -> Result<(Span, Construct<Unresolved>)> {
    // Note this can't be a commit_after because its possible that this is not a signal construction, and instead a branch of an either.
    (t(TokenKind::Dot), (local_name, construction))
        .map(|(pre, (chosen, (then_full_span, construct)))| {
            let short_span = pre.span.join(chosen.span());
            let full_span = pre.span.join(then_full_span);
            (
                full_span,
                Construct::Signal(short_span, chosen, Box::new(construct)),
            )
        })
        .parse_next(input)
}

fn cons_case(input: &mut Input) -> Result<(Span, Construct<Unresolved>)> {
    commit_after(t(TokenKind::Case), branches_body(cons_branch))
        .map(|(pre, (branches_span, branches, else_branch))| {
            let full_span = pre.span.join(branches_span);
            let short_span = pre.span();
            (
                full_span,
                Construct::Case(short_span, ConstructBranches(branches), else_branch),
            )
        })
        .parse_next(input)
}

fn cons_break(input: &mut Input) -> Result<(Span, Construct<Unresolved>)> {
    t(TokenKind::Bang)
        .map(|token| {
            let span = token.span();
            (span.clone(), Construct::Break(span))
        })
        .parse_next(input)
}

fn cons_begin(input: &mut Input) -> Result<(Span, Construct<Unresolved>)> {
    commit_after(t(TokenKind::Begin), (label, construction))
        .map(|(begin_kw, (label, (then_full_span, construct)))| {
            let short_span = match &label {
                Some(label) => begin_kw.span.join(label.span()),
                None => begin_kw.span(),
            };
            let full_span = begin_kw.span.join(then_full_span);
            (
                full_span,
                Construct::Begin {
                    span: short_span,
                    unfounded: false,
                    label,
                    then: Box::new(construct),
                },
            )
        })
        .parse_next(input)
}

fn cons_unfounded(input: &mut Input) -> Result<(Span, Construct<Unresolved>)> {
    commit_after(t(TokenKind::Unfounded), (label, construction))
        .map(|(unfounded_kw, (label, (then_full_span, construct)))| {
            let short_span = match &label {
                Some(label) => unfounded_kw.span.join(label.span()),
                None => unfounded_kw.span(),
            };
            let full_span = unfounded_kw.span.join(then_full_span);
            (
                full_span,
                Construct::Begin {
                    span: short_span,
                    unfounded: true,
                    label,
                    then: Box::new(construct),
                },
            )
        })
        .parse_next(input)
}

fn cons_loop(input: &mut Input) -> Result<(Span, Construct<Unresolved>)> {
    commit_after(t(TokenKind::Loop), label)
        .map(|(token, label)| {
            let span = match &label {
                Some(label) => token.span.join(label.span()),
                None => token.span(),
            };
            (span.clone(), Construct::Loop(span, label))
        })
        .parse_next(input)
}

fn cons_branch(input: &mut Input) -> Result<ConstructBranch<Unresolved>> {
    alt((
        cons_branch_then,
        cons_branch_receive,
        cons_branch_generic_receive,
    ))
    .parse_next(input)
}

fn cons_branch_then(input: &mut Input) -> Result<ConstructBranch<Unresolved>> {
    commit_after(t(TokenKind::FatArrow), expression)
        .map(|(pre, expression)| {
            ConstructBranch::Then(pre.span.join(expression.span()), expression)
        })
        .parse_next(input)
}

fn cons_branch_receive(input: &mut Input) -> Result<ConstructBranch<Unresolved>> {
    commit_after(
        t(TokenKind::LParen),
        (
            list1(pattern_prefix_item),
            t(TokenKind::RParen),
            cons_branch,
        ),
    )
    .map(|(open, (items, _close, rest))| {
        let span = open.span.join(rest.span());
        fold_pattern_prefix(
            items,
            rest,
            |name, rest| ConstructBranch::ReceiveType(span.clone(), name, Box::new(rest)),
            |pattern, rest| ConstructBranch::Receive(span.clone(), pattern, Box::new(rest), vec![]),
        )
    })
    .parse_next(input)
}

fn cons_branch_generic_receive(input: &mut Input) -> Result<ConstructBranch<Unresolved>> {
    commit_after(
        t(TokenKind::Lt),
        (
            list1(type_parameter),
            t(TokenKind::Gt),
            t(TokenKind::LParen),
            pattern,
            t(TokenKind::RParen),
            cons_branch,
        ),
    )
    .map(
        |(vars_open, (vars, _vars_close, _pattern_open, arg, _pattern_close, rest))| {
            let span = vars_open.span.join(rest.span());
            ConstructBranch::Receive(span.clone(), arg, Box::new(rest), vars)
        },
    )
    .parse_next(input)
}

fn application(input: &mut Input) -> Result<Expression<Unresolved>> {
    (
        alt((
            global_name.map(|name| Expression::Global(name.span(), name)),
            local_name.map(|name| Expression::Variable(name.span(), name)),
            expr_grouped,
        )),
        apply,
    )
        .map(|(expr, apply)| match apply {
            Some((full_span, apply)) => {
                Expression::Application(expr.span().join(full_span), Box::new(expr), apply)
            }
            None => expr,
        })
        .context(StrContext::Label("application"))
        .parse_next(input)
}

fn apply(input: &mut Input) -> Result<Option<(Span, Apply<Unresolved>)>> {
    opt(alt((
        apply_begin,
        apply_unfounded,
        apply_loop,
        apply_signal,
        apply_case,
        apply_send,
        apply_default,
        apply_try,
        apply_pipe,
    )))
    .parse_next(input)
}

fn apply_send(input: &mut Input) -> Result<(Span, Apply<Unresolved>)> {
    commit_after(
        t(TokenKind::LParen),
        (
            list1(|input: &mut Input| send_prefix_item(TokenKind::RParen, input)),
            t(TokenKind::RParen),
            apply,
        ),
    )
    .map(|(open, (items, close, then))| {
        let (then_full_span, then) = match then {
            Some((span, apply)) => (span, apply),
            None => {
                let s = close.span.only_end();
                (s.clone(), Apply::Noop(s))
            }
        };
        let short_span = open.span.join(close.span());
        let full_span = open.span.join(then_full_span);
        let apply = fold_send_prefix(
            items,
            then,
            |typ, then| Apply::SendType(short_span.clone(), typ, Box::new(then)),
            |arg, then| Apply::Send(short_span.clone(), Box::new(arg), Box::new(then)),
        );
        (full_span, apply)
    })
    .parse_next(input)
}

fn apply_signal(input: &mut Input) -> Result<(Span, Apply<Unresolved>)> {
    (t(TokenKind::Dot), (local_name, apply))
        .map(|(pre, (chosen, then))| {
            let (then_full_span, then) = match then {
                Some((span, apply)) => (span, apply),
                None => {
                    let s = chosen.span.only_end();
                    (s.clone(), Apply::Noop(s))
                }
            };
            let short_span = pre.span.join(chosen.span());
            let full_span = pre.span.join(then_full_span);
            (full_span, Apply::Signal(short_span, chosen, Box::new(then)))
        })
        .parse_next(input)
}

fn apply_case(input: &mut Input) -> Result<(Span, Apply<Unresolved>)> {
    commit_after(
        (t(TokenKind::Dot), t(TokenKind::Case)),
        branches_body(apply_branch),
    )
    .map(|((pre, case_kw), (branches_span, branches, else_branch))| {
        let full_span = pre.span.join(branches_span);
        let short_span = pre.span.join(case_kw.span());
        (
            full_span,
            Apply::Case(short_span, ApplyBranches(branches), else_branch),
        )
    })
    .parse_next(input)
}

fn apply_begin(input: &mut Input) -> Result<(Span, Apply<Unresolved>)> {
    commit_after((t(TokenKind::Dot), t(TokenKind::Begin)), (label, apply))
        .map(|((pre, begin_kw), (label, then))| {
            let (then_full_span, then) = match (&label, then) {
                (_, Some((span, apply))) => (span, apply),
                (Some(label), None) => {
                    let s = label.span.only_end();
                    (s.clone(), Apply::Noop(s))
                }
                (None, None) => {
                    let s = begin_kw.span.only_end();
                    (s.clone(), Apply::Noop(s))
                }
            };
            let short_span = match &label {
                Some(label) => pre.span.join(label.span()),
                None => pre.span.join(begin_kw.span()),
            };
            let full_span = pre.span.join(then_full_span);
            (
                full_span,
                Apply::Begin {
                    span: short_span,
                    unfounded: false,
                    label,
                    then: Box::new(then),
                },
            )
        })
        .parse_next(input)
}

fn apply_unfounded(input: &mut Input) -> Result<(Span, Apply<Unresolved>)> {
    commit_after((t(TokenKind::Dot), t(TokenKind::Unfounded)), (label, apply))
        .map(|((pre, unfounded_kw), (label, then))| {
            let (then_full_span, then) = match (&label, then) {
                (_, Some((span, apply))) => (span, apply),
                (Some(label), None) => {
                    let s = label.span.only_end();
                    (s.clone(), Apply::Noop(s))
                }
                (None, None) => {
                    let s = unfounded_kw.span.only_end();
                    (s.clone(), Apply::Noop(s))
                }
            };
            let short_span = match &label {
                Some(label) => pre.span.join(label.span()),
                None => pre.span.join(unfounded_kw.span()),
            };
            let full_span = pre.span.join(then_full_span);
            (
                full_span,
                Apply::Begin {
                    span: short_span,
                    unfounded: true,
                    label,
                    then: Box::new(then),
                },
            )
        })
        .parse_next(input)
}

fn apply_loop(input: &mut Input) -> Result<(Span, Apply<Unresolved>)> {
    commit_after((t(TokenKind::Dot), t(TokenKind::Loop)), label)
        .map(|((pre1, pre2), label)| {
            let span = match &label {
                Some(label) => pre1.span.join(label.span()),
                None => pre1.span.join(pre2.span()),
            };
            (span.clone(), Apply::Loop(span, label))
        })
        .parse_next(input)
}

fn apply_try(input: &mut Input) -> Result<(Span, Apply<Unresolved>)> {
    commit_after((t(TokenKind::Dot), t(TokenKind::Try)), (label, apply))
        .map(|((dot, pre), (label, then))| {
            let (then_full_span, then) = match then {
                Some((span, apply)) => (span, apply),
                None => {
                    let s = pre.span.only_end();
                    (s.clone(), Apply::Noop(s))
                }
            };
            let short_span = match &label {
                Some(label) => dot.span.join(label.span()),
                None => dot.span.join(pre.span()),
            };
            let full_span = dot.span.join(then_full_span);
            (full_span, Apply::Try(short_span, label, Box::new(then)))
        })
        .parse_next(input)
}

fn apply_default(input: &mut Input) -> Result<(Span, Apply<Unresolved>)> {
    commit_after(
        (t(TokenKind::Dot), t(TokenKind::Default)),
        (
            t(TokenKind::LParen),
            expression,
            t(TokenKind::RParen),
            apply,
        ),
    )
    .map(|((dot, _pre), (_, expr, close, then))| {
        let (then_full_span, then) = match then {
            Some((span, apply)) => (span, apply),
            None => {
                let s = close.span.only_end();
                (s.clone(), Apply::Noop(s))
            }
        };
        let short_span = dot.span.join(close.span());
        let full_span = dot.span.join(then_full_span);
        (
            full_span,
            Apply::Default(short_span, Box::new(expr), Box::new(then)),
        )
    })
    .parse_next(input)
}

fn apply_pipe(input: &mut Input) -> Result<(Span, Apply<Unresolved>)> {
    commit_after(
        t(TokenKind::ThinArrow),
        (
            alt((
                global_name.map(|name| Expression::Global(name.span(), name)),
                local_name.map(|name| Expression::Variable(name.span(), name)),
                expr_grouped,
            )),
            apply,
        ),
    )
    .map(|(pre, (function, then))| {
        let (then_full_span, then) = match then {
            Some((span, apply)) => (span, apply),
            None => {
                let s = function.span().only_end();
                (s.clone(), Apply::Noop(s))
            }
        };
        let short_span = pre.span.join(function.span());
        let full_span = pre.span.join(then_full_span);
        (
            full_span,
            Apply::Pipe(short_span, Box::new(function), Box::new(then)),
        )
    })
    .parse_next(input)
}

fn apply_branch(input: &mut Input) -> Result<ApplyBranch<Unresolved>> {
    alt((
        apply_branch_then,
        apply_branch_receive,
        apply_branch_generic_receive,
        apply_branch_continue,
        apply_branch_try,
        apply_branch_default,
    ))
    .parse_next(input)
}

fn apply_branch_then(input: &mut Input) -> Result<ApplyBranch<Unresolved>> {
    (local_name, cut_err((t(TokenKind::FatArrow), expression)))
        .map(|(name, (_, expression))| {
            ApplyBranch::Then(name.span.join(expression.span()), name, expression)
        })
        .parse_next(input)
}

fn apply_branch_receive(input: &mut Input) -> Result<ApplyBranch<Unresolved>> {
    commit_after(
        t(TokenKind::LParen),
        (
            list1(pattern_prefix_item),
            t(TokenKind::RParen),
            apply_branch,
        ),
    )
    .map(|(open, (items, _close, rest))| {
        let span = open.span.join(rest.span());
        fold_pattern_prefix(
            items,
            rest,
            |name, rest| ApplyBranch::ReceiveType(span.clone(), name, Box::new(rest)),
            |pattern, rest| ApplyBranch::Receive(span.clone(), pattern, Box::new(rest), vec![]),
        )
    })
    .parse_next(input)
}

fn apply_branch_generic_receive(input: &mut Input) -> Result<ApplyBranch<Unresolved>> {
    commit_after(
        t(TokenKind::Lt),
        (
            list1(type_parameter),
            t(TokenKind::Gt),
            t(TokenKind::LParen),
            pattern,
            t(TokenKind::RParen),
            apply_branch,
        ),
    )
    .map(
        |(vars_open, (vars, _vars_close, _pattern_open, arg, _pattern_close, rest))| {
            let span = vars_open.span.join(rest.span());
            ApplyBranch::Receive(span.clone(), arg, Box::new(rest), vars)
        },
    )
    .parse_next(input)
}

fn apply_branch_continue(input: &mut Input) -> Result<ApplyBranch<Unresolved>> {
    commit_after(t(TokenKind::Bang), (t(TokenKind::FatArrow), expression))
        .map(|(token, (_, expression))| {
            ApplyBranch::Continue(token.span.join(expression.span()), expression)
        })
        .parse_next(input)
}

fn apply_branch_try(input: &mut Input) -> Result<ApplyBranch<Unresolved>> {
    commit_after(t(TokenKind::Try), (label, apply_branch))
        .map(|(kw, (label, rest))| {
            ApplyBranch::Try(kw.span.join(rest.span()), label, Box::new(rest))
        })
        .parse_next(input)
}

fn apply_branch_default(input: &mut Input) -> Result<ApplyBranch<Unresolved>> {
    commit_after(
        (t(TokenKind::Default), t(TokenKind::LParen)),
        (expression, t(TokenKind::RParen), apply_branch),
    )
    .map(|((kw, _), (expr, _close, rest))| {
        ApplyBranch::Default(kw.span.join(rest.span()), Box::new(expr), Box::new(rest))
    })
    .parse_next(input)
}

fn process(input: &mut Input) -> Result<(Span, Process<Unresolved>)> {
    alt((
        proc_if,
        proc_poll,
        proc_repoll,
        proc_submit,
        proc_let,
        proc_compound_assign,
        proc_catch,
        proc_throw,
        global_command,
        command,
    ))
    .context(StrContext::Label("process"))
    .parse_next(input)
}

fn proc_let(input: &mut Input) -> Result<(Span, Process<Unresolved>)> {
    commit_after(
        t(TokenKind::Let),
        (pattern, t(TokenKind::Eq), expression, opt(process)),
    )
    .map(|(pre, (pattern, _, expression, then_opt))| {
        let span = pre.span.join(expression.span());
        let (full_span, then) = match then_opt {
            Some((then_full_span, then)) => (pre.span.join(then_full_span), Box::new(then)),
            None => (
                span.clone(),
                Box::new(Process::Fallthrough(expression.span().only_end())),
            ),
        };
        (
            full_span,
            Process::Let {
                span,
                pattern,
                then,
                value: Box::new(expression),
            },
        )
    })
    .parse_next(input)
}

fn compound_assign_operator(input: &mut Input) -> Result<(Span, ArithmeticOperator)> {
    alt((
        t(TokenKind::PlusEq).map(|token| (token.span(), ArithmeticOperator::Add)),
        t(TokenKind::MinusEq).map(|token| (token.span(), ArithmeticOperator::Sub)),
        t(TokenKind::StarEq).map(|token| (token.span(), ArithmeticOperator::Mul)),
        t(TokenKind::SlashEq).map(|token| (token.span(), ArithmeticOperator::Div)),
    ))
    .parse_next(input)
}

fn proc_compound_assign(input: &mut Input) -> Result<(Span, Process<Unresolved>)> {
    commit_after(
        (local_name, compound_assign_operator),
        (expression, opt(process)),
    )
    .map(|((name, (op_span, op)), (right, then_opt))| {
        let left = Expression::Variable(name.span(), name.clone());
        let value = Expression::Arithmetic {
            span: left.span().join(right.span()),
            op_span,
            op,
            left: Box::new(left),
            right: Box::new(right),
        };
        let span = name.span.join(value.span());
        let (full_span, then) = match then_opt {
            Some((then_full_span, then)) => (name.span.join(then_full_span), Box::new(then)),
            None => (
                span.clone(),
                Box::new(Process::Fallthrough(value.span().only_end())),
            ),
        };
        (
            full_span,
            Process::Let {
                span,
                pattern: Pattern::Name(name.span(), name, None),
                value: Box::new(value),
                then,
            },
        )
    })
    .parse_next(input)
}

fn proc_catch(input: &mut Input) -> Result<(Span, Process<Unresolved>)> {
    commit_after(
        t(TokenKind::Catch),
        (
            label,
            pattern,
            t(TokenKind::FatArrow),
            t(TokenKind::LCurly),
            process,
            t(TokenKind::RCurly),
            process,
        ),
    )
    .map(
        |(pre, (label, pattern, _, _, (_block_full_span, block), _, (then_full_span, then)))| {
            let span = pre.span.join(block.span());
            let full_span = pre.span.join(then_full_span);
            (
                full_span,
                Process::Catch {
                    span,
                    label,
                    pattern,
                    block: Box::new(block),
                    then: Box::new(then),
                },
            )
        },
    )
    .parse_next(input)
}

fn proc_throw(input: &mut Input) -> Result<(Span, Process<Unresolved>)> {
    commit_after(t(TokenKind::Throw), (label, expression))
        .map(|(pre, (label, expression))| {
            let span = pre.span.join(expression.span());
            (
                span.clone(),
                Process::Throw(span, label, Box::new(expression)),
            )
        })
        .parse_next(input)
}

fn proc_poll(input: &mut Input) -> Result<(Span, Process<Unresolved>)> {
    commit_after(
        t(TokenKind::Poll),
        (
            label,
            t(TokenKind::LParen),
            list0(expression),
            t(TokenKind::RParen),
            t(TokenKind::LCurly),
            local_name,
            t(TokenKind::FatArrow),
            t(TokenKind::LCurly),
            opt(process),
            t(TokenKind::RCurly),
            opt(t(TokenKind::Comma)),
            t(TokenKind::Else),
            cut_err((
                t(TokenKind::FatArrow),
                t(TokenKind::LCurly),
                opt(process),
                t(TokenKind::RCurly),
                opt(t(TokenKind::Comma)),
            )),
            t(TokenKind::RCurly),
        ),
    )
    .map(
        |(
            kw,
            (
                label,
                _open,
                clients,
                _close,
                _open2,
                name,
                _arrow,
                body_open,
                then,
                body_close,
                _comma1,
                _else_kw,
                (_else_arrow, else_open, else_body, else_close, _comma2),
                close,
            ),
        )| {
            let then = then
                .map(|(_, p)| p)
                .unwrap_or(Process::Fallthrough(body_open.span.join(body_close.span())));
            let else_body = else_body
                .map(|(_, p)| p)
                .unwrap_or(Process::Fallthrough(else_open.span.join(else_close.span())));
            let span = kw.span.join(close.span());
            (
                span.clone(),
                Process::Poll {
                    span,
                    label,
                    clients,
                    name,
                    then: Box::new(then),
                    else_: Box::new(else_body),
                },
            )
        },
    )
    .parse_next(input)
}

fn proc_repoll(input: &mut Input) -> Result<(Span, Process<Unresolved>)> {
    commit_after(
        t(TokenKind::Repoll),
        (
            label,
            t(TokenKind::LParen),
            list0(expression),
            t(TokenKind::RParen),
            t(TokenKind::LCurly),
            local_name,
            t(TokenKind::FatArrow),
            t(TokenKind::LCurly),
            opt(process),
            t(TokenKind::RCurly),
            opt(t(TokenKind::Comma)),
            t(TokenKind::Else),
            cut_err((
                t(TokenKind::FatArrow),
                t(TokenKind::LCurly),
                opt(process),
                t(TokenKind::RCurly),
                opt(t(TokenKind::Comma)),
            )),
            t(TokenKind::RCurly),
        ),
    )
    .map(
        |(
            kw,
            (
                label,
                _open,
                clients,
                _close,
                _open2,
                name,
                _arrow,
                body_open,
                then,
                body_close,
                _comma1,
                _else_kw,
                (_else_arrow, else_open, else_body, else_close, _comma2),
                close,
            ),
        )| {
            let then = then
                .map(|(_, p)| p)
                .unwrap_or(Process::Fallthrough(body_open.span.join(body_close.span())));
            let else_body = else_body
                .map(|(_, p)| p)
                .unwrap_or(Process::Fallthrough(else_open.span.join(else_close.span())));
            let span = kw.span.join(close.span());
            (
                span.clone(),
                Process::Repoll {
                    span,
                    label,
                    clients,
                    name,
                    then: Box::new(then),
                    else_: Box::new(else_body),
                },
            )
        },
    )
    .parse_next(input)
}

fn proc_submit(input: &mut Input) -> Result<(Span, Process<Unresolved>)> {
    commit_after(
        t(TokenKind::Submit),
        (
            label,
            t(TokenKind::LParen),
            list0(expression),
            t(TokenKind::RParen),
        ),
    )
    .map(|(kw, (label, _open, values, close))| {
        let span = kw.span.join(close.span());
        (
            span.clone(),
            Process::Submit {
                span,
                label,
                values,
            },
        )
    })
    .parse_next(input)
}

fn proc_if(input: &mut Input) -> Result<(Span, Process<Unresolved>)> {
    alt((proc_if_inline, proc_if_block)).parse_next(input)
}

fn proc_if_block(input: &mut Input) -> Result<(Span, Process<Unresolved>)> {
    commit_after(
        (t(TokenKind::If), t(TokenKind::LCurly)),
        (
            repeat(1.., proc_if_branch),
            opt((
                t(TokenKind::Else),
                cut_err(proc_if_else_body),
                opt(t(TokenKind::Comma)),
            )),
            t(TokenKind::RCurly),
            opt(pass_process),
        ),
    )
    .map(|((kw, open), (branches, else_body, close, then_process))| {
        let span = kw.span.join(close.span());
        let full_span = match &then_process {
            Some((then_full_span, _)) => kw.span.join(then_full_span.clone()),
            None => span.clone(),
        };
        (
            full_span,
            Process::If {
                span,
                branches,
                else_: else_body.map(|(_, body, _)| {
                    Box::new(body.unwrap_or(Process::Fallthrough(open.span.only_end())))
                }),
                then: then_process.map(|(_, p)| Box::new(p)),
            },
        )
    })
    .parse_next(input)
}

fn proc_if_branch(input: &mut Input) -> Result<(Condition<Unresolved>, Process<Unresolved>)> {
    (
        condition,
        t(TokenKind::FatArrow),
        t(TokenKind::LCurly),
        opt(process),
        t(TokenKind::RCurly),
        opt(t(TokenKind::Comma)),
    )
        .map(|(condition, _, open, body, close, _)| {
            (
                condition,
                body.map(|(_, p)| p)
                    .unwrap_or(Process::Fallthrough(open.span.join(close.span()))),
            )
        })
        .parse_next(input)
}

fn proc_if_else_body(input: &mut Input) -> Result<Option<Process<Unresolved>>> {
    (
        t(TokenKind::FatArrow),
        t(TokenKind::LCurly),
        opt(process),
        t(TokenKind::RCurly),
    )
        .map(|(_, open, body, close)| {
            body.map(|(_, p)| p)
                .unwrap_or(Process::Fallthrough(open.span.join(close.span())))
        })
        .map(Some)
        .parse_next(input)
}

fn proc_if_inline(input: &mut Input) -> Result<(Span, Process<Unresolved>)> {
    (
        t(TokenKind::If),
        alt((
            (t(TokenKind::LCurly), condition, t(TokenKind::RCurly))
                .map(|(_, condition, _)| condition),
            (not(peek(t(TokenKind::LCurly))), condition).map(|(_, condition)| condition),
        )),
        t(TokenKind::FatArrow),
        t(TokenKind::LCurly),
        opt(process),
        t(TokenKind::RCurly),
        opt(pass_process),
    )
        .map(|(kw, condition, _, open, then_proc, close, else_process)| {
            let (tail_full_span, tail_proc) = match else_process {
                Some((full_span, p)) => (full_span, p),
                None => {
                    let s = close.span().only_end();
                    (s.clone(), Process::Fallthrough(s))
                }
            };
            let span = kw.span.join(tail_proc.span());
            let full_span = kw.span.join(tail_full_span);
            (
                full_span,
                Process::If {
                    span,
                    branches: vec![(
                        condition,
                        then_proc
                            .map(|(_, p)| p)
                            .unwrap_or(Process::Fallthrough(open.span.join(close.span()))),
                    )],
                    else_: Some(Box::new(Process::Fallthrough(close.span().only_end()))),
                    then: Some(Box::new(tail_proc)),
                },
            )
        })
        .parse_next(input)
}

fn global_command(input: &mut Input) -> Result<(Span, Process<Unresolved>)> {
    (global_name, cmd)
        .map(|(name, cmd)| match cmd {
            Some((full_span, cmd)) => {
                let span = name.span.join(full_span);
                (span.clone(), Process::GlobalCommand(span, name, cmd))
            }
            None => {
                let span = name.span();
                let noop_span = name.span.only_end();
                (
                    span.clone(),
                    Process::GlobalCommand(span, name, noop_cmd(noop_span)),
                )
            }
        })
        .parse_next(input)
}

fn command(input: &mut Input) -> Result<(Span, Process<Unresolved>)> {
    (local_name, cmd)
        .map(|(name, cmd)| match cmd {
            Some((full_span, cmd)) => {
                let span = name.span.join(full_span);
                (span.clone(), Process::Command(span, name, cmd))
            }
            None => {
                let span = name.span();
                let noop_span = name.span.only_end();
                (
                    span.clone(),
                    Process::Command(span, name, noop_cmd(noop_span)),
                )
            }
        })
        .parse_next(input)
}

fn noop_cmd(span: Span) -> Command<Unresolved> {
    Command::Then(Box::new(Process::Fallthrough(span)))
}

fn cmd(input: &mut Input) -> Result<Option<(Span, Command<Unresolved>)>> {
    alt((
        alt((
            cmd_link,
            cmd_signal,
            cmd_case,
            cmd_break,
            cmd_continue,
            cmd_begin,
            cmd_unfounded,
            cmd_loop,
            cmd_send,
            cmd_receive,
            cmd_generic_receive,
            cmd_try,
            cmd_default,
            cmd_pipe,
        ))
        .map(Some),
        cmd_then,
    ))
    .context(StrContext::Label("command"))
    .parse_next(input)
}

fn cmd_then(input: &mut Input) -> Result<Option<(Span, Command<Unresolved>)>> {
    (opt(t(TokenKind::Semicolon)), opt(process))
        .map(|(_, opt)| {
            opt.map(|(full_span, process)| (full_span, Command::Then(Box::new(process))))
        })
        .parse_next(input)
}

fn cmd_link(input: &mut Input) -> Result<(Span, Command<Unresolved>)> {
    commit_after(t(TokenKind::Link), expression)
        .map(|(token, expression)| {
            let span = token.span.join(expression.span());
            (span.clone(), Command::Link(span, Box::new(expression)))
        })
        .parse_next(input)
}

fn cmd_send(input: &mut Input) -> Result<(Span, Command<Unresolved>)> {
    commit_after(
        t(TokenKind::LParen),
        (
            list1(|input: &mut Input| send_prefix_item(TokenKind::RParen, input)),
            t(TokenKind::RParen),
            cmd,
        ),
    )
    .map(|(open, (items, close, cmd))| {
        let (cmd_full_span, cmd) = match cmd {
            Some((span, cmd)) => (span, cmd),
            None => {
                let s = close.span.only_end();
                (s.clone(), noop_cmd(s))
            }
        };
        let short_span = open.span.join(close.span());
        let full_span = open.span.join(cmd_full_span);
        let cmd = fold_send_prefix(
            items,
            cmd,
            |typ, cmd| Command::SendType(short_span.clone(), typ, Box::new(cmd)),
            |expression, cmd| Command::Send(short_span.clone(), expression, Box::new(cmd)),
        );
        (full_span, cmd)
    })
    .parse_next(input)
}

fn cmd_receive(input: &mut Input) -> Result<(Span, Command<Unresolved>)> {
    commit_after(
        t(TokenKind::LBrack),
        (list1(pattern_prefix_item), t(TokenKind::RBrack), cmd),
    )
    .map(|(open, (items, close, cmd))| {
        let (cmd_full_span, cmd) = match cmd {
            Some((span, cmd)) => (span, cmd),
            None => {
                let s = close.span.only_end();
                (s.clone(), noop_cmd(s))
            }
        };
        let short_span = open.span.join(close.span());
        let full_span = open.span.join(cmd_full_span);
        let cmd = fold_pattern_prefix(
            items,
            cmd,
            |name, cmd| Command::ReceiveType(short_span.clone(), name, Box::new(cmd)),
            |pattern, cmd| Command::Receive(short_span.clone(), pattern, Box::new(cmd), vec![]),
        );
        (full_span, cmd)
    })
    .parse_next(input)
}

fn cmd_generic_receive(input: &mut Input) -> Result<(Span, Command<Unresolved>)> {
    commit_after(
        t(TokenKind::Lt),
        (
            list1(type_parameter),
            t(TokenKind::Gt),
            t(TokenKind::LBrack),
            pattern,
            t(TokenKind::RBrack),
            cmd,
        ),
    )
    .map(
        |(vars_open, (vars, _vars_close, _pattern_open, arg, pattern_close, cmd))| {
            let (cmd_full_span, cmd) = match cmd {
                Some((span, cmd)) => (span, cmd),
                None => {
                    let s = pattern_close.span.only_end();
                    (s.clone(), noop_cmd(s))
                }
            };
            let short_span = vars_open.span.join(pattern_close.span());
            let full_span = vars_open.span.join(cmd_full_span);
            (
                full_span,
                Command::Receive(short_span, arg, Box::new(cmd), vars),
            )
        },
    )
    .parse_next(input)
}

fn cmd_signal(input: &mut Input) -> Result<(Span, Command<Unresolved>)> {
    (t(TokenKind::Dot), (local_name, cmd))
        .map(|(pre, (name, cmd))| {
            let (cmd_full_span, cmd) = match cmd {
                Some((span, cmd)) => (span, cmd),
                None => {
                    let s = name.span.only_end();
                    (s.clone(), noop_cmd(s))
                }
            };
            let short_span = pre.span.join(name.span());
            let full_span = pre.span.join(cmd_full_span);
            (full_span, Command::Signal(short_span, name, Box::new(cmd)))
        })
        .parse_next(input)
}

fn cmd_case(input: &mut Input) -> Result<(Span, Command<Unresolved>)> {
    commit_after(
        (t(TokenKind::Dot), t(TokenKind::Case)),
        (branches_body(cmd_branch), opt(pass_process)),
    )
    .map(
        |((pre, case_kw), ((branches_span, branches, else_branch), pass_process))| {
            let full_span = match &pass_process {
                Some((ps_full_span, _)) => pre.span.join(ps_full_span.clone()),
                None => pre.span.join(branches_span),
            };
            let short_span = pre.span.join(case_kw.span());
            (
                full_span,
                Command::Case(
                    short_span,
                    CommandBranches(branches),
                    else_branch,
                    pass_process.map(|(_, p)| Box::new(p)),
                ),
            )
        },
    )
    .parse_next(input)
}

fn cmd_break(input: &mut Input) -> Result<(Span, Command<Unresolved>)> {
    t(TokenKind::Bang)
        .map(|token| {
            let span = token.span();
            (span.clone(), Command::Break(span))
        })
        .parse_next(input)
}

fn cmd_continue(input: &mut Input) -> Result<(Span, Command<Unresolved>)> {
    (t(TokenKind::Quest), opt(process))
        .map(|(token, process)| match process {
            Some((full_span, process)) => {
                let span = token.span.join(full_span);
                (span.clone(), Command::Continue(span, Box::new(process)))
            }
            None => {
                let span = token.span();
                (
                    span.clone(),
                    Command::Continue(span, Box::new(Process::Fallthrough(token.span.only_end()))),
                )
            }
        })
        .parse_next(input)
}

fn cmd_begin(input: &mut Input) -> Result<(Span, Command<Unresolved>)> {
    commit_after((t(TokenKind::Dot), t(TokenKind::Begin)), (label, cmd))
        .map(|((pre, begin_kw), (label, cmd))| {
            let (cmd_full_span, cmd) = match (&label, cmd) {
                (_, Some((span, cmd))) => (span, cmd),
                (Some(label), None) => {
                    let s = label.span.only_end();
                    (s.clone(), noop_cmd(s))
                }
                (None, None) => {
                    let s = begin_kw.span.only_end();
                    (s.clone(), noop_cmd(s))
                }
            };
            let short_span = match &label {
                Some(label) => pre.span.join(label.span()),
                None => pre.span.join(begin_kw.span()),
            };
            let full_span = pre.span.join(cmd_full_span);
            (
                full_span,
                Command::Begin {
                    span: short_span,
                    unfounded: false,
                    label,
                    then: Box::new(cmd),
                },
            )
        })
        .parse_next(input)
}

fn cmd_unfounded(input: &mut Input) -> Result<(Span, Command<Unresolved>)> {
    commit_after((t(TokenKind::Dot), t(TokenKind::Unfounded)), (label, cmd))
        .map(|((pre, unfounded_kw), (label, cmd))| {
            let (cmd_full_span, cmd) = match (&label, cmd) {
                (_, Some((span, cmd))) => (span, cmd),
                (Some(label), None) => {
                    let s = label.span.only_end();
                    (s.clone(), noop_cmd(s))
                }
                (None, None) => {
                    let s = unfounded_kw.span.only_end();
                    (s.clone(), noop_cmd(s))
                }
            };
            let short_span = match &label {
                Some(label) => pre.span.join(label.span()),
                None => pre.span.join(unfounded_kw.span()),
            };
            let full_span = pre.span.join(cmd_full_span);
            (
                full_span,
                Command::Begin {
                    span: short_span,
                    unfounded: true,
                    label,
                    then: Box::new(cmd),
                },
            )
        })
        .parse_next(input)
}

fn cmd_loop(input: &mut Input) -> Result<(Span, Command<Unresolved>)> {
    commit_after((t(TokenKind::Dot), t(TokenKind::Loop)), label)
        .map(|((pre1, pre2), label)| {
            let span = match &label {
                Some(label) => pre1.span.join(label.span()),
                None => pre1.span.join(pre2.span()),
            };
            (span.clone(), Command::Loop(span, label))
        })
        .parse_next(input)
}

fn cmd_try(input: &mut Input) -> Result<(Span, Command<Unresolved>)> {
    (t(TokenKind::Dot), (t(TokenKind::Try), label, cmd))
        .map(|(dot, (try_kw, label, cmd))| {
            let (cmd_full_span, cmd) = match cmd {
                Some((span, cmd)) => (span, cmd),
                None => {
                    let s = try_kw.span.only_end();
                    (s.clone(), noop_cmd(s))
                }
            };
            let short_span = match &label {
                Some(label) => dot.span.join(label.span()),
                None => dot.span.join(try_kw.span()),
            };
            let full_span = dot.span.join(cmd_full_span);
            (full_span, Command::Try(short_span, label, Box::new(cmd)))
        })
        .parse_next(input)
}

fn cmd_default(input: &mut Input) -> Result<(Span, Command<Unresolved>)> {
    (
        t(TokenKind::Dot),
        (
            t(TokenKind::Default),
            t(TokenKind::LParen),
            expression,
            t(TokenKind::RParen),
            cmd,
        ),
    )
        .map(|(dot, (_, _, expr, close, cmd))| {
            let (cmd_full_span, cmd) = match cmd {
                Some((span, cmd)) => (span, cmd),
                None => {
                    let s = close.span.only_end();
                    (s.clone(), noop_cmd(s))
                }
            };
            let short_span = dot.span.join(close.span());
            let full_span = dot.span.join(cmd_full_span);
            (
                full_span,
                Command::Default(short_span, Box::new(expr), Box::new(cmd)),
            )
        })
        .parse_next(input)
}

fn cmd_pipe(input: &mut Input) -> Result<(Span, Command<Unresolved>)> {
    commit_after(
        t(TokenKind::ThinArrow),
        (
            alt((
                global_name.map(|name| Expression::Global(name.span(), name)),
                local_name.map(|name| Expression::Variable(name.span(), name)),
                expr_grouped,
            )),
            cmd,
        ),
    )
    .map(|(pre, (function, cmd))| {
        let (cmd_full_span, cmd) = match cmd {
            Some((span, cmd)) => (span, cmd),
            None => {
                let s = function.span().only_end();
                (s.clone(), noop_cmd(s))
            }
        };
        let short_span = pre.span.join(function.span());
        let full_span = pre.span.join(cmd_full_span);
        (
            full_span,
            Command::Pipe(short_span, Box::new(function), Box::new(cmd)),
        )
    })
    .parse_next(input)
}

fn pass_process(input: &mut Input) -> Result<(Span, Process<Unresolved>)> {
    alt((proc_if, proc_let, global_command, command)).parse_next(input)
}

fn cmd_branch(input: &mut Input) -> Result<CommandBranch<Unresolved>> {
    alt((
        cmd_branch_then,
        cmd_branch_bind_then,
        cmd_branch_continue,
        cmd_branch_receive,
        cmd_branch_generic_receive,
        cmd_branch_try,
        cmd_branch_default,
    ))
    .parse_next(input)
}

fn cmd_branch_then(input: &mut Input) -> Result<CommandBranch<Unresolved>> {
    commit_after(
        t(TokenKind::FatArrow),
        (t(TokenKind::LCurly), opt(process), t(TokenKind::RCurly)),
    )
    .map(|(pre, (open, process, close))| {
        CommandBranch::Then(
            pre.span.join(close.span()),
            match process {
                Some((_, process)) => process,
                None => Process::Fallthrough(open.span.only_end()),
            },
        )
    })
    .parse_next(input)
}

fn cmd_branch_bind_then(input: &mut Input) -> Result<CommandBranch<Unresolved>> {
    (
        local_name,
        cut_err((
            t(TokenKind::FatArrow),
            (t(TokenKind::LCurly), opt(process), t(TokenKind::RCurly)),
        )),
    )
        .map(|(name, (pre, (open, process, close)))| {
            CommandBranch::BindThen(
                pre.span.join(close.span()),
                name,
                match process {
                    Some((_, process)) => process,
                    None => Process::Fallthrough(open.span.only_end()),
                },
            )
        })
        .parse_next(input)
}

fn cmd_branch_receive(input: &mut Input) -> Result<CommandBranch<Unresolved>> {
    commit_after(
        t(TokenKind::LParen),
        (list1(pattern_prefix_item), t(TokenKind::RParen), cmd_branch),
    )
    .map(|(open, (items, _close, rest))| {
        let span = open.span.join(rest.span());
        fold_pattern_prefix(
            items,
            rest,
            |name, rest| CommandBranch::ReceiveType(span.clone(), name, Box::new(rest)),
            |pattern, rest| CommandBranch::Receive(span.clone(), pattern, Box::new(rest), vec![]),
        )
    })
    .parse_next(input)
}

fn cmd_branch_generic_receive(input: &mut Input) -> Result<CommandBranch<Unresolved>> {
    commit_after(
        t(TokenKind::Lt),
        (
            list1(type_parameter),
            t(TokenKind::Gt),
            t(TokenKind::LParen),
            pattern,
            t(TokenKind::RParen),
            cmd_branch,
        ),
    )
    .map(
        |(vars_open, (vars, _vars_close, _pattern_open, arg, _pattern_close, rest))| {
            let span = vars_open.span.join(rest.span());
            CommandBranch::Receive(span.clone(), arg, Box::new(rest), vars)
        },
    )
    .parse_next(input)
}

fn cmd_branch_continue(input: &mut Input) -> Result<CommandBranch<Unresolved>> {
    commit_after(
        t(TokenKind::Bang),
        (
            t(TokenKind::FatArrow),
            t(TokenKind::LCurly),
            opt(process),
            t(TokenKind::RCurly),
        ),
    )
    .map(|(token, (_, open, process, close))| {
        CommandBranch::Continue(
            token.span.join(close.span()),
            match process {
                Some((_, process)) => process,
                None => Process::Fallthrough(open.span.only_end()),
            },
        )
    })
    .parse_next(input)
}

fn cmd_branch_try(input: &mut Input) -> Result<CommandBranch<Unresolved>> {
    commit_after(t(TokenKind::Try), (label, cmd_branch))
        .map(|(kw, (label, rest))| {
            CommandBranch::Try(kw.span.join(rest.span()), label, Box::new(rest))
        })
        .parse_next(input)
}

fn cmd_branch_default(input: &mut Input) -> Result<CommandBranch<Unresolved>> {
    commit_after(
        (t(TokenKind::Default), t(TokenKind::LParen)),
        (expression, t(TokenKind::RParen), cmd_branch),
    )
    .map(|((kw, _), (expr, _close, rest))| {
        CommandBranch::Default(kw.span.join(rest.span()), Box::new(expr), Box::new(rest))
    })
    .parse_next(input)
}

fn label(input: &mut Input) -> Result<Option<LocalName>> {
    opt(preceded(t(TokenKind::At), local_name)).parse_next(input)
}

#[cfg(test)]
mod test {
    use super::*;

    fn parse_single_definition_expression(source: &str) -> Expression<Unresolved> {
        let parsed = parse_source_file(source, "Main.par".into()).unwrap();
        match &parsed.body.definitions[0].body {
            DefinitionBody::Par(expr) => expr.clone(),
            DefinitionBody::External(_) => panic!("expected Par definition body"),
        }
    }

    #[test]
    fn test_parse_examples() {
        let input = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/src/HelloWorld.par"
        ));
        assert!(parse_module(input, "HelloWorld.par".into()).is_ok());
        let input = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/src/Fibonacci.par"
        ));
        assert!(parse_module(input, "Fibonacci.par".into()).is_ok());
        let input = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/src/RockPaperScissors.par"
        ));
        assert!(parse_module(input, "RockPaperScissors.par".into()).is_ok());
        let input = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/src/StringManipulation.par"
        ));
        assert!(parse_module(input, "StringManipulation.par".into()).is_ok());
        let input = "begin the errors";
        assert!(parse_module(input, "error.par".into()).is_err());
    }

    #[test]
    fn test_parse_if_syntax() {
        let expr_program = "def IfExpr = if { .true! => .false!, else => .true!, }";
        assert!(parse_module(expr_program, "if_expr.par".into()).is_ok());

        let proc_program =
            "def IfProc: ! = chan exit { if { .true! => { exit! } else => { exit! } } }";
        assert!(parse_module(proc_program, "if_proc.par".into()).is_ok());

        let inline_program = "def IfInline: ! = chan exit { if .true! => { exit! } exit! }";
        assert!(parse_module(inline_program, "if_inline.par".into()).is_ok());
    }

    #[test]
    fn test_parse_function_body_condition_after_receive() {
        let source = "\
module Minimal

import @core/Bool

def And = [a: Bool, b: Bool] a and b
";
        assert!(parse_module(source, "minimal.par".into()).is_ok());
    }

    #[test]
    fn test_parse_data_construction_condition_operands() {
        let source = "\
module Minimal

def BoolAnd = .true! and .false!
";
        assert!(parse_module(source, "minimal.par".into()).is_ok());
    }

    #[test]
    fn test_parse_infix_operator_precedence() {
        let expr = parse_single_definition_expression(
            "\
module Main

def Value = 1 + 2 * 3
",
        );

        match expr {
            Expression::Arithmetic {
                op: ArithmeticOperator::Add,
                left,
                right,
                ..
            } => {
                assert!(matches!(
                    *left,
                    Expression::Primitive(_, Primitive::Number(Number::Int(_)))
                ));
                assert!(matches!(
                    *right,
                    Expression::Arithmetic {
                        op: ArithmeticOperator::Mul,
                        ..
                    }
                ));
            }
            other => panic!("unexpected AST: {other:#?}"),
        }
    }

    #[test]
    fn test_parse_comparison_chains_and_grouping() {
        let chained = parse_single_definition_expression(
            "\
module Main

def Value = a < b < c
",
        );
        match chained {
            Expression::ComparisonChain { first, rest, .. } => {
                assert!(matches!(
                    *first,
                    Expression::Variable(_, LocalName { ref string, .. }) if string == "a"
                ));
                assert_eq!(rest.len(), 2);
                assert_eq!(rest[0].op, ComparisonOperator::Less);
                assert_eq!(rest[1].op, ComparisonOperator::Less);
                assert!(matches!(
                    rest[0].expr,
                    Expression::Variable(_, LocalName { ref string, .. }) if string == "b"
                ));
                assert!(matches!(
                    rest[1].expr,
                    Expression::Variable(_, LocalName { ref string, .. }) if string == "c"
                ));
            }
            other => panic!("unexpected AST: {other:#?}"),
        }

        let grouped = parse_single_definition_expression(
            "\
module Main

def Value = {a < b} < c
",
        );
        match grouped {
            Expression::ComparisonChain { first, rest, .. } => {
                assert_eq!(rest.len(), 1);
                assert!(matches!(
                    *first,
                    Expression::Grouped(_, inner)
                        if matches!(*inner, Expression::ComparisonChain { .. })
                ));
            }
            other => panic!("unexpected AST: {other:#?}"),
        }
    }

    #[test]
    fn test_parse_template_strings() {
        let expr = parse_single_definition_expression(
            "\
module Main

def Value = `Hi ${name}, age #{1 + {2}}.`
",
        );

        match expr {
            Expression::Template { parts, .. } => {
                assert_eq!(parts.len(), 5);
                assert!(
                    matches!(&parts[0], TemplatePart::Literal(value) if value.as_str() == "Hi ")
                );
                assert!(matches!(
                    &parts[1],
                    TemplatePart::StringExpr(Expression::Variable(_, LocalName { string, .. }))
                    if string == "name"
                ));
                assert!(
                    matches!(&parts[2], TemplatePart::Literal(value) if value.as_str() == ", age ")
                );
                assert!(matches!(
                    &parts[3],
                    TemplatePart::DataExpr(Expression::Arithmetic {
                        op: ArithmeticOperator::Add,
                        ..
                    })
                ));
                assert!(matches!(&parts[4], TemplatePart::Literal(value) if value.as_str() == "."));
            }
            other => panic!("unexpected AST: {other:#?}"),
        }
    }

    #[test]
    fn test_parse_empty_and_escaped_template_strings() {
        let empty = parse_single_definition_expression(
            "\
module Main

def Value = ``
",
        );
        assert!(matches!(empty, Expression::Template { parts, .. } if parts.is_empty()));

        let escaped = parse_single_definition_expression(
            r#"module Main

def Value = `\` \${ \#{ \n`
"#,
        );
        match escaped {
            Expression::Template { parts, .. } => {
                assert_eq!(parts.len(), 1);
                assert!(matches!(
                    &parts[0],
                    TemplatePart::Literal(value) if value.as_str() == "` ${ #{ \n"
                ));
            }
            other => panic!("unexpected AST: {other:#?}"),
        }
    }

    #[test]
    fn test_parse_not_and_neg_as_identifiers() {
        let source = "\
module Main

def UseNot = let not = 1 in not
def UseNeg = let neg = 2 in neg
";
        let parsed = parse_source_file(source, "Main.par".into()).unwrap();
        assert!(matches!(
            &parsed.body.definitions[0].body,
            DefinitionBody::Par(Expression::Let {
                pattern: Pattern::Name(_, LocalName { string, .. }, _),
                then,
                ..
            })
                if string == "not"
                    && matches!(
                        **then,
                        Expression::Variable(_, LocalName { ref string, .. })
                            if string == "not"
                    )
        ));
        assert!(matches!(
            &parsed.body.definitions[1].body,
            DefinitionBody::Par(Expression::Let {
                pattern: Pattern::Name(_, LocalName { string, .. }, _),
                then,
                ..
            })
                if string == "neg"
                    && matches!(
                        **then,
                        Expression::Variable(_, LocalName { ref string, .. })
                            if string == "neg"
                    )
        ));
    }

    #[test]
    fn test_parse_send_prefix_followed_by_box_case() {
        let source = "\
module Main

def Value = [type a, eq] (type List<box a>) box case {
  .empty => .end!,
}
";
        assert!(parse_module(source, "main.par".into()).is_ok());
    }

    #[test]
    fn test_parse_send_prefix_followed_by_loop() {
        let source = "\
module Main

def Value = [x] (x) loop
";
        assert!(parse_module(source, "main.par".into()).is_ok());
    }

    #[test]
    fn test_parse_deduplicate_example() {
        let input = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/src/Deduplicate.par"
        ));
        assert!(parse_module(input, "Deduplicate.par".into()).is_ok());
    }

    #[test]
    fn test_parse_sieve_test_module() {
        let input = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/src/Sieve.par"
        ));
        assert!(parse_module(input, "Sieve.par".into()).is_ok());
    }

    #[test]
    fn test_parse_box_case_with_multiple_branches() {
        let source = "\
module Main

def Value = [type a, eq] (type List<box a>) box case {
  .empty => .end!,
  .insert(x, set) => .item(x) set,
  .contains(y, set) => set.begin.case {
    .end! => .false!,
    .item(x) xs => eq(x, y).case {
      .true! => .true!,
      .false! => xs.loop,
    },
  },
}
";
        assert!(parse_module(source, "main.par".into()).is_ok());
    }

    #[test]
    fn test_parse_deduplicate_final_box_receive_call() {
        let source = "\
module Deduplicate

import {
  @core/Int
}

def IntListSet = ListSet(type Int, box Int.Equals)

def TestDedup =
  Deduplicate(type Int, IntListSet)
    (Map(type Int, type Int, box [n] Int.Mod(n, 7), Int.Range(1, 1000)))
";
        assert!(parse_module(source, "Deduplicate.par".into()).is_ok());
    }

    #[test]
    fn test_parse_unified_explicit_type_items() {
        let source = "\
module Minimal

dec Swap : [type a, type b, (a) b] (b) a
def Swap = [type a, type b, (x) y] (y) x

def Swapped = Swap(type String, type Int, (\"A\") 2)
";
        assert!(parse_module(source, "minimal.par".into()).is_ok());
    }

    #[test]
    fn test_parse_interleaved_explicit_type_items() {
        let source = "\
module Minimal

def Pack = [type a, x, type b, y] (x) y
def Packed = Pack(type String, \"left\", type Int, 1)
";
        assert!(parse_module(source, "minimal.par".into()).is_ok());
    }

    #[test]
    fn test_parse_condition_operands_with_grouping_and_receive() {
        let source = "\
module Minimal

def AfterReceive = [a: Bool] .true! and .false!
def AfterReceiveVariable = [a: Bool] .true! and a
def GroupedTypeIn = {type Bool in .true!} and {type Bool in .false!}
def GroupedReceive = {[a: Bool] a} and {type Bool in .false!}
def SendTypeData = (type Bool) .true! and .false!
";
        assert!(parse_module(source, "minimal.par".into()).is_ok());
    }

    #[test]
    fn test_parse_constrained_type_parameters() {
        let source = "\
module Minimal

dec Explicit : [type a: number, (a) !] !
dec Implicit : <a: signed>[a] a
def Explicit = [type a: number, p] !
def Implicit = <a: signed>[x] x
";
        assert!(parse_module(source, "minimal.par".into()).is_ok());
    }

    #[test]
    fn test_reject_constrained_type_definition_parameters() {
        let source = "\
module Minimal

type Boxed<a: box> = a
";
        assert!(parse_module(source, "minimal.par".into()).is_err());
    }

    #[test]
    fn test_doc_comments_attach_to_type_and_explicit_declaration() {
        let source = "\
/*Module docs*/
module Main

/*Type docs*/
type Item = !

// First line
// Second line
dec Run : !

// Not docs
def Helper: ! = external
";
        let parsed = parse_source_file(source, "Main.par".into()).unwrap();

        assert_eq!(
            parsed
                .module_decl
                .as_ref()
                .and_then(|module_decl| module_decl.doc.as_ref())
                .map(|doc| doc.markdown.as_str()),
            Some("Module docs")
        );
        assert_eq!(
            parsed.body.type_defs[0]
                .doc
                .as_ref()
                .map(|doc| doc.markdown.as_str()),
            Some("Type docs")
        );
        assert_eq!(
            parsed.body.declarations[0]
                .doc
                .as_ref()
                .map(|doc| doc.markdown.as_str()),
            Some("First line\nSecond line")
        );
        assert_eq!(parsed.body.declarations[1].doc, None);
    }

    #[test]
    fn test_doc_comments_require_direct_precedence() {
        let source = "\
/*Not attached to module*/

module Main

/*Not attached*/

type Item = !

// Also not attached

dec Run : !
";
        let parsed = parse_source_file(source, "Main.par".into()).unwrap();

        assert_eq!(
            parsed
                .module_decl
                .as_ref()
                .and_then(|module_decl| module_decl.doc.as_ref()),
            None
        );
        assert_eq!(parsed.body.type_defs[0].doc, None);
        assert_eq!(parsed.body.declarations[0].doc, None);
    }

    #[test]
    fn test_parse_float_literals() {
        let source = "\
module Main

def A = 1.0
def B = -0.5
def C = +3.25
def D = 1_000.25
def E = 6.02e23
def F = 1.0e-6
";
        assert!(parse_module(source, "float_literals.par".into()).is_ok());
    }

    #[test]
    fn test_reject_invalid_float_literals() {
        for source in [
            "module Main\ndef Bad = .5\n",
            "module Main\ndef Bad = 1.\n",
            "module Main\ndef Bad = 1e10\n",
            "module Main\ndef Bad = 1._0\n",
            "module Main\ndef Bad = 1.0e\n",
            "module Main\ndef Bad = 1.0e+\n",
        ] {
            assert!(parse_module(source, "bad_float.par".into()).is_err());
        }
    }
}
