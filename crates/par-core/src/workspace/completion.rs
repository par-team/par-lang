use super::CheckedWorkspace;
use crate::frontend_impl::language::{LocalName, Universal};
use crate::frontend_impl::types::Type;
use crate::location::FileName;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionCandidate {
    pub label: String,
    pub insert_text: String,
    pub detail: String,
    pub kind: CompletionCandidateKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionCandidateKind {
    Keyword,
    Branch,
    ModuleType,
    ModuleDeclaration,
}

impl CheckedWorkspace {
    pub fn dot_completions_at(
        &self,
        file: &FileName,
        source: &str,
        row: u32,
        column: u32,
    ) -> Vec<CompletionCandidate> {
        let Some((dot, hover_row, hover_column)) = dot_before_position(source, row, column) else {
            return Vec::new();
        };
        let Some(completion_context) = DotCompletionContext::before_dot(source, dot) else {
            return Vec::new();
        };
        let active_loop_labels = loop_labels_before_dot(source, dot);

        let mut candidates = Vec::new();
        let completed_module_alias =
            self.push_module_alias_completion_candidates(file, source, dot, &mut candidates);

        // `push_module_alias_completion_candidates` returns true only for `Alias.` receivers
        // that resolve through imports (for example `Console.` or `raw->Os.`).
        // In that case alias-member completion is authoritative: if we also ran hover/type-based
        // completion, we'd incorrectly mix in value-type items like `begin`/branch labels.
        if !completed_module_alias {
            if let Some(hover) = self.hover_at(file, hover_row, hover_column) {
                if let Some(typ) = hover.typ() {
                    self.push_type_completion_candidates(
                        typ,
                        completion_context,
                        &active_loop_labels,
                        &mut candidates,
                    );
                }
            }
        }

        candidates.sort_by(|a, b| {
            a.label
                .cmp(&b.label)
                .then(a.insert_text.cmp(&b.insert_text))
        });
        candidates.dedup_by(|a, b| a.label == b.label && a.insert_text == b.insert_text);
        candidates
    }

    fn push_module_alias_completion_candidates(
        &self,
        file: &FileName,
        source: &str,
        dot: usize,
        candidates: &mut Vec<CompletionCandidate>,
    ) -> bool {
        let Some(alias) = module_alias_before_dot(source, dot) else {
            return false;
        };
        let Some(scope) = self.workspace.import_scope(file) else {
            return false;
        };
        let Some(module) = scope.aliases.get(alias) else {
            return false;
        };

        let vis = &self.workspace.visibility;
        let from = &scope.current_module;

        for (name, _) in self.checked.type_defs.globals.iter() {
            if name.module == *module
                && !name.is_primary_export()
                && vis.type_visible_from(from, name)
            {
                let label = name.primary.clone();
                candidates.push(CompletionCandidate {
                    insert_text: label.clone(),
                    label,
                    detail: "module type".to_string(),
                    kind: CompletionCandidateKind::ModuleType,
                });
            }
        }

        for name in self
            .checked
            .declarations
            .keys()
            .chain(self.checked.definitions.keys())
        {
            if name.module == *module
                && !name.is_primary_export()
                && vis.declaration_visible_from(from, name)
            {
                let label = name.primary.clone();
                candidates.push(CompletionCandidate {
                    insert_text: label.clone(),
                    label,
                    detail: "module declaration".to_string(),
                    kind: CompletionCandidateKind::ModuleDeclaration,
                });
            }
        }

        true
    }

    fn push_type_completion_candidates(
        &self,
        typ: &Type<Universal>,
        context: DotCompletionContext,
        active_loop_labels: &BTreeSet<String>,
        candidates: &mut Vec<CompletionCandidate>,
    ) {
        match typ {
            Type::Choice(_, branches) => {
                push_branch_completion_candidates(branches, "signal choice branch", candidates);
            }
            Type::Either(_, branches) if context == DotCompletionContext::Construction => {
                push_branch_completion_candidates(branches, "construct either branch", candidates);
            }
            Type::Either(_, branches) => {
                // Inside a recursive body's pre-`begin` phase, offering either-branch actions
                // (`case`, branch labels, `try`/`default`) is invalid: the user must first
                // establish the recursion point with `begin`/`unfounded`.
                if context == DotCompletionContext::RecursiveBody {
                    return;
                }
                let mut branch_names = branches.keys().map(|branch| branch.string.as_str());
                match (
                    branch_names.next(),
                    branch_names.next(),
                    branch_names.next(),
                ) {
                    (Some("err"), Some("ok"), None) => {
                        candidates.push(CompletionCandidate {
                            label: "try".to_string(),
                            insert_text: "try".to_string(),
                            detail: "propagate .err to the active catch and continue with .ok"
                                .to_string(),
                            kind: CompletionCandidateKind::Keyword,
                        });
                    }
                    (Some("none"), Some("some"), None) => {
                        candidates.push(CompletionCandidate {
                            label: "default".to_string(),
                            insert_text: "default(".to_string(),
                            detail: "use a default value for .none and continue with .some"
                                .to_string(),
                            kind: CompletionCandidateKind::Keyword,
                        });
                    }
                    _ => {}
                }
                candidates.push(CompletionCandidate {
                    label: "case".to_string(),
                    insert_text: "case {\n  ".to_string(),
                    detail: "case on either branches".to_string(),
                    kind: CompletionCandidateKind::Keyword,
                });
            }
            Type::Recursive {
                asc, label, body, ..
            } => {
                if context == DotCompletionContext::Normal {
                    for (keyword, detail) in [
                        ("begin", "begin recursive session"),
                        (
                            "unfounded",
                            "begin recursive session without totality checking",
                        ),
                    ] {
                        let insert_text = label.as_ref().map_or_else(
                            || keyword.to_string(),
                            |label| format!("{keyword}@{label}"),
                        );
                        candidates.push(CompletionCandidate {
                            label: keyword.to_string(),
                            insert_text,
                            detail: detail.to_string(),
                            kind: CompletionCandidateKind::Keyword,
                        });
                    }

                    // Offer `loop` when a typed ascendant exists (well-founded `begin`) or when
                    // source text before the cursor contains a matching labeled loop point
                    // (`begin@label`/`unfounded@label`), which can be valid even with empty `asc`.
                    let has_unlabeled_loop_point = active_loop_labels.contains("");
                    let has_any_labeled_loop_point =
                        active_loop_labels.iter().any(|label| !label.is_empty());
                    let bare_loop_insert_text = match label.as_ref() {
                        Some(label)
                            if !asc.is_empty()
                                || active_loop_labels.contains(label.string.as_str()) =>
                        {
                            Some(format!("loop@{label}"))
                        }
                        None => {
                            // In mixed contexts (an outer unlabeled begin plus an inner labeled
                            // begin/unfounded), bare `loop` is often illegal for the current value.
                            // Prefer explicit labeled loops whenever any labeled loop point is active.
                            (has_unlabeled_loop_point && !has_any_labeled_loop_point)
                                .then(|| "loop".to_string())
                        }
                        _ => None,
                    };
                    if let Some(insert_text) = bare_loop_insert_text {
                        candidates.push(CompletionCandidate {
                            label: "loop".to_string(),
                            insert_text,
                            detail: "loop to the matching begin".to_string(),
                            kind: CompletionCandidateKind::Keyword,
                        });
                    }

                    // Loop labels come from command labels, not from the recursive type itself.
                    // If the type is unlabeled, still surface explicit `loop@label` variants that
                    // are active in the source before the cursor.
                    if label.is_none() {
                        for active_label in
                            active_loop_labels.iter().filter(|label| !label.is_empty())
                        {
                            let loop_label = format!("loop@{active_label}");
                            candidates.push(CompletionCandidate {
                                label: loop_label.clone(),
                                insert_text: loop_label,
                                detail: "loop to an active labeled begin/unfounded".to_string(),
                                kind: CompletionCandidateKind::Keyword,
                            });
                        }
                    }
                }
                // Expand the recursive node and continue completion on its body shape.
                // When entering from `Normal`, switch to `RecursiveBody` so nested `either`
                // completions require `begin`/`unfounded` before offering `case` branches.
                if let Ok(expanded) =
                    Type::expand_recursive(&Default::default(), label, body, typ.display_hint())
                {
                    let next_context = if context == DotCompletionContext::Normal {
                        DotCompletionContext::RecursiveBody
                    } else {
                        context.descend_into_body()
                    };
                    self.push_type_completion_candidates(
                        &expanded,
                        next_context,
                        active_loop_labels,
                        candidates,
                    );
                }
            }
            Type::Iterative { body, .. } => {
                self.push_type_completion_candidates(
                    body,
                    context.descend_into_body(),
                    active_loop_labels,
                    candidates,
                );
            }
            Type::Box(_, inner) | Type::DualBox(_, inner) => {
                self.push_type_completion_candidates(
                    inner,
                    context,
                    active_loop_labels,
                    candidates,
                );
            }
            Type::Name(..) | Type::DualName(..) => {
                if let Ok(expanded) = typ.expand_definition(&self.checked.type_defs) {
                    self.push_type_completion_candidates(
                        &expanded,
                        context,
                        active_loop_labels,
                        candidates,
                    );
                }
            }
            Type::Primitive(..)
            | Type::DualPrimitive(..)
            | Type::Var(..)
            | Type::DualVar(..)
            | Type::Break(..)
            | Type::Continue(..)
            | Type::Self_(..)
            | Type::DualSelf(..)
            | Type::Hole(..)
            | Type::DualHole(..)
            | Type::Fail(..)
            | Type::Function(..)
            | Type::Pair(..)
            | Type::Forall(..)
            | Type::Exists(..) => {}
        }
    }
}

fn push_branch_completion_candidates(
    branches: &BTreeMap<LocalName, Type<Universal>>,
    detail: &'static str,
    candidates: &mut Vec<CompletionCandidate>,
) {
    for branch in branches.keys() {
        let label = branch.to_string();
        candidates.push(CompletionCandidate {
            insert_text: label.clone(),
            label,
            detail: detail.to_string(),
            kind: CompletionCandidateKind::Branch,
        });
    }
}

/// Returns the identifier-like token immediately before `.`.
///
/// Example: `module_alias_before_dot("Use(  Console .Open)", 14) == Some("Console")`.
fn module_alias_before_dot(source: &str, dot: usize) -> Option<&str> {
    let receiver = source.get(..dot)?.trim_end();
    let start = receiver
        .char_indices()
        .rev()
        .find_map(|(i, ch)| (!is_completion_suffix_char(ch)).then_some(i + ch.len_utf8()))
        .unwrap_or(0);
    receiver.get(start..).filter(|s| !s.is_empty())
}

/// Completion mode inferred from the text immediately before a `.` trigger.
///
/// Variants:
/// - `Normal`: receiver/member completion, e.g. `value.`
/// - `AfterBegin`: completion right after `begin`/`begin@label`, e.g. `list.begin.`
/// - `Construction`: constructor-style completion, e.g. `.`, or `.repeat.`
/// - `RecursiveBody`: completion inside a recursive body before `begin`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DotCompletionContext {
    Normal,
    AfterBeginOrUnfounded,
    Construction,
    RecursiveBody,
}

impl DotCompletionContext {
    /// Infers the completion context from the receiver text before `dot`.
    ///
    /// Distinguishes constructor-style completion, post-`begin` completion,
    /// and normal member completion.
    fn before_dot(source: &str, dot: usize) -> Option<Self> {
        let receiver = source.get(..dot)?.trim_end();
        let tail = receiver.split_whitespace().next_back()?;
        // If the token starts with a constructor-like segment (e.g. `.ok!`) but is followed by
        // a closed call and receiver chain/end (e.g. `Make(!, .ok!)` / `...)->Try.Ok`),
        // completion should use receiver/member semantics rather than constructor semantics.
        let has_postfix_receiver_after_closed_call =
            tail.rsplit_once(')').is_some_and(|(_, after)| {
                after.is_empty() || after.starts_with("->") || after.starts_with('.')
            });
        let separator_tail = tail
            .char_indices()
            .rev()
            .find_map(|(i, ch)| matches!(ch, '(' | ',' | ';' | '{' | '[').then_some(i + 1))
            .and_then(|start| tail.get(start..))
            .unwrap_or(tail);

        // Inside argument lists, completions like `f(.` / `f(.repeat.` should stay in
        // constructor context instead of falling back to receiver/member context.
        if (separator_tail.is_empty() || separator_tail.starts_with('.'))
            && !has_postfix_receiver_after_closed_call
        {
            return Some(Self::Construction);
        }

        if (tail.starts_with('.') && !has_postfix_receiver_after_closed_call)
            || !tail.chars().any(is_completion_suffix_char)
        {
            return Some(Self::Construction);
        }

        let last = tail.rsplit_once('.').map_or(tail, |(_, seg)| seg);
        Some(
            if last == "begin"
                || last.starts_with("begin@")
                || last == "unfounded"
                || last.starts_with("unfounded@")
            {
                Self::AfterBeginOrUnfounded
            } else {
                Self::Normal
            },
        )
    }

    fn descend_into_body(self) -> Self {
        match self {
            Self::Construction => Self::Construction,
            Self::AfterBeginOrUnfounded => Self::AfterBeginOrUnfounded,
            Self::Normal | Self::RecursiveBody => Self::Normal,
        }
    }
}

/// Finds the `.` that starts a completion receiver before `(row, column)`.
///
/// Returns `(dot_offset, dot_row, dot_column)` when completion should trigger.
/// Example: `dot_before_position("ab\nc.d", 1, 3) == Some((4, 1, 1))`.
fn dot_before_position(source: &str, row: u32, column: u32) -> Option<(usize, u32, u32)> {
    let cursor = offset_for_position(source, row, column)?;
    let prefix = source.get(..cursor)?;
    let trimmed = prefix.trim_end_matches(char::is_whitespace);
    if prefix.len() != trimmed.len() && !trimmed.ends_with('.') {
        return None;
    }
    let before_completion_prefix = trimmed.trim_end_matches(is_completion_suffix_char);
    let dot = before_completion_prefix.strip_suffix('.')?.len();
    row_and_column_for_offset(source, dot).map(|(dot_row, dot_column)| (dot, dot_row, dot_column))
}

fn is_completion_suffix_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

/// Collect active loop-point labels before `dot` from `begin`/`unfounded`.
/// Uses `""` as a sentinel for an unlabeled loop point.
/// Example: `loop_labels_before_dot("x.begin y.unfounded@outer z.", dot)`
/// returns `{ "", "outer" }`.
fn loop_labels_before_dot(source: &str, dot: usize) -> BTreeSet<String> {
    let mut labels = BTreeSet::new();
    let Some(prefix) = source.get(..dot) else {
        return labels;
    };

    for marker in ["begin@", "unfounded@"] {
        let mut scan_from = 0;
        while let Some(relative) = prefix[scan_from..].find(marker) {
            let label_start = scan_from + relative + marker.len();
            let tail = &prefix[label_start..];
            let label_end = tail
                .char_indices()
                .find_map(|(idx, ch)| (!is_completion_suffix_char(ch)).then_some(idx))
                .unwrap_or(tail.len());
            if label_end > 0 {
                labels.insert(tail[..label_end].to_string());
            }
            scan_from = label_start + label_end;
        }
    }

    for marker in ["begin", "unfounded"] {
        let mut scan_from = 0;
        while let Some(relative) = prefix[scan_from..].find(marker) {
            let start = scan_from + relative;
            let end = start + marker.len();
            let prev_is_suffix = start > 0
                && prefix[..start]
                    .chars()
                    .next_back()
                    .is_some_and(is_completion_suffix_char);
            let next = prefix[end..].chars().next();
            let next_is_suffix = next.is_some_and(is_completion_suffix_char);
            let next_is_labeled = next == Some('@');
            if !prev_is_suffix && !next_is_suffix && !next_is_labeled {
                labels.insert("".to_string());
            }
            scan_from = end;
        }
    }

    labels
}

/// Converts a zero-based `(row, UTF-16 column)` position in `source` to a byte `offset`.
///
/// Returns `None` when the position is out of bounds or not on a UTF-8 boundary.
/// Example: `offset_for_position("a\nb", 1, 0) == Some(2)`.
fn offset_for_position(source: &str, row: u32, column: u32) -> Option<usize> {
    let mut current_row = 0;
    let mut current_column = 0;
    for (offset, ch) in source.char_indices() {
        if current_row == row && current_column == column {
            return Some(offset);
        }
        if ch == '\n' {
            current_row += 1;
            current_column = 0;
        } else {
            current_column += ch.len_utf16() as u32;
        }
    }
    (current_row == row && current_column == column).then_some(source.len())
}

/// Converts a byte `offset` in `source` to zero-based `(row, UTF-16 column)`.
///
/// Returns `None` when `offset` is out of bounds or not on a UTF-8 boundary.
/// Example: `row_and_column_for_offset("a\nb", 2) == Some((1, 0))`.
fn row_and_column_for_offset(source: &str, offset: usize) -> Option<(u32, u32)> {
    if offset > source.len() || !source.is_char_boundary(offset) {
        return None;
    }
    let (mut row, mut column) = (0, 0);
    for ch in source.get(..offset)?.chars() {
        if ch == '\n' {
            row += 1;
            column = 0;
        } else {
            column += ch.len_utf16() as u32;
        }
    }
    Some((row, column))
}

#[cfg(test)]
mod tests {
    use super::super::{
        CheckedWorkspace, LoadedPackageFile, ParsedPackage, WorkspacePackage, WorkspacePackages,
        assemble_workspace, parse_loaded_files,
    };
    use super::*;
    use arcstr::literal;
    use par_runtime::pkgid::PackageId;
    use std::path::PathBuf;

    fn test_package_id() -> PackageId {
        PackageId::Special(literal!("__test__"))
    }

    #[test]
    fn position_conversions_use_utf16_columns() {
        let source = "aé😀\nb";

        assert_eq!(row_and_column_for_offset(source, 7), Some((0, 4)));
        assert_eq!(offset_for_position(source, 0, 4), Some(7));
        assert_eq!(offset_for_position(source, 0, 3), None);
        assert_eq!(row_and_column_for_offset(source, 8), Some((1, 0)));
    }

    fn parsed_package_from_source(source: &str) -> ParsedPackage {
        parse_loaded_files(vec![LoadedPackageFile {
            name: FileName::from("local/Main.par"),
            relative_path_from_src: PathBuf::from("Main.par"),
            source: source.to_owned(),
        }])
        .unwrap()
    }

    fn checked_workspace_from_source(source: &str) -> CheckedWorkspace {
        let parsed = parsed_package_from_source(source);
        let (checked, type_errors) = assemble_workspace(WorkspacePackages {
            root_package: test_package_id(),
            packages: vec![WorkspacePackage::new(test_package_id(), parsed)],
        })
        .unwrap()
        .type_check();
        assert!(type_errors.is_empty(), "type errors: {:?}", type_errors);
        checked
    }

    fn dot_completions_at_marker(
        checked: &CheckedWorkspace,
        live_source: &str,
        marker: &str,
    ) -> Vec<CompletionCandidate> {
        let file = checked.workspace().sources().keys().next().unwrap();
        dot_completions_at_marker_in_file(checked, file, live_source, marker)
    }

    fn dot_completions_at_marker_in_file(
        checked: &CheckedWorkspace,
        file: &FileName,
        live_source: &str,
        marker: &str,
    ) -> Vec<CompletionCandidate> {
        let cursor = live_source.match_indices(marker).last().unwrap().0 + marker.len();
        let (row, column) = row_and_column_for_offset(live_source, cursor).unwrap();
        checked.dot_completions_at(file, live_source, row, column)
    }

    fn assert_source_dot_completion_labels(
        source: &str,
        live_source: &str,
        marker: &str,
        expected: &[&str],
        unexpected: &[&str],
    ) {
        let checked = checked_workspace_from_source(source);
        let completions = dot_completions_at_marker(&checked, live_source, marker);
        for label in expected {
            assert!(
                completions
                    .iter()
                    .any(|candidate| candidate.label == *label),
                "missing completion label {label:?}"
            );
        }
        for label in unexpected {
            assert!(
                completions
                    .iter()
                    .all(|candidate| candidate.label != *label),
                "unexpected completion label {label:?}"
            );
        }
    }

    #[test]
    fn dot_completion_includes_choice_branches_for_receiver_contexts() {
        let cases = [
            (
                "choice",
                "def Main : Client = ClientValue\n",
                "ClientValue\n",
                "ClientValue.\n",
            ),
            (
                "choice",
                "def Main : ! = chan exit {\n    ClientValue.close\n    exit!\n}\n",
                "ClientValue.close",
                "ClientValue.",
            ),
            (
                "box choice",
                "def Main : Client = ClientValue\n",
                "ClientValue\n",
                "ClientValue.\n",
            ),
        ];

        for (type_prefix, main_definition, before, after) in cases {
            let source = format!(
                "\
module Main

type Client = {type_prefix} {{
    .close => !,
    .next => !,
}}

def ClientValue : Client = external
{main_definition}"
            );
            let live_source = source.replace(before, after);
            assert_source_dot_completion_labels(
                &source,
                &live_source,
                "ClientValue.",
                &["close", "next"],
                &[],
            );
        }
    }

    #[test]
    fn dot_completion_for_recursive_value_omits_bare_loop() {
        let source = "\
module Main

type Client = recursive either {
    .close!,
    .next self,
}

def ClientValue : Client = external
def Main : Client = ClientValue
";
        let live_source = source.replace("ClientValue\n", "ClientValue.\n");
        assert_source_dot_completion_labels(
            source,
            &live_source,
            "ClientValue.",
            &["begin", "unfounded"],
            &["case", "close", "next", "loop"],
        );
    }

    #[test]
    fn dot_completion_includes_recursive_keywords_for_path_shaped_type() {
        let source = "\
module Main

type Bytes = !

type Option<a> = either {
    .none!,
    .some a,
}

type Path = iterative@append recursive@parent box choice {
    .name => Bytes,
    .parent => Option<self@parent>,
    .append(Bytes) => self@append,
}

def PathValue : Path = external
def Main : Path = PathValue
";
        let live_source = source.replace("PathValue\n", "PathValue.\n");
        assert_source_dot_completion_labels(
            source,
            &live_source,
            "PathValue.",
            &["begin", "unfounded", "name", "parent", "append"],
            &["loop"],
        );
    }

    #[test]
    fn dot_completion_includes_loop_for_recursive_branch_variable() {
        let source = "\
module Main

type Items = recursive either {
    .end!,
    .item self,
}

def Walk : [Items] Items = [list]
    list.begin.case {
        .end! => list,
        .item list => list,
    }
";
        let live_source = source.replace(".item list => list,", ".item list => list.,");
        assert_source_dot_completion_labels(source, &live_source, "list.", &["loop"], &[]);
    }

    #[test]
    fn dot_completion_uses_labeled_recursive_insert_text() {
        let source = "\
module Main

type Items = recursive@items either {
    .end!,
    .item self@items,
}

def Walk : [Items] Items = [list]
    list.begin@items.case {
        .end! => list,
        .item list => list,
    }
";
        let live_source = source.replace(".item list => list,", ".item list => list.,");
        let checked = checked_workspace_from_source(source);
        let completions = dot_completions_at_marker(&checked, &live_source, "list.");
        let begin = completions
            .iter()
            .find(|candidate| candidate.label == "begin")
            .unwrap();
        let unfounded = completions
            .iter()
            .find(|candidate| candidate.label == "unfounded")
            .unwrap();
        let loop_ = completions
            .iter()
            .find(|candidate| candidate.label == "loop")
            .unwrap();

        assert_eq!(begin.insert_text, "begin@items");
        assert_eq!(unfounded.insert_text, "unfounded@items");
        assert_eq!(loop_.insert_text, "loop@items");
    }

    #[test]
    fn dot_completion_after_function_application_handles_recursive_contexts() {
        let source = "\
module Main

type Counter = recursive either {
    .end!,
    .step self,
}

dec Repeat : [!] Counter
def Repeat = external
def Main : ! = Repeat(!).begin.case {
    .end! => !,
    .step rest => !,
}
";

        for (replacement, marker, expected, unexpected) in [
            (
                "Repeat(!).",
                "Repeat(!).",
                &["begin", "unfounded"][..],
                &["loop", "case", "end", "step"][..],
            ),
            (
                "Repeat(!).begin.",
                "Repeat(!).begin.",
                &["case"][..],
                &["begin", "loop"][..],
            ),
        ] {
            let live_source = source.replace("Repeat(!).begin.case", replacement);
            assert_source_dot_completion_labels(source, &live_source, marker, expected, unexpected);
        }
    }

    #[test]
    fn dot_completion_for_recursive_either_value_omits_case_branches_before_begin() {
        let source = "\
module Main

type Server = recursive either {
    .shutdown!,
    .incoming self,
}

def Listen : Server = external
def Main : ! = Listen.begin.case {
    .shutdown! => !,
    .incoming next => next.loop,
}
";
        let live_source = source.replace("Listen.begin.case", "Listen.");
        assert_source_dot_completion_labels(
            source,
            &live_source,
            "Listen.",
            &["begin", "unfounded"],
            &["case", "shutdown", "incoming", "loop"],
        );

        let live_source = source.replace("Listen.begin.case", "Listen.begin.");
        assert_source_dot_completion_labels(
            source,
            &live_source,
            "Listen.begin.",
            &["case"],
            &["begin", "unfounded", "shutdown", "incoming", "loop"],
        );

        let live_source = source.replace("Listen.begin.case", "Listen.unfounded.");
        assert_source_dot_completion_labels(
            source,
            &live_source,
            "Listen.unfounded.",
            &["case"],
            &["begin", "unfounded", "shutdown", "incoming", "loop"],
        );
    }

    #[test]
    fn dot_completion_after_unfounded_omits_either_branch_labels() {
        let source = "\
module Main

type Items = recursive either {
    .end!,
    .item self,
}

def Work : [Items] ! = [input] chan exit {
    input.unfounded@outer
    input.case {
        .end! => {
            exit!
        }
        .item next => {
            next.loop@outer
        }
    }
}

def Main : ! = Work(.end!)
";
        let live_source = source.replace("input.case", "input.");
        assert_source_dot_completion_labels(
            source,
            &live_source,
            "input.",
            &["case"],
            &["begin", "unfounded", "end", "item", "loop"],
        );
    }

    #[test]
    fn dot_completion_inside_unfounded_labeled_loop_body_offers_loop() {
        let source = "\
module Main

type Items = recursive@outer either {
    .end!,
    .item self@outer,
}

def Work : [Items] ! = [input] chan exit {
    input.unfounded@outer
    input.case {
        .end! => {
            exit!
        }
        .item next => {
            next.loop@outer
        }
    }
}

def Main : ! = Work(.end!)
";
        let live_source = source.replace("next.loop@outer", "next.");
        assert_source_dot_completion_labels(
            source,
            &live_source,
            "next.",
            &["loop"],
            &["case", "end", "item"],
        );
    }

    #[test]
    fn dot_completion_inside_unfounded_labeled_loop_body_offers_labeled_loop_for_unlabeled_type() {
        let source = "\
module Main

type Items = recursive either {
    .end!,
    .item self,
}

def Work : [Items] ! = [input] chan exit {
    input.unfounded@outer
    input.case {
        .end! => {
            exit!
        }
        .item next => {
            next.loop@outer
        }
    }
}

def Main : ! = Work(.end!)
";
        let live_source = source.replace("next.loop@outer", "next.");
        let checked = checked_workspace_from_source(source);
        let completions = dot_completions_at_marker(&checked, &live_source, "next.");

        assert!(
            completions.iter().any(|candidate| {
                candidate.label == "loop@outer" && candidate.insert_text == "loop@outer"
            }),
            "missing completion loop@outer: {:?}",
            completions
        );
        assert!(
            completions
                .iter()
                .all(|candidate| !(candidate.label == "loop" && candidate.insert_text == "loop")),
            "unexpected bare loop completion: {:?}",
            completions
        );
    }

    #[test]
    fn dot_completion_in_nested_unlabeled_and_labeled_loops_omits_bare_loop() {
        let source = "\
module Main

type Items = recursive either {
    .end!,
    .item self,
}

def Work : [Items] ! = [outer] chan exit {
    outer.begin.case {
        .end! => {
            exit!
        }
        .item lines => {
            lines.begin@file.case {
                .end! => {
                    exit!
                }
                .item next => {
                    next.loop@file
                }
            }
        }
    }
}

def Main : ! = Work(.item .end!)
";
        let live_source = source.replace("next.loop@file", "next.");
        let checked = checked_workspace_from_source(source);
        let completions = dot_completions_at_marker(&checked, &live_source, "next.");

        assert!(
            completions.iter().any(|candidate| {
                candidate.label == "loop@file" && candidate.insert_text == "loop@file"
            }),
            "missing completion loop@file: {:?}",
            completions
        );
        assert!(
            completions
                .iter()
                .all(|candidate| !(candidate.label == "loop" && candidate.insert_text == "loop")),
            "unexpected bare loop completion: {:?}",
            completions
        );
    }

    #[test]
    fn dot_completion_for_iterative_choice_value_omits_loop() {
        let source = "\
module Main

type Console = iterative choice {
    .close => !,
    .print(!) => self,
    .prompt(!) => self,
}

def Open : Console = external
def Main : Console = Open
";
        let live_source = source.replace("Open\n", "Open.\n");
        assert_source_dot_completion_labels(
            source,
            &live_source,
            "Open.",
            &["close", "print", "prompt"],
            &["loop"],
        );
    }

    #[test]
    fn dot_completion_offers_either_keywords() {
        let try_source = "\
module Main

type Try = either {
    .ok!,
    .err!,
}

def TryValue : Try = external
def Main : ! = chan exit {
    catch err => { exit! }
    TryValue.try
    exit!
}
";
        let cases = [
            (
                "\
module Main

type Result = either {
    .ok!,
    .err!,
}

def ResultValue : Result = external
def Main : Result = ResultValue
",
                "ResultValue\n",
                "ResultValue.\n",
                "ResultValue.",
                &["try", "case"][..],
                &[][..],
            ),
            (
                "\
module Main

type Option = either {
    .some!,
    .none!,
}

def OptionValue : Option = external
def Main : ! = OptionValue.default(!)
",
                "OptionValue.default(!)",
                "OptionValue.",
                "OptionValue.",
                &["default", "case"][..],
                &[][..],
            ),
            (
                try_source,
                "TryValue.try",
                "TryValue.",
                "TryValue.",
                &["try", "case"][..],
                &[][..],
            ),
            (
                try_source,
                "TryValue.try",
                "TryValue.tr",
                "TryValue.tr",
                &["try", "case"][..],
                &[][..],
            ),
            (
                "\
module Main

type E = either {
    .a!,
    .b!,
}

def Main : E = .a!
",
                "= .a!",
                "= .",
                "= .",
                &["a", "b"][..],
                &[][..],
            ),
        ];

        for (source, before, after, marker, expected, unexpected) in cases {
            let live_source = source.replace(before, after);
            assert_source_dot_completion_labels(source, &live_source, marker, expected, unexpected);
        }
    }

    #[test]
    fn dot_completion_omits_candidates_for_unsupported_receiver_types() {
        let source = "\
module Main

dec FunctionValue : [!] !
def FunctionValue = external
def Main = 0
";
        let checked = checked_workspace_from_source(source);
        let file = checked.workspace().sources().keys().next().unwrap();

        let primitive_source = source.replace("0\n", "0.\n");
        let primitive_completions =
            dot_completions_at_marker_in_file(&checked, file, &primitive_source, "0.");
        assert!(primitive_completions.is_empty());

        let function_source = source.replace("0\n", "FunctionValue.\n");
        let function_completions =
            dot_completions_at_marker_in_file(&checked, file, &function_source, "FunctionValue.");
        assert!(function_completions.is_empty());
    }

    #[test]
    fn dot_completion_after_either_constructor_payload_offers_constructors() {
        let source = "\
module Main

type Pattern = recursive either {
    .empty!,
    .one!,
    .repeat self,
}

def Main : Pattern = .repeat.one!
";
        let live_source = source.replace(".repeat.one!", ".repeat.");
        assert_source_dot_completion_labels(
            source,
            &live_source,
            ".repeat.",
            &["empty", "one", "repeat"],
            &["begin", "case"],
        );
    }

    #[test]
    fn dot_completion_treats_minmax_parser_argument_constructors_as_construction_context() {
        let source = "\
module Main

type P = recursive either {
    .empty!,
    .one self,
    .repeat self,
}

dec Use : [P, P] !
def Use = external

def Main : ! = Use(.repeat.one.empty!, .empty!)
";

        for marker in ["Use(.", "Use(.repeat.", "Use(.repeat.one."] {
            let live_source = source.replace("Use(.repeat.one.empty!", marker);
            let checked = checked_workspace_from_source(source);
            let completions = dot_completions_at_marker(&checked, &live_source, marker);

            assert!(
                completions.iter().all(|candidate| {
                    !matches!(
                        candidate.label.as_str(),
                        "begin" | "unfounded" | "loop" | "case"
                    )
                }),
                "unexpected recursive/member completions for {marker:?}: {:?}",
                completions
            );
            assert!(
                completions.iter().any(|candidate| matches!(
                    candidate.label.as_str(),
                    "empty" | "one" | "repeat"
                )),
                "missing constructor-like completions for {marker:?}: {:?}",
                completions
            );
        }
    }

    #[test]
    fn dot_completion_after_call_with_constructor_argument_offers_either_keywords() {
        let source = "\
module Main

type E = either {
    .ok!,
    .err!,
}

dec Make : [!, E] E
def Make = external

def Main : E = Make(!, .ok!)
";
        let live_source = source.replace("Make(!, .ok!)", "Make(!, .ok!).");
        assert_source_dot_completion_labels(
            source,
            &live_source,
            "Make(!, .ok!).",
            &["try", "case"],
            &["ok", "err"],
        );
    }

    #[test]
    fn dot_completion_offers_module_members_for_import_aliases() {
        let main = "\
module Main

import Console

def Main : ! = Console.Open
";
        let parsed = parse_loaded_files(vec![
            LoadedPackageFile {
                name: FileName::from("local/Console.par"),
                relative_path_from_src: PathBuf::from("Console.par"),
                source: "\
export module Console

export {
    dec Open : !
}

def Open = external
"
                .to_owned(),
            },
            LoadedPackageFile {
                name: FileName::from("local/Main.par"),
                relative_path_from_src: PathBuf::from("Main.par"),
                source: main.to_owned(),
            },
        ])
        .unwrap();
        let (checked, type_errors) = assemble_workspace(WorkspacePackages {
            root_package: test_package_id(),
            packages: vec![WorkspacePackage::new(test_package_id(), parsed)],
        })
        .unwrap()
        .type_check();
        assert!(type_errors.is_empty(), "type errors: {:?}", type_errors);
        let file = FileName::from("local/Main.par");
        let live_source = main.replace("Console.Open", "Console.");
        let completions =
            dot_completions_at_marker_in_file(&checked, &file, &live_source, "Console.");
        assert!(
            completions
                .iter()
                .any(|candidate| candidate.label == "Open"),
            "missing completion label \"Open\""
        );
    }

    #[test]
    fn dot_completion_offers_module_members_for_import_aliases_inside_call_arguments() {
        let main = "\
module Main

import Console

dec Use : [!] !
def Use = external

def Main : ! = Use(Console.Open)
";
        let parsed = parse_loaded_files(vec![
            LoadedPackageFile {
                name: FileName::from("local/Console.par"),
                relative_path_from_src: PathBuf::from("Console.par"),
                source: "\
export module Console

export {
    dec Open : !
}

def Open = external
"
                .to_owned(),
            },
            LoadedPackageFile {
                name: FileName::from("local/Main.par"),
                relative_path_from_src: PathBuf::from("Main.par"),
                source: main.to_owned(),
            },
        ])
        .unwrap();
        let (checked, type_errors) = assemble_workspace(WorkspacePackages {
            root_package: test_package_id(),
            packages: vec![WorkspacePackage::new(test_package_id(), parsed)],
        })
        .unwrap()
        .type_check();
        assert!(type_errors.is_empty(), "type errors: {:?}", type_errors);
        let file = FileName::from("local/Main.par");
        let live_source = main.replace("Console.Open", "Use(Console.");
        let completions =
            dot_completions_at_marker_in_file(&checked, &file, &live_source, "Console.");
        assert!(
            completions
                .iter()
                .any(|candidate| candidate.label == "Open"),
            "missing completion label \"Open\""
        );
    }
    #[test]
    fn dot_completion_for_import_alias_omits_receiver_type_members() {
        let main = "\
module Main

import Os

dec ServePath : [Os.Path] !
def ServePath = external
dec Use : [!] Os.Path
def Use = [raw] raw->Os.Path
def Main = 0
";
        let parsed = parse_loaded_files(vec![
            LoadedPackageFile {
                name: FileName::from("local/Os.par"),
                relative_path_from_src: PathBuf::from("Os.par"),
                source: "\
export module Os

export {
    type Option<a> = either {
        .none!,
        .some a,
    }

    type Path = iterative@append recursive@parent box choice {
        .name => !,
        .absolute => !,
        .parent => Option<self@parent>,
        .append(!) => self@append,
    }

    dec Path : [!] Path
}

def Path = external
"
                .to_owned(),
            },
            LoadedPackageFile {
                name: FileName::from("local/Main.par"),
                relative_path_from_src: PathBuf::from("Main.par"),
                source: main.to_owned(),
            },
        ])
        .unwrap();
        let (checked, type_errors) = assemble_workspace(WorkspacePackages {
            root_package: test_package_id(),
            packages: vec![WorkspacePackage::new(test_package_id(), parsed)],
        })
        .unwrap()
        .type_check();
        assert!(type_errors.is_empty(), "type errors: {:?}", type_errors);
        let file = FileName::from("local/Main.par");
        for (before, after, marker) in [
            ("Os.Path", "Os.", "Os."),
            ("raw->Os.Path", "raw->Os.", "raw->Os."),
        ] {
            let live_source = main.replace(before, after);
            let completions =
                dot_completions_at_marker_in_file(&checked, &file, &live_source, marker);

            for label in ["Path"] {
                assert!(
                    completions.iter().any(|candidate| candidate.label == label),
                    "missing completion label {label:?}"
                );
            }

            for label in ["unfounded", "absolute", "append", "begin", "name", "parent"] {
                assert!(
                    completions.iter().all(|candidate| candidate.label != label),
                    "unexpected completion label {label:?}"
                );
            }
        }
    }

    #[test]
    fn dot_completion_dedups_module_alias_members_declared_and_defined() {
        let main = "\
module Main

import Console

def Main : ! = Console.Open
";
        let parsed = parse_loaded_files(vec![
            LoadedPackageFile {
                name: FileName::from("local/Console.par"),
                relative_path_from_src: PathBuf::from("Console.par"),
                source: "\
export module Console

export {
    dec Open : !
}

def Open = external
"
                .to_owned(),
            },
            LoadedPackageFile {
                name: FileName::from("local/Main.par"),
                relative_path_from_src: PathBuf::from("Main.par"),
                source: main.to_owned(),
            },
        ])
        .unwrap();
        let (checked, type_errors) = assemble_workspace(WorkspacePackages {
            root_package: test_package_id(),
            packages: vec![WorkspacePackage::new(test_package_id(), parsed)],
        })
        .unwrap()
        .type_check();
        assert!(type_errors.is_empty(), "type errors: {:?}", type_errors);
        let file = FileName::from("local/Main.par");
        let live_source = main.replace("Console.Open", "Console.");
        let completions =
            dot_completions_at_marker_in_file(&checked, &file, &live_source, "Console.");
        let open_count = completions
            .iter()
            .filter(|candidate| candidate.label == "Open")
            .count();

        assert_eq!(
            open_count, 1,
            "expected exactly one \"Open\" completion, got: {:?}",
            completions
        );
    }
}
