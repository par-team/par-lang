mod bench;
mod boxmap;
mod byte;
mod bytes;
mod char_;
mod console;
mod data;
mod debug;
mod float;
#[cfg(not(target_family = "wasm"))]
mod http;
mod int;
mod json;
mod list;
mod map;
mod nat;
mod number;
#[cfg(not(target_family = "wasm"))]
mod os;
mod parser;
#[cfg(not(target_family = "wasm"))]
mod sql;
mod string;
mod time;
mod url;

use std::collections::{BTreeMap, btree_map::Entry};
use std::env;
use std::path::PathBuf;

use par_core::frontend::{TypeDef, get_external_type_defs};
use par_core::source::FileName;
use par_core::workspace::{
    ExternalModule, LoadedPackageFile, ModulePath, WorkspaceDiscoveryError, WorkspacePackage,
    WorkspacePackages, parse_loaded_files,
};
use par_runtime::pkgid::{BuiltinPackage, PackageId};
use par_runtime::registry::PackageRef;

pub fn builtin_packages() -> impl Iterator<Item = WorkspacePackage> {
    // skip if NOSTD is set.
    let enable_builtins = env::var("NOSTD").ok().is_none();
    let packages = enable_builtins.then(|| {
        [
            core_package(),
            #[cfg(not(target_family = "wasm"))]
            basic_package(),
        ]
    });
    packages.into_iter().flatten()
}

fn core_package() -> WorkspacePackage {
    let parsed = parse_builtin_sources("core", CORE_SOURCE_FILES);
    let externals = load_external_type_defs(BuiltinPackage::Core);
    WorkspacePackage::new(PackageId::Builtin(BuiltinPackage::Core), parsed)
        .with_externals(externals)
}

#[cfg(not(target_family = "wasm"))]
fn basic_package() -> WorkspacePackage {
    let parsed = parse_builtin_sources("basic", BASIC_SOURCE_FILES);
    let externals = load_external_type_defs(BuiltinPackage::Basic);
    WorkspacePackage::new(PackageId::Builtin(BuiltinPackage::Basic), parsed)
        .with_dependency("core", PackageId::Builtin(BuiltinPackage::Core))
        .with_externals(externals)
}

struct BuiltinSourceFile {
    relative_path_from_src: &'static str,
    source: &'static str,
}

const CORE_SOURCE_FILES: &[BuiltinSourceFile] = &[
    BuiltinSourceFile {
        relative_path_from_src: "Bench.par",
        source: include_str!("../packages/core/src/Bench.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Bool.par",
        source: include_str!("../packages/core/src/Bool.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "BoxMap.par",
        source: include_str!("../packages/core/src/BoxMap.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Byte.par",
        source: include_str!("../packages/core/src/Byte.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Bytes.par",
        source: include_str!("../packages/core/src/Bytes.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Debug.par",
        source: include_str!("../packages/core/src/Debug.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Data.par",
        source: include_str!("../packages/core/src/Data.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Float.par",
        source: include_str!("../packages/core/src/Float.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Cell.par",
        source: include_str!("../packages/core/src/Cell.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Char.par",
        source: include_str!("../packages/core/src/Char.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Int.par",
        source: include_str!("../packages/core/src/Int.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Json.par",
        source: include_str!("../packages/core/src/Json.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "List.par",
        source: include_str!("../packages/core/src/List.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Map.par",
        source: include_str!("../packages/core/src/Map.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Nat.par",
        source: include_str!("../packages/core/src/Nat.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Option.par",
        source: include_str!("../packages/core/src/Option.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Number.par",
        source: include_str!("../packages/core/src/Number.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Ordering.par",
        source: include_str!("../packages/core/src/Ordering.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Stream.par",
        source: include_str!("../packages/core/src/Stream.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Try.par",
        source: include_str!("../packages/core/src/Try.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "String.par",
        source: include_str!("../packages/core/src/String.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Test.par",
        source: include_str!("../packages/core/src/Test.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Time.par",
        source: include_str!("../packages/core/src/Time.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Url.par",
        source: include_str!("../packages/core/src/Url.par"),
    },
];

const BASIC_SOURCE_FILES: &[BuiltinSourceFile] = &[
    BuiltinSourceFile {
        relative_path_from_src: "Console.par",
        source: include_str!("../packages/basic/src/Console.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Http.par",
        source: include_str!("../packages/basic/src/Http.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Os.par",
        source: include_str!("../packages/basic/src/Os.par"),
    },
    BuiltinSourceFile {
        relative_path_from_src: "Sql.par",
        source: include_str!("../packages/basic/src/Sql.par"),
    },
];

pub fn get_builtin_source(filename: &str) -> Option<&'static str> {
    let (package, path) = filename.split_once('/')?;
    let files = match package {
        "core" => CORE_SOURCE_FILES,
        "basic" => BASIC_SOURCE_FILES,
        _ => return None,
    };
    files
        .iter()
        .find_map(|file| (file.relative_path_from_src == path).then_some(file.source))
}

pub const PAR_BUILTIN_URI_SCHEME: &str = "par-builtin";

fn parse_builtin_sources(
    package_name: &str,
    source_files: &[BuiltinSourceFile],
) -> par_core::workspace::ParsedPackage {
    let files = source_files
        .iter()
        .map(|file| LoadedPackageFile {
            name: FileName::from(format!(
                "{PAR_BUILTIN_URI_SCHEME}:{}/{}",
                package_name, file.relative_path_from_src
            )),
            relative_path_from_src: PathBuf::from(file.relative_path_from_src),
            source: file.source.to_owned(),
        })
        .collect();
    parse_loaded_files(files).expect("embedded builtin package should parse")
}

fn load_external_type_defs(name: BuiltinPackage) -> BTreeMap<ModulePath, ExternalModule> {
    let mut externals = BTreeMap::<ModulePath, ExternalModule>::new();
    for type_def in get_external_type_defs(PackageRef::Builtin(name)) {
        let module_path = ModulePath {
            directories: type_def.path.path.iter().map(|s| s.to_string()).collect(),
            module: type_def.path.module.into(),
        };
        let module = externals.entry(module_path).or_default();
        module.type_defs.push(TypeDef::external(
            type_def.path.name,
            type_def.doc,
            type_def.typ.clone(),
        ));
    }
    externals
}

pub fn inject_builtin_packages(
    workspace_packages: &mut WorkspacePackages,
) -> Result<(), WorkspaceDiscoveryError> {
    for package in &mut workspace_packages.packages {
        if let PackageId::Builtin(name) = package.id {
            package.externals = load_external_type_defs(name);
            continue;
        }

        for &builtin in BuiltinPackage::ALL {
            match package.dependencies.entry(builtin.to_string()) {
                Entry::Vacant(v) => {
                    v.insert(PackageId::Builtin(builtin));
                }
                Entry::Occupied(_) => {
                    return Err(WorkspaceDiscoveryError::DependencyAliasCollision {
                        package: package.id.clone(),
                        alias: builtin.to_string(),
                    });
                }
            }
        }
    }

    if !matches!(workspace_packages.root_package, PackageId::Builtin(_)) {
        workspace_packages.packages.extend(builtin_packages());
    }
    Ok(())
}
