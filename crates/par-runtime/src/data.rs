use crate::atom::Atom;
use crate::primitive::Primitive;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Data {
    Unit,
    Either(Atom, Box<Data>),
    Pair(Box<Data>, Box<Data>),
    Primitive(Primitive),
}

impl fmt::Display for Data {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit => write!(f, "!"),
            Self::Pair(left, right) => write!(f, "({left}) {right}"),
            Self::Either(label, payload) => {
                write!(f, ".{label}")?;
                if !matches!(payload.as_ref(), Self::Unit | Self::Pair(_, _)) {
                    write!(f, " ")?;
                }
                write!(f, "{payload}")
            }
            Self::Primitive(primitive) => primitive.pretty(f, 0),
        }
    }
}
