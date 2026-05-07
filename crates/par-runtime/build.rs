use std::env;
use std::path::PathBuf;

macro_rules! atoms {
    ($($name:ident $(= $str:expr)?,)*) => {
        const ATOMS: &[&str] = &[
            $(($($str,)? stringify!($name),).0,)*
        ];
    };
}

include!("src/atom/sym.rs");

fn main() -> std::io::Result<()> {
    println!("cargo::rerun-if-changed=src/atom/sym.rs");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());

    let mut out = Vec::new();
    string_cache_codegen::AtomType::new("atom::atom_internal::Atom", "atom!")
        .atoms(ATOMS)
        .write_to(&mut out)?;
    let out = String::from_utf8(out)
        .unwrap()
        .replace("#[macro_export]", "");

    std::fs::write(out_dir.join("atoms.rs"), out)
}
