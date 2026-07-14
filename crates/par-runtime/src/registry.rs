use crate::flat::runtime::ExternalFn;
use crate::linker::{Linked, Unlinked};
use crate::pkgid::BuiltinPackage;
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PackageRef<'a> {
    Builtin(BuiltinPackage),
    Special(&'a str),
    Local(&'a str),
    Remote(&'a str),
}

impl PackageRef<'_> {
    pub const CORE: Self = Self::Builtin(BuiltinPackage::Core);
    pub const BASIC: Self = Self::Builtin(BuiltinPackage::Basic);
}

#[derive(Hash, Eq, PartialEq, Clone, Copy, Debug)]
pub struct DefinitionRef<'a> {
    pub package: PackageRef<'a>,
    pub path: &'a [&'a str],
    pub module: &'a str,
    pub name: &'a str,
}

#[derive(Clone, Copy)]
pub struct ExternalDef {
    pub path: DefinitionRef<'static>,
    pub f: ExternalFn,
}

inventory::collect!(ExternalDef);

type Registry = HashMap<Unlinked, Linked>;

static REGISTRY: LazyLock<Registry> = LazyLock::new(|| {
    inventory::iter::<ExternalDef>
        .into_iter()
        .map(|&ExternalDef { path, f }| (path.into(), f))
        .collect()
});

pub fn get_external_fn(path: &Unlinked) -> Option<ExternalFn> {
    REGISTRY.get(path).copied()
}

#[expect(non_upper_case_globals)]
#[doc(hidden)]
pub mod __package_ref {
    use super::PackageRef;
    pub const basic: PackageRef = PackageRef::BASIC;
    pub const core: PackageRef = PackageRef::CORE;
}

/// Provides implementation for an `external` definition.
/// # Syntax
/// ```ignore
/// external_def! {
///     @package/path/to/Module.Name => handler
/// }
/// external_def! {
///     @package/path/to/Module.Name => handler(arg1, arg2)
/// }
/// external_def! {
///     @package/path/to/Module.{
///         Name1 => handler1,
///         Name2 => handler2(arg1, arg2),
///     }
/// }
/// ```
/// Here, `package` is either `core` or `basic`, and `handler` must be the name of an async function
/// that takes a [`Handle`](crate::readback::Handle) as the first argument,
/// while all `arg`s would be passed as subsequent arguments.
/// See `par-builtin/src/builtin/` for usages.
///
/// The `%`-branches shown in the definition are an implementation detail and shouldn't be used.
#[macro_export]
macro_rules! external_def {
    (@$pkg:ident/$($path:ident)/+.$name:ident => $f:ident($($arg:expr),*)) => {
        ::inventory::submit!($crate::registry::ExternalDef {
            f: |handle| ::std::boxed::Box::pin($f(handle, $($arg),*)),
            path: {
                let [ref path @ .., module] = [$(::core::stringify!($path)),+];
                $crate::registry::DefinitionRef {
                    name: ::core::stringify!($name),
                    package: $crate::registry::__package_ref::$pkg,
                    module,
                    path,
                }
            }
        });
    };

    (@$($path:ident)/+.$name:ident => $f:ident) => {
        $crate::external_def! { @$($path)/+.$name => $f() }
    };
    (@$($path:ident)/+.$name:ident => $f:ident($(arg:expr),+ ,)) => {
        $crate::external_def! { @$($path)/+.$name => $f($($arg),+) }
    };

    (@$($path:ident)/+.{$($name:ident => $f:ident$(($($param:tt)*))?),*}) => {
        $crate::external_def! { %exp[$($path)+]; $($name => $f($($($param)*)?)),* }
    };
    (@$($path:ident)/+.{$($name:ident => $f:ident$(($($param:tt)*))?),+ ,}) => {
        $crate::external_def! { @$($path)/+.{$($name => $f$(($($param)*))?),+ } }
    };

    (%exp1[$($path:ident)+]; $name:ident => $f:ident$params:tt) => {
        $crate::external_def! { @$($path)/+.$name => $f$params }
    };
    (%exp$path:tt; $($name:ident => $f:ident$params:tt),*) => {
        $(
            $crate::external_def! {%exp1 $path; $name => $f$params }
        )*
    };
}
