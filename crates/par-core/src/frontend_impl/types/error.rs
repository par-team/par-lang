use crate::frontend_impl::language::{GlobalName, LocalName, TypeConstraint, Universal};
use crate::frontend_impl::types::{LoopId, Operation, Type};
use crate::location::Span;
use crate::workspace::{FileImportScope, render_global_name_in_scope, render_type_in_scope};
use miette::{LabeledSpan, SourceOffset, SourceSpan};
use std::fmt::Write;
use std::sync::Arc;

use super::Visibility;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TypeError<S> {
    TypeNameAlreadyDefined(Span, Span, GlobalName<S>),
    NameAlreadyDeclared(Span, Span, GlobalName<S>),
    NameAlreadyDefined(Span, Span, GlobalName<S>),
    DeclaredButNotDefined(Span, GlobalName<S>),
    ModuleExportInconsistent(Span, Span, S),
    ImportedModuleNotExported(Span, S),
    GlobalNameNotVisible(Span, GlobalName<S>, Visibility),
    VisibleItemExposesHiddenType(Span, GlobalName<S>, Visibility, GlobalName<S>, Visibility),
    NoMatchingRecursiveOrIterative(Span),
    SelfUsedInNegativePosition(Span),
    UnguardedRecursiveSelf(Span),
    UnguardedIterativeSelf(Span),
    TypeNameNotDefined(Span, GlobalName<S>),
    TypeVariableNotDefined(Span, LocalName),
    DependencyCycle(Span, Vec<GlobalName<S>>),
    WrongNumberOfTypeArgs(Span, GlobalName<S>, usize, usize),
    GlobalNameNotDefined(Span, GlobalName<S>),
    VariableDoesNotExist(Span, LocalName),
    ShadowedObligation(Span, LocalName),
    TypeMustBeKnownAtThisPoint(Span, #[allow(unused)] LocalName),
    ParameterTypeMustBeKnown(Span, LocalName),
    CannotAssignFromTo(Span, Type<S>, Type<S>),
    TypeDoesNotSatisfyConstraint(Span, LocalName, Type<S>, TypeConstraint),
    TypeParameterConstraintMismatch(Span, LocalName, TypeConstraint, TypeConstraint),
    UnfulfilledObligations(Span, Vec<LocalName>),
    InvalidOperation(Span, #[allow(unused)] Operation, Type<S>),
    InvalidBranch(Span, LocalName, Type<S>),
    MissingBranch(Span, LocalName, Type<S>),
    RedundantBranch(Span, LocalName, Type<S>),
    MultipleCleanupBranches(Span),
    CleanupBranchMismatch(Span, LocalName, bool),
    CleanupBranchMustBeExplicit(Span, LocalName),
    BlockVariableNotPreserved(Span, LocalName),
    MergeVariableTypesCannotBeUnified(Span, LocalName, Type<S>, Type<S>),
    VariableEscapesTypeScope(Span, LocalName),
    TypesCannotBeUnified(Span, Type<S>, Type<S>),
    NoSuchLoopPoint(Span, #[allow(unused)] Option<LocalName>),
    DoesNotDescendSubjectOfBegin(Span, #[allow(unused)] LoopId),
    CannotUnrollAscendantIterative(Span, #[allow(unused)] Option<LocalName>),
    LoopVariableNotPreserved(Span, LocalName),
    LoopVariableChangedType(Span, LocalName, Type<S>, Type<S>),
    PollMustHaveAtLeastOneClient(Span),
    PollClientMustBeRecursive(Span, Type<S>),
    SubmitOutsidePoll(Span),
    RepollOutsidePoll(Span),
    SubmittedClientNotAssignableToPoll(Span, Type<S>, Type<S>),
    SubmittedClientDoesNotDescend(Span),
    SubmitCannotTargetPollPoint(Span, Type<S>, Type<S>),
    PollVariableNotPreserved(Span, LocalName),
    PollVariableChangedType(Span, LocalName, Type<S>, Type<S>),
    PollBranchMustSubmit(Span),
    CannotUseLinearVariableInBox(Span, LocalName),
    NonExhaustiveIf(Span),
}

/// Create a `LabeledSpan` without a label at `span`
pub(crate) fn labels_from_span(_code: &str, span: &Span) -> Vec<LabeledSpan> {
    span.start()
        .into_iter()
        .map(|start| {
            LabeledSpan::new_with_span(
                None,
                SourceSpan::new(
                    SourceOffset::from(start.offset as usize),
                    span.len() as usize,
                ),
            )
        })
        .collect()
}

fn two_labels_from_two_spans(
    code: &str,
    span1: &Span,
    span2: &Span,
    label1: impl Into<Option<String>>,
    label2: impl Into<Option<String>>,
) -> Vec<LabeledSpan> {
    let mut labels = labels_from_span(code, span1);
    let label1 = label1.into();
    let label2 = label2.into();
    labels.iter_mut().for_each(|x| x.set_label(label1.clone()));
    let mut labels2 = labels_from_span(code, span2);
    labels2.iter_mut().for_each(|x| x.set_label(label2.clone()));
    labels.extend(labels2);
    labels
}

impl<S: Clone + Eq + std::hash::Hash + std::fmt::Display> TypeError<S> {
    fn to_report_with(
        &self,
        source_code: Arc<str>,
        render_name: impl Fn(&GlobalName<S>) -> String,
        render_type: impl Fn(&Type<S>, usize) -> String,
    ) -> miette::Report {
        let code = &source_code;
        match self {
            Self::TypeNameAlreadyDefined(span1, span2, name) => {
                let name = render_name(name);
                miette::miette!(
                    labels = two_labels_from_two_spans(code, span1, span2, "this".to_owned(), "is already defined here".to_owned()),
                    "Type `{}` is already defined.", name
                )
            }
            Self::NameAlreadyDeclared(span1, span2, name) => {
                let name = render_name(name);
                miette::miette!(
                    labels = two_labels_from_two_spans(code, span1, span2, "this".to_owned(), "is already declared here".to_owned()),
                    "`{}` is already declared.",
                    name,
                )
            }
            Self::NameAlreadyDefined(span1, span2, name) => {
                let name = render_name(name);
                miette::miette!(
                    labels = two_labels_from_two_spans(code, span1, span2, "this".to_owned(), "is already defined here".to_owned()),
                    "`{}` is already defined",
                    name,
                )
            }
            Self::DeclaredButNotDefined(span,  name) => {
                let name = render_name(name);
                let mut labels = labels_from_span(code, span);
                labels.iter_mut().for_each(|x| {
                    x.set_label(Some("declared here".to_owned()));
                });
                miette::miette!(
                    labels = labels,
                    "`{}` is declared, but is missing a corresponding definition.",
                    name
                )
            }
            Self::ModuleExportInconsistent(span1, span2, module) => {
                miette::miette!(
                    labels = two_labels_from_two_spans(
                        code,
                        span1,
                        span2,
                        "this file marks the module differently".to_owned(),
                        "than this file".to_owned(),
                    ),
                    "Module `{}` has inconsistent `export module` markings across its files.",
                    module,
                )
            }
            Self::ImportedModuleNotExported(span, module) => {
                let labels = labels_from_span(code, span);
                miette::miette!(
                    labels = labels,
                    "Module `{}` is not exported from its package.",
                    module,
                )
            }
            Self::GlobalNameNotVisible(span, name, visibility) => {
                let labels = labels_from_span(code, span);
                let name = render_name(name);
                miette::miette!(
                    labels = labels,
                    "`{}` is not visible from here.\n\nIt is {}.",
                    name,
                    visibility,
                )
            }
            Self::VisibleItemExposesHiddenType(
                span,
                item_name,
                item_visibility,
                type_name,
                type_visibility,
            ) => {
                let labels = labels_from_span(code, span);
                let item_name = render_name(item_name);
                let type_name = render_name(type_name);
                miette::miette!(
                    labels = labels,
                    "`{}` is {} but its type mentions `{}`, which is only {}.",
                    item_name,
                    item_visibility,
                    type_name,
                    type_visibility,
                )
            }
            Self::NoMatchingRecursiveOrIterative(span) => {
                let labels = labels_from_span(code, span);
                miette::miette!(
                    labels = labels,
                    "This `self` has no matching `recursive` or `iterative`.",
                )
            }
            Self::SelfUsedInNegativePosition(span) => {
                let labels = labels_from_span(code, span);
                miette::miette!(
                    labels = labels,
                    "This `self` is used in a negative position.\n\nNegative self-references are not allowed."
                )
            }
            Self::UnguardedRecursiveSelf(span) => {
                let labels = labels_from_span(code, span);
                miette::miette!(
                    labels = labels,
                    "This recursive's `self` is not guarded by an either.\n\nUnguarded self references are not allowed."
                )
            },
            Self::UnguardedIterativeSelf(span) => {
                let labels = labels_from_span(code, span);
                miette::miette!(
                    labels = labels,
                    "This iterative's `self` is not guarded by a choice.\n\nUnguarded self references are not allowed."
                )
            }
            Self::TypeNameNotDefined(span, name) => {
                let labels = labels_from_span(code, span);
                let name = render_name(name);
                miette::miette!(labels = labels, "Type `{}` is not defined.", name)
            }
            Self::TypeVariableNotDefined(span, name) => {
                let labels = labels_from_span(code, span);
                miette::miette!(labels = labels, "Type variable `{}` is not defined.", name)
            }
            Self::DependencyCycle(span, deps) => {
                let labels = labels_from_span(code, span);
                let mut deps_str = String::new();
                for (i, dep) in deps.iter().enumerate() {
                    if i > 0 {
                        write!(&mut deps_str, " -> ").unwrap();
                    }
                    write!(&mut deps_str, "{}", render_name(dep)).unwrap();
                }
                miette::miette!(
                    labels = labels,
                    "There is a dependency cycle:\n\n  {}\n\nDependency cycles are not allowed.",
                    deps_str
                )
            }
            Self::WrongNumberOfTypeArgs(span, name, required_number, provided_number) => {
                let labels = labels_from_span(code, span);
                let name = render_name(name);
                miette::miette!(
                    labels = labels,
                    "Type `{}` has {} type arguments, but {} were provided.",
                    name,
                    required_number,
                    provided_number
                )
            }
            Self::GlobalNameNotDefined(span, name) => {
                let labels = labels_from_span(code, span);
                let name = render_name(name);
                miette::miette!(labels = labels, "`{}` is not defined.", name)
            }
            Self::VariableDoesNotExist(span, name) => {
                let labels = labels_from_span(code, span);
                miette::miette!(labels = labels, "Variable `{}` does not exist.", name)
            }
            Self::ShadowedObligation(span, name) => {
                let labels = labels_from_span(code, span);
                miette::miette!(
                    labels = labels,
                    "Cannot re-assign `{}` before handling it.",
                    name,
                )
            }
            Self::TypeMustBeKnownAtThisPoint(span, name) => {
                let labels = labels_from_span(code, span);
                miette::miette!(labels = labels, "Type of `{}` must be known at this point.", name)
            }
            Self::ParameterTypeMustBeKnown(span, param) => {
                let labels = labels_from_span(code, span);

                // filter out internal pattern matching variables
                // issue #44: https://github.com/par-team/par-lang/issues/44
                if param.is_match() {
                    miette::miette!(
                        labels = labels,
                        help = "Consider adding a type annotation to the pattern, e.g., [(x : Type)y]",
                        "Type annotation required for pattern matching"
                    )
                } else {
                    miette::miette!(
                        labels = labels,
                        "Type of parameter `{}` must be known.",
                        param,
                    )
                }
            }
            Self::CannotAssignFromTo(span, from_type, to_type) => {
                let labels = labels_from_span(code, span);
                let from_type_str = render_type(from_type, 1);
                let to_type_str = render_type(to_type, 1);
                miette::miette!(
                    labels = labels,
                    "This type was required:\n\n  {}\n\nBut an incompatible type was provided:\n\n  {}\n",
                    to_type_str,
                    from_type_str,
                )
            }
            Self::TypeDoesNotSatisfyConstraint(span, name, typ, constraint) => {
                let labels = labels_from_span(code, span);
                let typ_str = render_type(typ, 1);
                miette::miette!(
                    labels = labels,
                    "Type argument for `{}` must satisfy the `{}` constraint, but got:\n\n  {}\n",
                    name,
                    constraint,
                    typ_str,
                )
            }
            Self::TypeParameterConstraintMismatch(span, name, written, expected) => {
                let labels = labels_from_span(code, span);
                let written = if *written == TypeConstraint::Any {
                    String::from("no constraint")
                } else {
                    format!("`{written}`")
                };
                let expected = if *expected == TypeConstraint::Any {
                    String::from("no constraint")
                } else {
                    format!("`{expected}`")
                };
                miette::miette!(
                    labels = labels,
                    "Type parameter `{}` is annotated with {}, but {} is required here.",
                    name,
                    written,
                    expected,
                )
            }
            Self::UnfulfilledObligations(span, names) => {
                let labels = labels_from_span(code, span);
                miette::miette!(
                    labels = labels,
                    "Cannot end this process before handling {}.",
                    names
                        .iter()
                        .enumerate()
                        .map(|(i, name)| if i == 0 {
                            format!("`{}`", name)
                        } else {
                            format!(", `{}`", name)
                        })
                        .collect::<String>()
                )
            }
            Self::InvalidOperation(span, _, typ) => {
                let labels = labels_from_span(code, span);
                let typ_str = render_type(typ, 1);
                miette::miette!(
                    labels = labels,
                    "This operation cannot be performed on:\n\n  {}\n",
                    typ_str
                )
            }
            Self::InvalidBranch(span, branch, typ) => {
                let labels = labels_from_span(code, span);
                let typ_str = render_type(typ, 1);
                miette::miette!(
                    labels = labels,
                    "Branch `{}` is not available on:\n\n  {}\n",
                    branch,
                    typ_str
                )
            }
            Self::MissingBranch(span, branch, typ) => {
                let labels = labels_from_span(code, span);
                let typ_str = render_type(typ, 1);
                miette::miette!(
                    labels = labels,
                    "Branch `{}` was not handled for:\n\n  {}\n",
                    branch,
                    typ_str
                )
            }
            Self::RedundantBranch(span, branch, typ) => {
                let labels = labels_from_span(code, span);
                let typ_str = render_type(typ, 1);
                miette::miette!(
                    labels = labels,
                    "Branch `{}` is not possible for:\n\n  {}\n",
                    branch,
                    typ_str
                )
            }
            Self::MultipleCleanupBranches(span) => {
                let labels = labels_from_span(code, span);
                miette::miette!(
                    labels = labels,
                    "An `either` or `choice` may have at most one cleanup branch."
                )
            }
            Self::CleanupBranchMismatch(span, branch, expected) => {
                let labels = labels_from_span(code, span);
                if *expected {
                    miette::miette!(
                        labels = labels,
                        "Branch `{}` must be marked `*` to match its type.",
                        branch
                    )
                } else {
                    miette::miette!(
                        labels = labels,
                        "Branch `{}` must not be marked `*`; its type does not mark it as the cleanup branch.",
                        branch
                    )
                }
            }
            Self::CleanupBranchMustBeExplicit(span, branch) => {
                let labels = labels_from_span(code, span);
                miette::miette!(
                    labels = labels,
                    "Cleanup branch `{}` must be handled explicitly and marked `*`; it cannot be covered by `else`.",
                    branch
                )
            }
            Self::BlockVariableNotPreserved(span, name) => {
                let labels = labels_from_span(code, span);
                miette::miette!(
                    labels = labels,
                    "Variable `{}` is not preserved along this control-flow path.",
                    name
                )
            }
            Self::MergeVariableTypesCannotBeUnified(span, name, t1, t2) => {
                let labels = labels_from_span(code, span);
                let t1s = render_type(t1, 1);
                let t2s = render_type(t2, 1);
                miette::miette!(
                    labels = labels,
                    "Types of `{}` across merging paths cannot be unified:\n\n  {}\n\n  {}\n",
                    name,
                    t1s,
                    t2s
                )
            }
            Self::VariableEscapesTypeScope(span, name) => {
                let labels = labels_from_span(code, span);
                miette::miette!(
                    labels = labels,
                    "Variable `{}` cannot escape the scope of a local type variable.",
                    name,
                )
            }
            Self::TypesCannotBeUnified(span, _typ1, _typ2) => {
                miette::miette!(
                    labels = labels_from_span(code, span),
                    "Types could not be unified here."
                )
            }
            Self::NoSuchLoopPoint(span, _) => {
                let labels = labels_from_span(code, span);
                miette::miette!(labels = labels, "There is no matching loop point in scope.")
            }
            Self::DoesNotDescendSubjectOfBegin(span, _) => {
                let labels = labels_from_span(code, span);
                miette::miette!(
                    labels = labels,
                    "This `loop` may diverge. Value does not descend from the corresponding `begin`.\n\nIf this is intended, use `unfounded`.",
                )
            }
            Self::LoopVariableNotPreserved(span, name) => {
                let labels = labels_from_span(code, span);
                miette::miette!(
                    labels = labels,
                    "`{}` is used by next iteration, but is no longer defined.",
                    name,
                )
            }
            Self::LoopVariableChangedType(span, name, loop_type, begin_type) => {
                let labels = labels_from_span(code, span);
                let loop_type_str = render_type(loop_type, 1);
                let begin_type_str = render_type(begin_type, 1);
                miette::miette!(
                    labels = labels,
                    "For next iteration, `{}` is required to be:\n\n  {}\n\nBut it has an incompatible type:\n\n  {}\n",
                    name,
                    begin_type_str,
                    loop_type_str,
                )
            }
            Self::PollMustHaveAtLeastOneClient(span) => {
                let labels = labels_from_span(code, span);
                miette::miette!(
                    labels = labels,
                    "`poll(...)` must have at least one initial client.",
                )
            }
            Self::PollClientMustBeRecursive(span, typ) => {
                let labels = labels_from_span(code, span);
                let typ_str = render_type(typ, 1);
                miette::miette!(
                    labels = labels,
                    "Clients of `poll(...)` must have a `recursive` type, but this has type:\n\n  {}\n",
                    typ_str,
                )
            }
            Self::SubmitOutsidePoll(span) => {
                let labels = labels_from_span(code, span);
                miette::miette!(labels = labels, "`submit(...)` can only be used inside a `poll` branch.")
            }
            Self::RepollOutsidePoll(span) => {
                let labels = labels_from_span(code, span);
                miette::miette!(labels = labels, "`repoll(...)` can only be used inside a `poll` branch.")
            }
            Self::SubmittedClientNotAssignableToPoll(span, client_type, poll_type) => {
                let labels = labels_from_span(code, span);
                let client_str = render_type(client_type, 1);
                let poll_str = render_type(poll_type, 1);
                miette::miette!(
                    labels = labels,
                    "This `submit(...)` cannot submit this client.\n\nIt has type:\n\n  {}\n\nBut this `poll(...)` expects clients of type:\n\n  {}\n",
                    client_str,
                    poll_str,
                )
            }
            Self::SubmittedClientDoesNotDescend(span) => {
                let labels = labels_from_span(code, span);
                miette::miette!(
                    labels = labels,
                    "This `submit(...)` cannot submit this client: it does not descend from the corresponding `poll(...)` client.",
                )
            }
            Self::SubmitCannotTargetPollPoint(span, current_point_type, target_point_type) => {
                let labels = labels_from_span(code, span);
                let _ = (current_point_type, target_point_type);
                miette::miette!(
                    labels = labels,
                    "This `submit(...)` cannot target this poll point.\n\nFrom here, the pool may still contain clients that do not descend from the targeted poll point.\n\nThis can happen when targeting an outer poll point from inside a `repoll(...)` handler: even if this `submit(...)` does not submit any incompatible clients, other clients may still remain in the pool.\n",
                )
            }
            Self::PollVariableNotPreserved(span, name) => {
                let labels = labels_from_span(code, span);
                miette::miette!(
                    labels = labels,
                    "`{}` is used by a subsequent poll iteration, but is no longer defined.",
                    name,
                )
            }
            Self::PollVariableChangedType(span, name, current_type, poll_type) => {
                let labels = labels_from_span(code, span);
                let current_type_str = render_type(current_type, 1);
                let poll_type_str = render_type(poll_type, 1);
                miette::miette!(
                    labels = labels,
                    "For the next poll iteration, `{}` is required to be:\n\n  {}\n\nBut it has an incompatible type:\n\n  {}\n",
                    name,
                    poll_type_str,
                    current_type_str,
                )
            }
            Self::PollBranchMustSubmit(span) => {
                let labels = labels_from_span(code, span);
                miette::miette!(
                    labels = labels,
                    "This branch must end by calling `submit(...)` or `repoll(...)` exactly once.",
                )
            }
            Self::CannotUseLinearVariableInBox(span, name) => {
                let labels = labels_from_span(code, span);
                miette::miette!(labels = labels, "Cannot use linear variable `{}` in a `box` expression.", name)
            }
            Self::NonExhaustiveIf(span) => {
                let labels = labels_from_span(code, span);
                miette::miette!(labels = labels, "Conditions are not exhaustive; an `else` branch is required here.")
            }
            Self::CannotUnrollAscendantIterative(span, _) => {
                let labels = labels_from_span(code, span);
                miette::miette!(
                    labels = labels,
                    "This `loop` may diverge. Operating on the loop variable is not allowed.\n\nIf this is intended, use `unfounded`.",
                )

            }
        }.with_source_code(source_code)
    }
}

impl TypeError<Universal> {
    pub fn to_report(
        &self,
        source_code: Arc<str>,
        scope: Option<&FileImportScope<Universal>>,
    ) -> miette::Report {
        self.to_report_with(
            source_code,
            |name| render_global_name_in_scope(scope, name),
            |typ, indent| render_type_in_scope(scope, typ, indent),
        )
    }
}

impl<S: Clone + Eq + std::hash::Hash> TypeError<S> {
    pub fn spans(&self) -> (Span, Option<Span>) {
        match self {
            Self::TypeNameAlreadyDefined(span1, span2, _)
            | Self::NameAlreadyDeclared(span1, span2, _)
            | Self::NameAlreadyDefined(span1, span2, _)
            | Self::ModuleExportInconsistent(span1, span2, _) => {
                (span1.clone(), Some(span2.clone()))
            }

            Self::DeclaredButNotDefined(span, _)
            | Self::ImportedModuleNotExported(span, _)
            | Self::GlobalNameNotVisible(span, _, _)
            | Self::VisibleItemExposesHiddenType(span, _, _, _, _)
            | Self::NoMatchingRecursiveOrIterative(span)
            | Self::SelfUsedInNegativePosition(span)
            | Self::UnguardedRecursiveSelf(span)
            | Self::UnguardedIterativeSelf(span)
            | Self::TypeNameNotDefined(span, _)
            | Self::TypeVariableNotDefined(span, _)
            | Self::DependencyCycle(span, _)
            | Self::WrongNumberOfTypeArgs(span, _, _, _)
            | Self::GlobalNameNotDefined(span, _)
            | Self::VariableDoesNotExist(span, _)
            | Self::ShadowedObligation(span, _)
            | Self::TypeMustBeKnownAtThisPoint(span, _)
            | Self::ParameterTypeMustBeKnown(span, _)
            | Self::CannotAssignFromTo(span, _, _)
            | Self::TypeDoesNotSatisfyConstraint(span, _, _, _)
            | Self::TypeParameterConstraintMismatch(span, _, _, _)
            | Self::UnfulfilledObligations(span, _)
            | Self::InvalidOperation(span, _, _)
            | Self::InvalidBranch(span, _, _)
            | Self::MissingBranch(span, _, _)
            | Self::RedundantBranch(span, _, _)
            | Self::MultipleCleanupBranches(span)
            | Self::CleanupBranchMismatch(span, _, _)
            | Self::CleanupBranchMustBeExplicit(span, _)
            | Self::BlockVariableNotPreserved(span, _)
            | Self::MergeVariableTypesCannotBeUnified(span, _, _, _)
            | Self::VariableEscapesTypeScope(span, _)
            | Self::NoSuchLoopPoint(span, _)
            | Self::DoesNotDescendSubjectOfBegin(span, _)
            | Self::LoopVariableNotPreserved(span, _)
            | Self::LoopVariableChangedType(span, _, _, _)
            | Self::PollMustHaveAtLeastOneClient(span)
            | Self::PollClientMustBeRecursive(span, _)
            | Self::SubmitOutsidePoll(span)
            | Self::RepollOutsidePoll(span)
            | Self::SubmittedClientNotAssignableToPoll(span, _, _)
            | Self::SubmittedClientDoesNotDescend(span)
            | Self::SubmitCannotTargetPollPoint(span, _, _)
            | Self::PollVariableNotPreserved(span, _)
            | Self::PollVariableChangedType(span, _, _, _)
            | Self::PollBranchMustSubmit(span)
            | Self::CannotUseLinearVariableInBox(span, _)
            | Self::NonExhaustiveIf(span)
            | Self::CannotUnrollAscendantIterative(span, _) => (span.clone(), None),

            Self::TypesCannotBeUnified(span, _typ1, _typ2) => (span.clone(), None),
        }
    }
}
