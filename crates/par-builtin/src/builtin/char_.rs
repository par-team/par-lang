use num_bigint::BigUint;
use num_traits::ToPrimitive;

use par_core::frontend::{ExternalTypeDef, PrimitiveType, Type};
use par_core::source::Span;
use par_runtime::atom::sym;
use par_runtime::readback::Handle;
use par_runtime::registry::{DefinitionRef, ExternalDef, PackageRef};

inventory::submit!(ExternalTypeDef {
    path: DefinitionRef {
        package: PackageRef::CORE,
        path: &[],
        module: "Char",
        name: "Char"
    },
    doc: r"A primitive type representing a [Unicode scalar value](https://www.unicode.org/glossary/#unicode_scalar_value).",
    typ: Type::Primitive(Span::None, PrimitiveType::Char)
});

macro_rules! core_char_external {
    ($name:literal, $f:path $(, $arg:expr)*) => {
        inventory::submit!(ExternalDef {
            path: DefinitionRef {
                package: PackageRef::CORE,
                path: &[],
                module: "Char",
                name: $name,
            },
            f: |handle| Box::pin($f(handle $(, $arg)*)),
        });
    };
}

core_char_external!("Code", char_code);
core_char_external!("FromCode", char_from_code);
core_char_external!("Is", char_is);
core_char_external!("ToLower", char_to_lower);
core_char_external!("ToUpper", char_to_upper);

async fn char_code(mut handle: Handle) {
    let c = handle.receive().char().await;
    handle.provide_nat(BigUint::from(c as u32));
}

async fn char_from_code(mut handle: Handle) {
    let code = handle.receive().nat().await;
    let ch = code
        .to_u32()
        .and_then(char::from_u32)
        .unwrap_or(char::REPLACEMENT_CHARACTER);
    handle.provide_char(ch);
}

async fn char_to_lower(mut handle: Handle) {
    let ch = handle.receive().char().await;
    handle.provide_char(ch.to_lowercase().next().unwrap_or(ch));
}

async fn char_to_upper(mut handle: Handle) {
    let ch = handle.receive().char().await;
    handle.provide_char(ch.to_uppercase().next().unwrap_or(ch));
}

async fn char_is(mut handle: Handle) {
    let ch = handle.receive().char().await;
    let class = CharClass::readback(handle.receive()).await;
    if class.contains(ch) {
        handle.signal(sym::true_);
    } else {
        handle.signal(sym::false_);
    }
    handle.break_();
}

#[derive(Debug, Clone)]
pub(super) enum CharClass {
    Any,
    Char(char),
    Whitespace,
    AsciiAny,
    AsciiAlpha,
    AsciiAlphanum,
    AsciiDigit,
}

impl CharClass {
    pub(super) async fn readback(mut handle: Handle) -> Self {
        match handle.case().await {
            sym::any => {
                handle.continue_();
                Self::Any
            }
            sym::ascii => match handle.case().await {
                sym::alpha => {
                    handle.continue_();
                    Self::AsciiAlpha
                }
                sym::alphanum => {
                    handle.continue_();
                    Self::AsciiAlphanum
                }
                sym::any => {
                    handle.continue_();
                    Self::AsciiAny
                }
                sym::digit => {
                    handle.continue_();
                    Self::AsciiDigit
                }
                _ => unreachable!(),
            },
            sym::char => Self::Char(handle.char().await),
            sym::whitespace => {
                handle.continue_();
                Self::Whitespace
            }
            _ => unreachable!(),
        }
    }

    pub(super) fn contains(&self, ch: char) -> bool {
        match self {
            Self::Any => true,
            Self::Char(ch1) => ch == *ch1,
            Self::Whitespace => ch.is_whitespace(),
            Self::AsciiAny => ch.is_ascii(),
            Self::AsciiAlpha => ch.is_ascii_alphabetic(),
            Self::AsciiAlphanum => ch.is_ascii_alphanumeric(),
            Self::AsciiDigit => ch.is_ascii_digit(),
        }
    }
}
