use super::CheckedWorkspace;
use crate::frontend_impl::language::{LocalName, Universal};
use crate::frontend_impl::types::Type;
use crate::location::FileName;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionCandidate {
    pub label: String,
    pub insert_text: String,
    pub detail: String,
    pub is_keyword: bool,
}

impl CompletionCandidate {
    fn branch(label: impl Into<String>, detail: impl Into<String>) -> Self {
        let label = label.into();
        Self {
            insert_text: label.clone(),
            label,
            detail: detail.into(),
            is_keyword: false,
        }
    }

    fn keyword(
        label: impl Into<String>,
        insert_text: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            label: label.into(),
            insert_text: insert_text.into(),
            detail: detail.into(),
            is_keyword: true,
        }
    }
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

        let mut candidates = Vec::new();
        self.push_module_alias_completion_candidates(file, source, dot, &mut candidates);

        if let Some(hover) = self.hover_at(file, hover_row, hover_column) {
            if let Some(typ) = hover.typ() {
                self.push_type_completion_candidates(typ, completion_context, &mut candidates);
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
    ) {
        let Some(alias) = module_alias_before_dot(source, dot) else {
            return;
        };
        let Some(scope) = self.workspace.import_scope(file) else {
            return;
        };
        let Some(module) = scope.aliases.get(alias) else {
            return;
        };

        let vis = &self.workspace.visibility;
        let from = &scope.current_module;

        for (name, _) in self.checked.type_defs.globals.iter() {
            if name.module == *module
                && !name.is_primary_export()
                && vis.type_visible_from(from, name)
            {
                candidates.push(CompletionCandidate::branch(
                    name.primary.clone(),
                    "module type",
                ));
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
                candidates.push(CompletionCandidate::branch(
                    name.primary.clone(),
                    "module declaration",
                ));
            }
        }
    }

    fn push_type_completion_candidates(
        &self,
        typ: &Type<Universal>,
        context: DotCompletionContext,
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
                push_branch_completion_candidates(branches, "either branch", candidates);
                let mut branch_names = branches.keys().map(|branch| branch.string.as_str());
                match (
                    branch_names.next(),
                    branch_names.next(),
                    branch_names.next(),
                ) {
                    (Some("err"), Some("ok"), None) => {
                        candidates.push(CompletionCandidate::keyword(
                            "try",
                            "try",
                            "propagate .err to the active catch and continue with .ok",
                        ));
                    }
                    (Some("none"), Some("some"), None) => {
                        candidates.push(CompletionCandidate::keyword(
                            "default",
                            "default(",
                            "use a default value for .none and continue with .some",
                        ));
                    }
                    _ => {}
                }
                candidates.push(CompletionCandidate::keyword(
                    "case",
                    "case {\n  ",
                    "case on either branches",
                ));
            }
            Type::Recursive {
                asc, label, body, ..
            } => {
                if context == DotCompletionContext::Normal {
                    candidates.push(recursive_keyword_completion(
                        label.as_ref(),
                        "begin",
                        "begin recursive session",
                    ));
                    candidates.push(recursive_keyword_completion(
                        label.as_ref(),
                        "unfounded",
                        "begin recursive session without totality checking",
                    ));
                    // If there is a begin in the current context, then we can offer a loop to it.
                    if !asc.is_empty() {
                        candidates.push(recursive_keyword_completion(
                            label.as_ref(),
                            "loop",
                            "loop to the matching begin",
                        ));
                    }
                }
                if let Ok(expanded) =
                    Type::expand_recursive(&Default::default(), label, body, typ.display_hint())
                {
                    self.push_type_completion_candidates(
                        &expanded,
                        context.descend_into_body(),
                        candidates,
                    );
                }
            }
            Type::Iterative { body, .. } => {
                self.push_type_completion_candidates(body, context.descend_into_body(), candidates);
            }
            Type::Box(_, inner) | Type::DualBox(_, inner) => {
                self.push_type_completion_candidates(inner, context, candidates);
            }
            Type::Name(..) | Type::DualName(..) => {
                if let Ok(expanded) = typ.expand_definition(&self.checked.type_defs) {
                    self.push_type_completion_candidates(&expanded, context, candidates);
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
        candidates.push(CompletionCandidate::branch(branch.to_string(), detail));
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DotCompletionContext {
    Normal,
    AfterBegin,
    Construction,
}

impl DotCompletionContext {
    /// Infers the completion context from the receiver text before `dot`.
    ///
    /// Distinguishes constructor-style completion, post-`begin` completion,
    /// and normal member completion.
    fn before_dot(source: &str, dot: usize) -> Option<Self> {
        let receiver = source.get(..dot)?.trim_end();
        let tail = receiver.split_whitespace().next_back()?;

        if tail.starts_with('.') || !tail.chars().any(is_completion_suffix_char) {
            return Some(Self::Construction);
        }

        let last = tail.rsplit_once('.').map_or(tail, |(_, seg)| seg);
        Some(if last == "begin" || last.starts_with("begin@") {
            Self::AfterBegin
        } else {
            Self::Normal
        })
    }

    fn descend_into_body(self) -> Self {
        match self {
            Self::Construction => Self::Construction,
            Self::Normal | Self::AfterBegin => Self::Normal,
        }
    }
}

// Build a keyword completion where displayed text stays the keyword,
// but inserted text includes the recursion label when present.
// Example: keyword "begin" + label "items" inserts "begin@items".
// Example: keyword "begin" + no label inserts "begin".
fn recursive_keyword_completion(
    recursion_label: Option<&LocalName>,
    keyword: &'static str,
    detail: &'static str,
) -> CompletionCandidate {
    let insert_text =
        recursion_label.map_or_else(|| keyword.to_string(), |label| format!("{keyword}@{label}"));
    CompletionCandidate::keyword(keyword, insert_text, detail)
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

/// Converts a zero-based `(row, column)` position in `source` to a byte `offset`.
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
            current_column += 1;
        }
    }
    (current_row == row && current_column == column).then_some(source.len())
}

/// Converts a byte `offset` in `source` to zero-based `(row, column)`.
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
            column += 1;
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
            &["begin", "unfounded", "case"],
            &["loop"],
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
                &["begin", "unfounded", "case"][..],
                &["loop"][..],
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
                &["ok", "err", "case"][..],
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
