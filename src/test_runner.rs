use std::collections::BTreeMap;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use colored::Colorize;
use par_core::runtime::TranspiledGlobal;
use par_core::{
    frontend::{
        Type,
        language::{GlobalName, Universal},
        set_miette_hook,
    },
    runtime::{Compiled, RuntimeCompilerError},
    source::Span,
    testing::{AssertionResult, provide_test},
    workspace::{CheckedWorkspace, ModulePath, WorkspaceDiscoveryError, WorkspaceError},
};
use par_runtime::linker::Linked;
use par_runtime::pkgid::{BuiltinPackage, PackageId};
use par_runtime::spawn::TokioSpawn;

use crate::package_utils::{
    SourceLookup, find_local_module, parse_target, root_module_slash_path, source_for_fallback,
};
use crate::workspace_support::{ScopedTypeError, WorkspaceBuildError, checked_workspace_from_path};

#[derive(Clone)]
enum BuildError {
    Discovery(WorkspaceDiscoveryError),
    Workspace(WorkspaceError),
    Type {
        errors: Vec<ScopedTypeError>,
        sources: SourceLookup,
    },
    InetCompile {
        error: RuntimeCompilerError,
        sources: SourceLookup,
    },
}

impl BuildError {
    fn display(&self) -> String {
        match self {
            Self::Discovery(error) => error.to_string(),
            Self::Workspace(error) => error.to_string(),
            Self::Type { errors, sources } => errors
                .iter()
                .map(|error| format!("{:?}", error.to_report(sources)))
                .collect::<Vec<_>>()
                .join("\n"),
            Self::InetCompile { error, sources } => format!(
                "inet compilation error: {}",
                error.display(&source_for_fallback(sources))
            ),
        }
    }
}

fn build_for_run(
    package_path: &Path,
    max_interactions: u32,
) -> Result<(CheckedWorkspace, Compiled<Linked>, Vec<ModulePath>), BuildError> {
    let build =
        checked_workspace_from_path(package_path, None).map_err(map_workspace_build_error)?;
    if build.type_errors.iter().any(|e| !e.error.is_warning()) {
        return Err(BuildError::Type {
            errors: build.type_errors,
            sources: build.sources.clone(),
        });
    }
    let sources = build.sources.clone();
    let (checked, rt_compiled, _) =
        build
            .compile_linked(max_interactions)
            .map_err(|(_, error)| BuildError::InetCompile {
                error,
                sources: sources.clone(),
            })?;
    let local_modules = checked.workspace().root_modules();
    Ok((checked, rt_compiled, local_modules))
}

fn map_workspace_build_error(error: WorkspaceBuildError) -> BuildError {
    match error {
        WorkspaceBuildError::Discovery(error) => BuildError::Discovery(error),
        WorkspaceBuildError::Workspace(error) => BuildError::Workspace(error),
    }
}

#[derive(Debug)]
pub enum TestStatus {
    Passed,
    PassedWithNoAssertions,
    Failed(String),
    FailedWithAssertions(Vec<AssertionResult>),
}

impl TestStatus {
    pub fn is_passed(&self) -> bool {
        matches!(
            self,
            TestStatus::Passed | TestStatus::PassedWithNoAssertions
        )
    }
}

impl Display for TestStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TestStatus::Passed => write!(f, "passed"),
            TestStatus::PassedWithNoAssertions => write!(f, "passed with no assertions"),
            TestStatus::Failed(msg) => write!(f, "failed: {msg}"),
            TestStatus::FailedWithAssertions(v) => {
                write!(f, "failed with {} assertion(s)", v.len())
            }
        }
    }
}

#[derive(Debug)]
pub struct TestResult {
    name: String,
    duration: Duration,
    pub status: TestStatus,
}

pub fn run_tests(
    package_path: PathBuf,
    target: Option<String>,
    filter: Option<String>,
    max_interactions: u32,
) -> bool {
    set_miette_hook();
    println!(
        "{} {}",
        "Running tests in package:".bright_blue(),
        package_path.display()
    );
    println!();

    let (checked, rt_compiled, local_modules) = match build_for_run(&package_path, max_interactions)
    {
        Ok(result) => result,
        Err(error) => {
            eprintln!("{}", error.display().bright_red());
            return false;
        }
    };

    let parsed_target = target.as_deref().map(parse_target);
    let module_selector = parsed_target
        .as_ref()
        .map(|parsed| parsed.module_path.as_str());
    let name_selector = parsed_target
        .as_ref()
        .and_then(|parsed| parsed.definition_name.as_deref());
    let selected_module = module_selector.as_deref().and_then(|selector| {
        find_local_module(selector, &local_modules).map(ModulePath::to_slash_path)
    });

    if module_selector.is_some() && selected_module.is_none() {
        eprintln!(
            "{} {}",
            "No such local module for test target:".bright_red(),
            target.unwrap_or_default()
        );
        return false;
    }

    let selected_module = selected_module.as_deref();
    let selected_name = name_selector.as_deref();

    let mut grouped_results: BTreeMap<String, Vec<TestResult>> = BTreeMap::new();
    let mut total_tests = 0usize;
    let mut passed_tests = 0usize;
    let start_time = Instant::now();

    let tests = collect_test_definitions(
        &checked,
        &local_modules,
        selected_module,
        selected_name,
        filter.as_deref(),
    );

    if tests.is_empty() {
        println!("{}", "No test definitions found".yellow());
        return false;
    }

    for (name, kind) in tests {
        let result = match kind {
            DefinitionKind::Test => test_single_definition(&checked, &rt_compiled, &name),
            DefinitionKind::Run => run_single_definition(&checked, &rt_compiled, &name),
        };
        total_tests += 1;
        if result.status.is_passed() {
            passed_tests += 1;
        }
        let module = root_module_slash_path(checked.workspace().root_package(), &name.module)
            .unwrap_or_else(|| "<unknown>".to_string());
        grouped_results.entry(module).or_default().push(result);
    }

    for (module, results) in &grouped_results {
        print_test_results(module, results);
    }

    let duration = start_time.elapsed();
    println!();
    print_summary(total_tests, passed_tests, duration);
    passed_tests == total_tests
}

#[derive(Debug, Clone, Copy)]
enum DefinitionKind {
    Test,
    Run,
}

fn collect_test_definitions(
    checked: &CheckedWorkspace,
    local_modules: &[ModulePath],
    selected_module: Option<&str>,
    selected_name: Option<&str>,
    filter: Option<&str>,
) -> Vec<(GlobalName<Universal>, DefinitionKind)> {
    checked
        .checked_module()
        .definitions
        .iter()
        .filter_map(|(name, _)| {
            let module = root_module_slash_path(checked.workspace().root_package(), &name.module)?;
            if !is_local_module(module.as_str(), local_modules) {
                return None;
            }

            if let Some(selected_module) = selected_module
                && module != selected_module
            {
                return None;
            }
            if let Some(selected_name) = selected_name
                && name.primary != selected_name
            {
                return None;
            }
            if let Some(filter) = filter
                && !name.primary.contains(filter)
            {
                return None;
            }

            if name.primary.starts_with("Test") && name.primary != "Test" {
                return Some((name.clone(), DefinitionKind::Test));
            }
            if name.primary.starts_with("Run") && name.primary != "Run" {
                return Some((name.clone(), DefinitionKind::Run));
            }
            None
        })
        .collect()
}

fn is_local_module(module: &str, local_modules: &[ModulePath]) -> bool {
    local_modules
        .iter()
        .any(|candidate| candidate.to_slash_path() == module)
}

fn test_single_definition(
    program: &CheckedWorkspace,
    rt_compiled: &Compiled<Linked>,
    test_name: &GlobalName<Universal>,
) -> TestResult {
    let start = Instant::now();
    let name_label = test_name.to_string();
    let missing_type_name = name_label.clone();
    let runtime = match crate::tokio_factory::create_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            return TestResult {
                name: name_label,
                duration: start.elapsed(),
                status: TestStatus::Failed(format!("Failed to create runtime: {}", e)),
            };
        }
    };

    let result = runtime.block_on(async {
        let ty = rt_compiled
            .get_type_of(test_name)
            .ok_or_else(|| format!("Type not found for test '{}'", missing_type_name))?;
        require_assignable_type(program, &ty, &test_type(), "[Test] !")?;
        run_test(rt_compiled, test_name).await
    });

    let duration = start.elapsed();
    let final_result = match result {
        Ok(status) => status,
        Err(msg) => TestStatus::Failed(msg),
    };

    TestResult {
        name: name_label,
        duration,
        status: final_result,
    }
}

fn run_single_definition(
    program: &CheckedWorkspace,
    rt_compiled: &Compiled<Linked>,
    run_name: &GlobalName<Universal>,
) -> TestResult {
    let start = Instant::now();
    let name_label = run_name.to_string();
    let missing_type_name = name_label.clone();
    let runtime = match crate::tokio_factory::create_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            return TestResult {
                name: name_label,
                duration: start.elapsed(),
                status: TestStatus::Failed(format!("Failed to create runtime: {}", e)),
            };
        }
    };

    let result = runtime.block_on(async {
        let ty = rt_compiled
            .get_type_of(run_name)
            .ok_or_else(|| format!("Type not found for test '{}'", missing_type_name))?;
        require_assignable_type(program, &ty, &Type::Break(Span::None), "!")?;
        let package = match rt_compiled.code.name_to_package.get(run_name) {
            Some(TranspiledGlobal::Package(pkg)) => pkg.clone(),
            Some(TranspiledGlobal::Unimplemented) => {
                return Err(format!("Test '{missing_type_name}' is incomplete (directly or indirectly contains a todo; run `par check` for details)"));
            }
            None => return Err(format!("Test package not found for '{missing_type_name}'")),
        };

        let (handle, fut) = par_runtime::start_and_instantiate(
            Arc::new(TokioSpawn::new()),
            rt_compiled.code.arena.clone(),
            package,
        );
        handle.continue_();
        fut.await;
        Ok(TestStatus::PassedWithNoAssertions)
    });

    let duration = start.elapsed();
    let final_result = match result {
        Ok(status) => status,
        Err(msg) => TestStatus::Failed(msg),
    };

    TestResult {
        name: name_label,
        duration,
        status: final_result,
    }
}

fn test_type() -> Type<Universal> {
    let test_name = GlobalName::new(
        Span::None,
        Universal {
            package: PackageId::Builtin(BuiltinPackage::Core),
            directories: vec![],
            module: String::from("Test"),
        },
        String::from("Test"),
    );
    Type::Function(
        Span::None,
        Box::new(Type::Name(Span::None, test_name, vec![])),
        Box::new(Type::Break(Span::None)),
        vec![],
    )
}

fn require_assignable_type(
    program: &CheckedWorkspace,
    actual: &Type<Universal>,
    expected: &Type<Universal>,
    expected_name: &str,
) -> Result<(), String> {
    let assignable = actual
        .is_definitely_assignable_to(expected, &program.checked_module().type_defs)
        .map_err(|error| format!("Failed to check definition type: {error:?}"))?
        .is_assignable();
    if assignable {
        Ok(())
    } else {
        Err(format!(
            "Definition does not have the expected {expected_name} type"
        ))
    }
}

async fn run_test(
    rt_compiled: &Compiled<Linked>,
    name: &GlobalName<Universal>,
) -> Result<TestStatus, String> {
    let (sender, receiver) = mpsc::channel();

    let package = match rt_compiled.code.name_to_package.get(name) {
        Some(TranspiledGlobal::Package(pkg)) => pkg.clone(),
        Some(TranspiledGlobal::Unimplemented) => {
            return Err("Test is incomplete (directly or indirectly contains a todo; run `par check` for details)".to_string());
        }
        None => return Err("Test package not found".to_string()),
    };
    let (mut root, reducer_future) = par_runtime::start_and_instantiate(
        Arc::new(TokioSpawn::new()),
        rt_compiled.code.arena.clone(),
        package,
    );

    let test_handle = root.send();
    provide_test(test_handle, sender).await;
    root.continue_();
    reducer_future.await;

    let mut results = vec![];
    while let Ok(result) = receiver.try_recv() {
        results.push(result);
    }
    let failed_assertions: Vec<_> = results.iter().filter(|r| !r.passed).cloned().collect();

    if failed_assertions.is_empty() && !results.is_empty() {
        Ok(TestStatus::Passed)
    } else if !failed_assertions.is_empty() {
        Ok(TestStatus::FailedWithAssertions(failed_assertions))
    } else {
        Ok(TestStatus::PassedWithNoAssertions)
    }
}

const PASSED: &str = "[PASS]";
const FAILED: &str = "[FAIL]";

fn print_test_results(module: &str, results: &[TestResult]) {
    let all_passed = results.iter().all(|r| r.status.is_passed());
    let icon = if all_passed {
        PASSED.green()
    } else {
        FAILED.red()
    };

    println!("{} {}", icon, module.bright_white());

    for r in results {
        let icon = if r.status.is_passed() {
            PASSED.green()
        } else {
            FAILED.red()
        };
        let duration = format!("({:.3}s)", r.duration.as_secs_f32()).dimmed();
        println!("  {} {} {}", icon, r.name, duration);

        match &r.status {
            TestStatus::Failed(msg) => {
                println!("    {}", msg.red());
            }
            TestStatus::FailedWithAssertions(assertions) => {
                for a in assertions {
                    println!(
                        "    {} {}: {}",
                        FAILED.red(),
                        a.description,
                        "assertion failed".red()
                    );
                }
            }
            TestStatus::PassedWithNoAssertions => {}
            TestStatus::Passed => {}
        }
    }
}

fn print_summary(total: usize, passed: usize, duration: std::time::Duration) {
    let failed = total - passed;

    let summary = if failed == 0 {
        format!(
            "Summary: {} passed ({:.3}s)",
            passed,
            duration.as_secs_f32()
        )
        .green()
    } else {
        format!(
            "Summary: {} passed, {} failed ({:.3}s)",
            passed,
            failed,
            duration.as_secs_f32()
        )
        .red()
    };

    println!("{}", summary);
}
