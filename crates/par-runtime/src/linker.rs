use crate::flat::arena::{Arena, Index};
use crate::flat::runtime::{
    ExternalFn, Global, GlobalCont, GlobalValue, Package, PackageBody, PackagePtr,
};
use crate::pkgid::PackageId;
use crate::registry::{DefinitionRef, PackageRef, get_external_fn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::{self, Display};
use std::hash::Hash;
use std::sync::{Arc, OnceLock};

pub type Linked = ExternalFn;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Unlinked {
    pub package: PackageId,
    pub path: Vec<String>,
    pub module: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkError {
    pub missing: Unlinked,
}

impl<'a> From<PackageRef<'a>> for PackageId {
    fn from(package: PackageRef) -> Self {
        match package {
            PackageRef::Builtin(name) => PackageId::Builtin(name),
            PackageRef::Special(name) => PackageId::Special(name.into()),
            PackageRef::Local(name) => PackageId::Local(name.into()),
            PackageRef::Remote(name) => PackageId::Remote(name.into()),
        }
    }
}

impl<'a> From<DefinitionRef<'a>> for Unlinked {
    fn from(value: DefinitionRef<'a>) -> Self {
        Unlinked {
            package: value.package.into(),
            path: value.path.iter().map(|s| s.to_string()).collect(),
            module: value.module.to_string(),
            name: value.name.to_string(),
        }
    }
}

impl Display for Unlinked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path_prefix = if self.path.is_empty() {
            String::new()
        } else {
            format!("{}/", self.path.join("/"))
        };
        write!(
            f,
            "{}/{}{}.{}",
            self.package, path_prefix, self.module, self.name
        )
    }
}

impl Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Missing external registration for `{}`", self.missing)
    }
}

impl std::error::Error for LinkError {}

pub fn link_arena(arena: &Arena<Unlinked>) -> Result<Arena<Linked>, LinkError> {
    Ok(Arena {
        nodes: arena
            .nodes
            .iter()
            .map(link_global)
            .collect::<Result<_, _>>()?,
        strings: arena.strings.clone(),
        string_to_location: arena
            .string_to_location
            .iter()
            .map(|(k, v)| (k.clone(), Index(v.0.clone())))
            .collect(),
        case_branches: arena
            .case_branches
            .iter()
            .map(|(index, package)| Ok((Index(index.0.clone()), link_package_body(package)?)))
            .collect::<Result<_, _>>()?,
        packages: arena
            .packages
            .iter()
            .map(link_package)
            .collect::<Result<_, _>>()?,
        redexes: arena
            .redexes
            .iter()
            .map(|(a, b)| (Index(a.0.clone()), Index(b.0.clone())))
            .collect(),
    })
}

fn link_package(
    package: &OnceLock<Package<Unlinked>>,
) -> Result<OnceLock<Package<Linked>>, LinkError> {
    let package = package.get().unwrap();
    let lock = OnceLock::new();
    lock.set(Package {
        body: link_package_body(&package.body)?,
        num_vars: package.num_vars,
    })
    .unwrap();
    Ok(lock)
}

fn link_package_body(body: &PackageBody<Unlinked>) -> Result<PackageBody<Linked>, LinkError> {
    Ok(PackageBody {
        root: Index(body.root.0.clone()),
        captures: Index(body.captures.0.clone()),
        debug_name: body.debug_name.clone(),
        redexes: Index(body.redexes.0.clone()),
    })
}

fn link_global(node: &Global<Unlinked>) -> Result<Global<Linked>, LinkError> {
    Ok(match node {
        Global::Variable(id) => Global::Variable(*id),
        Global::Close { signal, erase } => Global::Close {
            signal: Index(signal.0.clone()),
            erase: Index(erase.0.clone()),
        },
        Global::Package(package_ptr, global_ptr, fab_behavior) => Global::Package(
            Index(package_ptr.0.clone()),
            Index(global_ptr.0.clone()),
            fab_behavior.clone(),
        ),
        Global::Destruct(cont) => Global::Destruct(link_global_cont(cont)?),
        Global::Value(value) => Global::Value(link_global_value(value)?),
        Global::Fanout(fanout) => Global::Fanout(Index(fanout.0.clone())),
    })
}

fn link_global_value(p0: &GlobalValue<Unlinked>) -> Result<GlobalValue<Linked>, LinkError> {
    Ok(match p0 {
        GlobalValue::Break => GlobalValue::Break,
        GlobalValue::Pair(a, b) => GlobalValue::Pair(Index(a.0.clone()), Index(b.0.clone())),
        GlobalValue::Either(s, v) => GlobalValue::Either(Index(s.0.clone()), Index(v.0.clone())),
        GlobalValue::ExternalFn(unlinked) => {
            GlobalValue::ExternalFn(get_external_fn(unlinked).ok_or_else(|| LinkError {
                missing: unlinked.clone(),
            })?)
        }
        GlobalValue::ExternalArc(e) => GlobalValue::ExternalArc(e.clone()),
        GlobalValue::Primitive(p) => GlobalValue::Primitive(p.clone()),
    })
}

fn link_global_cont(cont: &GlobalCont<Unlinked>) -> Result<GlobalCont<Linked>, LinkError> {
    Ok(match cont {
        GlobalCont::Continue => GlobalCont::Continue,
        GlobalCont::Par(a, b) => GlobalCont::Par(Index(a.0.clone()), Index(b.0.clone())),
        GlobalCont::Choice(a, b) => GlobalCont::Choice(Index(a.0.clone()), Index(b.0.clone())),
    })
}

pub fn link_package_ptr(package: &PackagePtr<Unlinked>) -> PackagePtr<Linked> {
    Index(package.0.clone())
}

#[derive(Serialize, Deserialize)]
pub struct Artifact<Ext: Clone> {
    pub arena: Arc<Arena<Ext>>,
    pub definition_to_package: HashMap<String, PackagePtr<Ext>>,
}

impl Artifact<Unlinked> {
    pub fn link(&self) -> Result<Artifact<Linked>, LinkError> {
        Ok(Artifact {
            arena: Arc::new(link_arena(self.arena.as_ref())?),
            definition_to_package: self
                .definition_to_package
                .iter()
                .map(|(k, v)| (k.clone(), link_package_ptr(v)))
                .collect(),
        })
    }
}
