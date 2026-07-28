use crate::package_manager::AddedDependencyStatus;
use crate::package_utils::{
    SourceLookup, find_local_module, parse_target, root_module_slash_path, source_for_fallback,
};
#[cfg(feature = "playground")]
use crate::playground::Playground;
use crate::workspace_support::{
    CheckedWorkspaceBuild, ScopedTypeError, WorkspaceBuildError, checked_workspace_from_path,
};
use clap::{Command, arg, command, value_parser};
use colored::Colorize;
#[cfg(feature = "playground")]
use eframe::egui;
use par_core::runtime::TranspiledGlobal;
use par_core::{
    frontend::{Type, is_lowercase_identifier, set_miette_hook},
    runtime::RuntimeCompilerError,
    workspace::{CheckedWorkspace, ModulePath, WorkspaceDiscoveryError, WorkspaceError},
};
use par_doc::DocOptions;
use tokio::time::Instant;
#[cfg(not(target_family = "wasm"))]
use url::Url;

use par_runtime::linker::{Artifact, ArtifactGlobal, Linked, Unlinked};
use par_runtime::spawn::TokioSpawn;
use std::fmt::Display;
use std::fs::{self, File};
#[cfg(feature = "playground")]
use std::io::Write;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

#[cfg(not(target_arch = "wasm32"))]
use mimalloc::MiMalloc;

#[cfg(not(target_arch = "wasm32"))]
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[cfg(not(target_family = "wasm"))]
mod language_server;
mod package_manager;
mod package_utils;
#[cfg(feature = "playground")]
mod playground;
mod test;
mod test_runner;
mod tokio_factory;
#[cfg(target_family = "wasm")]
mod wasm_spawn;
mod workspace_support;

const MAX_INTERACTIONS_DEFAULT: u32 = 10_000;
const HELLO_WORLD_SOURCE: &str = "\
module Main

import @core/Debug

def Main : ! = Debug.Log(\"Hello, World!\")
";

#[derive(Debug, Clone, PartialEq, Eq)]
enum DocSource {
    Local(PathBuf),
    Remote(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DocCommandPlan {
    source: DocSource,
    out_dir: PathBuf,
    temporary_out_dir: bool,
    only_exported: bool,
    open: bool,
}

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
            Self::Discovery(error) => error.to_string().bright_red().to_string(),
            Self::Workspace(error) => error.to_string().bright_red().to_string(),
            Self::Type { errors, sources } => errors
                .iter()
                .map(|error| {
                    let report = format!("{:?}", error.to_report(sources));
                    if error.error.is_warning() {
                        report.yellow().to_string()
                    } else {
                        report.bright_red().to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Self::InetCompile { error, sources } => format!(
                "inet compilation error: {}",
                error.display(&source_for_fallback(sources))
            )
            .bright_red()
            .to_string(),
        }
    }
}

impl Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display())
    }
}

#[derive(Debug)]
enum NewPackageError {
    InvalidPackageName(String),
    CurrentDirectory(String),
    PathExistsAsFile(PathBuf),
    DirectoryRead { path: PathBuf, message: String },
    DirectoryNotEmpty(PathBuf),
    DirectoryCreate { path: PathBuf, message: String },
    FileWrite { path: PathBuf, message: String },
}

impl Display for NewPackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPackageName(name) => write!(
                f,
                "`{name}` is not a valid package name. Package names must be lower-case identifiers and not reserved keywords."
            ),
            Self::CurrentDirectory(message) => {
                write!(f, "Failed to determine current directory: {message}")
            }
            Self::PathExistsAsFile(path) => {
                write!(
                    f,
                    "Path already exists and is not a directory: {}",
                    path.display()
                )
            }
            Self::DirectoryRead { path, message } => {
                write!(f, "Failed to read directory {}: {message}", path.display())
            }
            Self::DirectoryNotEmpty(path) => {
                write!(
                    f,
                    "Directory already exists and is not empty: {}",
                    path.display()
                )
            }
            Self::DirectoryCreate { path, message } => {
                write!(
                    f,
                    "Failed to create directory {}: {message}",
                    path.display()
                )
            }
            Self::FileWrite { path, message } => {
                write!(f, "Failed to write file {}: {message}", path.display())
            }
        }
    }
}

fn build_checked_package(package_path: &PathBuf) -> Result<CheckedWorkspaceBuild, BuildError> {
    let build =
        checked_workspace_from_path(package_path, None).map_err(map_workspace_build_error)?;
    if build.type_errors.iter().any(|e| !e.error.is_warning()) {
        return Err(BuildError::Type {
            errors: build.type_errors,
            sources: build.sources.clone(),
        });
    }
    for warning in build.warnings() {
        eprintln!(
            "{}",
            format!("{:?}", warning.to_report(&build.sources)).yellow()
        );
    }
    Ok(build)
}

fn build_unlinked_package(
    package_path: &PathBuf,
    max_interactions: u32,
) -> Result<
    (
        CheckedWorkspace,
        par_core::runtime::Compiled<Unlinked>,
        Vec<ModulePath>,
        SourceLookup,
    ),
    BuildError,
> {
    let build = build_checked_package(package_path)?;
    let sources = build.sources.clone();
    let (checked, rt_compiled, sources) =
        build
            .compile_unlinked(max_interactions)
            .map_err(|(_, error)| BuildError::InetCompile {
                error,
                sources: sources.clone(),
            })?;
    let local_modules = checked.workspace().root_modules();
    Ok((checked, rt_compiled, local_modules, sources))
}

fn build_runtime_package(
    package_path: &PathBuf,
    max_interactions: u32,
) -> Result<
    (
        CheckedWorkspace,
        par_core::runtime::Compiled<Linked>,
        Vec<ModulePath>,
    ),
    BuildError,
> {
    let (checked, rt_compiled, local_modules, sources) =
        build_unlinked_package(package_path, max_interactions)?;
    Ok((
        checked,
        rt_compiled
            .link()
            .map_err(|error| BuildError::InetCompile {
                error,
                sources: sources.clone(),
            })?,
        local_modules,
    ))
}

fn map_workspace_build_error(error: WorkspaceBuildError) -> BuildError {
    match error {
        WorkspaceBuildError::Discovery(error) => BuildError::Discovery(error),
        WorkspaceBuildError::Workspace(error) => BuildError::Workspace(error),
    }
}

fn create_new_package(package_name: &str) -> Result<PathBuf, NewPackageError> {
    let current_dir = std::env::current_dir()
        .map_err(|error| NewPackageError::CurrentDirectory(error.to_string()))?;
    create_new_package_in(&current_dir, package_name)
}

fn create_new_package_in(base_dir: &Path, package_name: &str) -> Result<PathBuf, NewPackageError> {
    if !is_lowercase_identifier(package_name) {
        return Err(NewPackageError::InvalidPackageName(package_name.to_owned()));
    }

    let package_dir = base_dir.join(package_name);
    if package_dir.exists() {
        if !package_dir.is_dir() {
            return Err(NewPackageError::PathExistsAsFile(package_dir));
        }
        let mut entries =
            fs::read_dir(&package_dir).map_err(|error| NewPackageError::DirectoryRead {
                path: package_dir.clone(),
                message: error.to_string(),
            })?;
        if entries
            .next()
            .transpose()
            .map_err(|error| NewPackageError::DirectoryRead {
                path: package_dir.clone(),
                message: error.to_string(),
            })?
            .is_some()
        {
            return Err(NewPackageError::DirectoryNotEmpty(package_dir));
        }
    } else {
        fs::create_dir(&package_dir).map_err(|error| NewPackageError::DirectoryCreate {
            path: package_dir.clone(),
            message: error.to_string(),
        })?;
    }

    let manifest_path = package_dir.join("Par.toml");
    let manifest_source = format!("[package]\nname = \"{package_name}\"\n");
    fs::write(&manifest_path, manifest_source).map_err(|error| NewPackageError::FileWrite {
        path: manifest_path,
        message: error.to_string(),
    })?;

    let src_dir = package_dir.join("src");
    fs::create_dir(&src_dir).map_err(|error| NewPackageError::DirectoryCreate {
        path: src_dir.clone(),
        message: error.to_string(),
    })?;

    let main_path = src_dir.join("Main.par");
    fs::write(&main_path, HELLO_WORLD_SOURCE).map_err(|error| NewPackageError::FileWrite {
        path: main_path,
        message: error.to_string(),
    })?;

    Ok(package_dir)
}

#[cfg(not(target_family = "wasm"))]
fn main() -> ExitCode {
    let matches = command!()
        .subcommand_required(true)
        .subcommand(
            Command::new("new")
                .about("Create a new Par package")
                .arg(arg!(<package> "Package name to create")),
        )
        .subcommand(
            Command::new("playground")
                .about(if cfg!(feature = "playground") {
                    "Start the Par playground"
                } else {
                    "Disabled in build"
                })
                .arg(
                    arg!([file] "Open a Par file in the playground")
                        .value_parser(value_parser!(PathBuf)),
                )
                .arg(arg!(--max_interactions <MAX_INTERACTIONS> ... "Maximum number of interactions during compilation")
            .value_parser(value_parser!(u32))),
        )
        .subcommand(
            Command::new("add")
                .about("Fetch remote dependencies for a Par package")
                .arg(
                    arg!(--package <PACKAGE> "Path to package directory (or any file/directory inside it)")
                        .value_parser(value_parser!(PathBuf))
                        .default_value("."),
                )
                .arg(arg!([source] "Remote dependency source to add, such as `github.com/user/repo`")),
        )
        .subcommand(
            Command::new("run")
                .about("Run a definition in a Par package")
                .arg(arg!(--stats "Print statistics after running the definition"))
                .arg(
                    arg!(--package <PACKAGE> "Path to package directory (or any file/directory inside it)")
                        .value_parser(value_parser!(PathBuf))
                        .default_value("."),
                )
                .arg(arg!([target] "Target to run: `path/to/Module` or `path/to/Module.Def`"))
                .arg(arg!(-f --flag <FLAG> ... "Set a flag"))
                .arg(arg!(--max_interactions <MAX_INTERACTIONS> ... "Maximum number of interactions during compilation")
            .value_parser(value_parser!(u32))),
        )
        .subcommand(
            Command::new("check")
                .about("Type check a Par package in the CLI")
                .arg(
                    arg!(--package <PACKAGE> "Path to package directory (or any file/directory inside it)")
                        .value_parser(value_parser!(PathBuf))
                        .default_value("."),
                )
                .arg(arg!(-f --flag <FLAG> ... "Set a flag")),
        )
        .subcommand(
            Command::new("doc")
                .about("Generate HTML documentation for a Par package and its dependencies")
                .arg(
                    arg!(--package <PACKAGE> "Path to a local package directory (or any file/directory inside it)")
                        .value_parser(value_parser!(PathBuf))
                        .conflicts_with("remote"),
                )
                .arg(
                    arg!(--remote <REMOTE> "Remote package source to fetch and document")
                        .conflicts_with("package"),
                )
                .arg(
                    arg!(--out <OUT> "Directory where generated HTML documentation will be written")
                        .value_parser(value_parser!(PathBuf)),
                )
                .arg(
                    arg!(--exported "Show only exported modules and items of the target package")
                        .conflicts_with("unexported"),
                )
                .arg(
                    arg!(--unexported "Include unexported modules and items of the target package")
                        .conflicts_with("exported"),
                )
                .arg(arg!(--open "Open the generated documentation in a browser")),
        )
        .subcommand(
            Command::new("compile")
                .about("Compile a Par package")
                .arg(
                    arg!(--package <PACKAGE> "Path to package directory (or any file/directory inside it)")
                        .value_parser(value_parser!(PathBuf))
                        .default_value("."),
                )
                .arg(arg!(-f --flag <FLAG> ... "Set a flag"))
                .arg(arg!(--max_interactions <MAX_INTERACTIONS> ... "Maximum number of interactions during compilation")
                    .value_parser(value_parser!(u32))),
        )
        .subcommand(
            Command::new("run-vm")
                .about("Run a definition in a Par package")
                .arg(arg!(--stats "Print statistics after running the definition"))
                .arg(
                    arg!(--file <FILE> "Path to .pvm file")
                        .value_parser(value_parser!(PathBuf))
                        .default_value("./compiled.pvm"),
                )
                .arg(arg!([target] "Target to run: `path/to/Module` or `path/to/Module.Def`"))
                .arg(arg!(-f --flag <FLAG> ... "Set a flag")),
        )
        .subcommand(
            Command::new("lsp")
                .about("Start the Par language server for editor integration")
                .arg(arg!(--stdio "Run lsp over stdio (default behavior)")),
        )
        .subcommand(
            Command::new("update")
                .about("Refetch all remote dependencies for a Par package")
                .arg(
                    arg!(--package <PACKAGE> "Path to package directory (or any file/directory inside it)")
                        .value_parser(value_parser!(PathBuf))
                        .default_value("."),
                ),
        )
        .subcommand(
            Command::new("test")
                .about("Run Par tests")
                .arg(
                    arg!(--package <PACKAGE> "Path to package directory (or any file/directory inside it)")
                        .value_parser(value_parser!(PathBuf))
                        .default_value("."),
                )
                .arg(arg!([target] "Test target: `path/to/Module` or `path/to/Module.TestName`"))
                .arg(arg!(--filter <FILTER> "Only run tests matching this filter").required(false))
                .arg(arg!(-f --flag <FLAG> ... "Set a flag"))
                .arg(arg!(--max_interactions <MAX_INTERACTIONS> ... "Maximum number of interactions during compilation")
            .value_parser(value_parser!(u32))),
        )
        .get_matches_from(wild::args());

    match matches.subcommand() {
        Some(("new", args)) => {
            let package = args.get_one::<String>("package").unwrap();
            match create_new_package(package) {
                Ok(path) => {
                    println!("{} {}", "Created package:".bright_green(), path.display());
                }
                Err(error) => {
                    eprintln!("{}", error.to_string().bright_red());
                    return ExitCode::FAILURE;
                }
            }
        }
        Some(("playground", args)) => {
            let file = args.get_one::<PathBuf>("file");
            let max_interactions = args
                .get_one::<u32>("max_interactions")
                .cloned()
                .unwrap_or(MAX_INTERACTIONS_DEFAULT);
            run_playground(file.cloned(), max_interactions);
        }
        Some(("add", args)) => {
            let package = args.get_one::<PathBuf>("package").unwrap().clone();
            let source = args.get_one::<String>("source").cloned();
            if add_dependencies(package, source).is_err() {
                return ExitCode::FAILURE;
            }
        }
        Some(("run", args)) => {
            let stats = *args.get_one::<bool>("stats").unwrap();
            let package = args.get_one::<PathBuf>("package").unwrap().clone();
            let target = args.get_one::<String>("target").cloned();
            let max_interactions = args
                .get_one::<u32>("max_interactions")
                .cloned()
                .unwrap_or(MAX_INTERACTIONS_DEFAULT);
            run_definition(package, target, stats, max_interactions);
        }
        Some(("compile", args)) => {
            let package = args.get_one::<PathBuf>("package").unwrap().clone();
            let max_interactions = args
                .get_one::<u32>("max_interactions")
                .cloned()
                .unwrap_or(MAX_INTERACTIONS_DEFAULT);
            compile(package, max_interactions);
        }
        Some(("run-vm", args)) => {
            let stats = *args.get_one::<bool>("stats").unwrap();
            let file = args.get_one::<PathBuf>("file").unwrap().clone();
            let target = args.get_one::<String>("target").cloned();
            run_definition_vm(file, target, stats);
        }
        Some(("check", args)) => {
            let package = args.get_one::<PathBuf>("package").unwrap().clone();
            if check(package).is_err() {
                return ExitCode::FAILURE;
            }
        }
        Some(("doc", args)) => {
            let package = args.get_one::<PathBuf>("package").cloned();
            let remote = args.get_one::<String>("remote").cloned();
            let out_dir = args.get_one::<PathBuf>("out").cloned();
            let exported = *args.get_one::<bool>("exported").unwrap();
            let unexported = *args.get_one::<bool>("unexported").unwrap();
            let open = *args.get_one::<bool>("open").unwrap();
            let plan = match plan_doc_command(package, remote, out_dir, exported, unexported, open)
            {
                Ok(plan) => plan,
                Err(error) => {
                    eprintln!("{}", error.bright_red());
                    return ExitCode::FAILURE;
                }
            };
            if let Err(error) = generate_docs(plan) {
                eprintln!("{}", error.bright_red());
                return ExitCode::FAILURE;
            }
        }
        Some(("lsp", _)) => run_language_server(),
        Some(("update", args)) => {
            let package = args.get_one::<PathBuf>("package").unwrap().clone();
            if update_dependencies(package).is_err() {
                return ExitCode::FAILURE;
            }
        }
        Some(("test", args)) => {
            let package = args.get_one::<PathBuf>("package").unwrap().clone();
            let target = args.get_one::<String>("target").cloned();
            let filter = args.get_one::<String>("filter");
            let max_interactions = args
                .get_one::<u32>("max_interactions")
                .cloned()
                .unwrap_or(MAX_INTERACTIONS_DEFAULT);
            if !run_tests(package, target, filter.cloned(), max_interactions) {
                return ExitCode::FAILURE;
            }
        }
        _ => unreachable!(),
    }

    ExitCode::SUCCESS
}

#[cfg(target_family = "wasm")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;

    // Redirect `log` message to `console.log` and friends:
    eframe::WebLogger::init(log::LevelFilter::Debug).ok();

    let web_options = eframe::WebOptions::default();

    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .expect("No window")
            .document()
            .expect("No document");

        let canvas = document
            .get_element_by_id("the_canvas_id")
            .expect("Failed to find the_canvas_id")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("the_canvas_id was not a HtmlCanvasElement");

        let file = None;

        let start_result = eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| Ok(Playground::new(cc, file, MAX_INTERACTIONS_DEFAULT))),
            )
            .await;

        // Remove the loading text and spinner:
        if let Some(loading_text) = document.get_element_by_id("loading_text") {
            match start_result {
                Ok(_) => {
                    loading_text.remove();
                }
                Err(e) => {
                    loading_text.set_inner_html(
                        "<p> The app has crashed. See the developer console for details. </p>",
                    );
                    panic!("Failed to start eframe: {e:?}");
                }
            }
        }
    });
}

#[cfg(feature = "playground")]
/// String to save on crash. Used by the playground to avoid losing everything on panic.
static CRASH_STR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

#[cfg(not(feature = "playground"))]
fn run_playground(_: Option<PathBuf>, _: u32) {
    eprintln!("Playground was disabled when building Par")
}

#[cfg(not(target_family = "wasm"))]
#[cfg(feature = "playground")]
fn run_playground(file: Option<PathBuf>, max_interactions: u32) {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1000.0, 700.0]),
        ..Default::default()
    };

    // Set hook for pretty-printer on error.
    set_miette_hook();
    // Add hook to try printing current playground contents to stderr on error.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut stderr = std::io::stderr().lock();
        if let Ok(crash_str) = CRASH_STR.lock() {
            if let Some(crash_str) = &*crash_str {
                // Ignore the error. We are already panicking.
                let _ = write!(
                    stderr,
                    "Panic in progress. This is a bug, please file an issue containing your code:\n```par\n{}\n```",
                    crash_str
                );
            }
        }
        hook(info)
    }));

    eframe::run_native(
        "Par Playground",
        options,
        Box::new(|cc| Ok(Playground::new(cc, file, max_interactions))),
    )
    .expect("egui crashed");
}

fn run_definition(
    package_path: PathBuf,
    target: Option<String>,
    print_stats: bool,
    max_interactions: u32,
) {
    let runtime = tokio_factory::create_runtime().expect("Failed to create Tokio runtime");
    runtime.block_on(async {
        let (checked, rt_compiled, local_modules) =
            match build_runtime_package(&package_path, max_interactions) {
                Ok((checked, rt_compiled, local_modules)) => (checked, rt_compiled, local_modules),
                Err(error) => {
                    println!("{}", error.display());
                    return;
                }
            };

        let Some(name) = resolve_target_definition(target.as_deref(), &checked, &local_modules)
        else {
            let target = target.unwrap_or_else(|| "Main.Main".to_string());
            println!("{}: {}", "Definition not found".bright_red(), target);
            return;
        };

        let Some(Type::Break(_)) = rt_compiled.get_type_of(name) else {
            println!(
                "{}: {}",
                "Definition does not have the unit (!) type".bright_red(),
                target.unwrap_or_else(|| "Main.Main".to_string())
            );
            return;
        };

        let package_to_run = match rt_compiled.code.name_to_package.get(name) {
            Some(TranspiledGlobal::Package(pkg)) => pkg.clone(),
            Some(TranspiledGlobal::Unimplemented) => {
                println!(
                    "{}",
                    format!(
                        "Definition {} is unimplemented (directly or indirectly contains a todo; run `par check` for details)",
                        target.unwrap_or_else(|| "Main.Main".to_string())
                    )
                    .bright_red()
                );
                return;
            }
            None => {
                println!(
                    "{}",
                    format!(
                        "Definition {} not found",
                        target.unwrap_or_else(|| "Main.Main".to_string())
                    )
                    .bright_red()
                );
                return;
            }
        };
        let start = Instant::now();
        let start_runtime = if print_stats {
            par_runtime::start_and_instantiate_with_stats
        } else {
            par_runtime::start_and_instantiate
        };
        let (root, reducer_future) = start_runtime(
            Arc::new(TokioSpawn::new()),
            rt_compiled.code.arena.clone(),
            package_to_run,
        );

        root.continue_();
        let stats = reducer_future.await;

        if print_stats {
            eprintln!("{}", stats.show(start.elapsed()));
            eprintln!("\tArena size: {}", rt_compiled.code.arena.memory_size());
        }
    });
}

fn run_definition_vm(binary_path: PathBuf, target: Option<String>, print_stats: bool) {
    let runtime = tokio_factory::create_runtime().expect("Failed to create Tokio runtime");
    runtime.block_on(async {
        let file = File::open(binary_path).expect("Failed to open file");
        let reader = BufReader::new(file);
        let artifact: Artifact<Unlinked> =
            bincode::deserialize_from(reader).expect("Failed to deserialize artifact");
        let artifact = match artifact.link() {
            Ok(artifact) => artifact,
            Err(error) => {
                println!("{}", error.to_string().bright_red());
                return;
            }
        };

        let parsed_target = parse_target(target.as_deref().unwrap_or("Main.Main"));
        let definition_target = parsed_target
            .definition_name
            .unwrap_or_else(|| "Main".to_string());
        let target = format!("{}.{}", parsed_target.module_path, definition_target);

        let start = Instant::now();
        let package_to_run = match artifact.definition_to_package.get(&target) {
            Some(ArtifactGlobal::Package(pkg)) => pkg,
            Some(ArtifactGlobal::Unimplemented) => {
                println!(
                    "{}",
                    format!("Definition {target} is unimplemented (directly or indirectly contains a todo; run `par check` for details)").bright_red()
                );
                return;
            }
            None => {
                println!("{}", format!("Definition {target} not found").bright_red());
                return;
            }
        };
        let start_runtime = if print_stats {
            par_runtime::start_and_instantiate_with_stats
        } else {
            par_runtime::start_and_instantiate
        };
        let (root, reducer_future) = start_runtime(
            Arc::new(TokioSpawn::new()),
            artifact.arena.clone(),
            package_to_run.clone(),
        );

        root.continue_();
        let stats = reducer_future.await;

        if print_stats {
            eprintln!("{}", stats.show(start.elapsed()));
            eprintln!("\tArena size: {}", artifact.arena.memory_size());
        }
    });
}

fn compile(package_path: PathBuf, max_interactions: u32) {
    let (checked, rt_compiled, _local_modules, _sources) =
        match build_unlinked_package(&package_path, max_interactions) {
            Ok((checked, rt_compiled, local_modules, sources)) => {
                (checked, rt_compiled, local_modules, sources)
            }
            Err(error) => {
                println!("{}", error.display());
                return;
            }
        };

    let artifact: Artifact<Unlinked> = rt_compiled
        .code
        .into_artifact(checked.workspace().root_package());
    let file = File::create("compiled.pvm").expect("Failed to create file");
    let writer = BufWriter::new(file);
    bincode::serialize_into(writer, &artifact).expect("Failed to serialize");
}

fn resolve_target_definition<'a>(
    target: Option<&str>,
    checked: &'a CheckedWorkspace,
    local_modules: &[ModulePath],
) -> Option<&'a par_core::frontend::language::GlobalName<par_core::frontend::language::Universal>> {
    let parsed_target = parse_target(target.unwrap_or("Main.Main"));
    let definition_target = parsed_target
        .definition_name
        .unwrap_or_else(|| "Main".to_string());

    let canonical_module = find_local_module(&parsed_target.module_path, local_modules)?;
    let module_name = canonical_module.to_slash_path();

    checked
        .checked_module()
        .definitions
        .iter()
        .find(|(name, _)| {
            name.primary == definition_target
                && root_module_slash_path(checked.workspace().root_package(), &name.module)
                    .as_deref()
                    == Some(module_name.as_str())
        })
        .map(|(name, _)| name)
}

fn check(package_path: PathBuf) -> Result<(), String> {
    println!("Checking package: {}", package_path.display());

    let build_result = build_runtime_package(&package_path, MAX_INTERACTIONS_DEFAULT);
    if let Err(error) = build_result {
        let error_string = error.display();
        eprintln!("{error_string}");
        return Err(error_string);
    }
    Ok(())
}

fn create_temp_dir(prefix: &str) -> Result<PathBuf, String> {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("par-{prefix}-{unique}"));
    fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    Ok(path)
}

fn plan_doc_command(
    package: Option<PathBuf>,
    remote: Option<String>,
    out_dir: Option<PathBuf>,
    exported: bool,
    unexported: bool,
    open: bool,
) -> Result<DocCommandPlan, String> {
    if package.is_some() && remote.is_some() {
        return Err(String::from(
            "`--package` and `--remote` cannot be used together",
        ));
    }
    if exported && unexported {
        return Err(String::from(
            "`--exported` and `--unexported` cannot be used together",
        ));
    }

    let source = match (package, remote) {
        (Some(path), None) => DocSource::Local(path),
        (None, Some(source)) => DocSource::Remote(source),
        (None, None) => DocSource::Local(PathBuf::from(".")),
        (Some(_), Some(_)) => unreachable!("validated above"),
    };

    let temporary_out_dir = out_dir.is_none();
    let out_dir = match out_dir {
        Some(out_dir) => out_dir,
        None => create_temp_dir("doc")?,
    };
    let only_exported = if exported {
        true
    } else if unexported {
        false
    } else {
        matches!(source, DocSource::Remote(_))
    };

    Ok(DocCommandPlan {
        source,
        out_dir,
        temporary_out_dir,
        only_exported,
        open: open || temporary_out_dir,
    })
}

fn generate_docs(plan: DocCommandPlan) -> Result<(), String> {
    let package_path = match &plan.source {
        DocSource::Local(path) => path.clone(),
        DocSource::Remote(source) => {
            let package_root = create_temp_dir("doc-remote-package")?.join("package");
            package_manager::fetch_remote_package(source, &package_root)
                .map_err(|error| error.to_string())?;
            package_root
        }
    };

    let generated = par_doc::generate_docs(DocOptions {
        package_path,
        out_dir: Some(plan.out_dir.clone()),
        only_exported: plan.only_exported,
    })
    .map_err(|error| error.to_string())?;

    println!(
        "{} {}",
        "Generated documentation:".bright_green(),
        generated.out_dir.display()
    );

    #[cfg(not(target_family = "wasm"))]
    if plan.open {
        let url = file_url_for_path(&generated.index_file)?;
        webbrowser::open(url.as_str()).map_err(|error| error.to_string())?;
        println!(
            "{} {}",
            "Opened documentation:".bright_green(),
            generated.index_file.display()
        );
    }

    Ok(())
}

#[cfg(not(target_family = "wasm"))]
fn file_url_for_path(path: &Path) -> Result<Url, String> {
    let absolute_path = fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        }
    });

    Url::from_file_path(&absolute_path).map_err(|()| {
        format!(
            "Failed to convert documentation path to file URL: {}",
            absolute_path.display()
        )
    })
}

fn add_dependencies(package_path: PathBuf, source: Option<String>) -> Result<(), String> {
    println!(
        "Managing dependencies in package: {}",
        package_path.display()
    );
    let result = package_manager::add(&package_path, source.as_deref())
        .map_err(|error| error.to_string())?;

    if let Some(status) = result.added_dependency {
        match status {
            AddedDependencyStatus::Added { alias, source } => {
                println!(
                    "{} @{} = {}",
                    "Added dependency:".bright_green(),
                    alias,
                    source
                );
            }
            AddedDependencyStatus::AlreadyPresent { alias, source } => {
                println!(
                    "{} @{} = {}",
                    "Dependency already present:".yellow(),
                    alias,
                    source
                );
            }
        }
    }

    if result.fetched_dependencies.is_empty() {
        println!("{}", "Dependencies are up to date.".bright_green());
    } else {
        println!(
            "{} {}",
            "Fetched dependencies:".bright_green(),
            result.fetched_dependencies.join(", ")
        );
    }
    Ok(())
}

fn update_dependencies(package_path: PathBuf) -> Result<(), String> {
    println!(
        "Updating dependencies in package: {}",
        package_path.display()
    );
    let result = package_manager::update(&package_path).map_err(|error| error.to_string())?;
    if result.fetched_dependencies.is_empty() {
        println!("{}", "No remote dependencies to update.".bright_green());
    } else {
        println!(
            "{} {}",
            "Refetched dependencies:".bright_green(),
            result.fetched_dependencies.join(", ")
        );
    }
    Ok(())
}

#[cfg(not(target_family = "wasm"))]
fn run_language_server() {
    language_server::language_server_main::main()
}

fn run_tests(
    package_path: PathBuf,
    target: Option<String>,
    filter: Option<String>,
    max_interactions: u32,
) -> bool {
    test_runner::run_tests(package_path, target, filter, max_interactions)
}
