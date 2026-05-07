mod atom_internal {
    #![allow(non_upper_case_globals)]
    include!(concat!(env!("OUT_DIR"), "/atoms.rs"));

    // silence unused warning
    const _: Atom = atom!("");

    pub(crate) use atom;
}

pub use atom_internal::Atom;

macro_rules! atoms {
    ($($name:ident $(= $str:tt)?,)*) => {
        $(
            #[allow(non_upper_case_globals)]
            pub const $name: $crate::atom::Atom = atoms!(@atom $name $(= $str)?);
        )*
    };
    (@atom $name:ident) => {
        $crate::atom::atom_internal::atom!($name)
    };
    (@atom $name:ident = $str:tt) => {
        $crate::atom::atom_internal::atom!($str)
    };
}

pub mod sym;
