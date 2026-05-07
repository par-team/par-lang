use num_bigint::BigUint;
use num_traits::ToPrimitive;

use par_core::frontend::ExternalTypeDef;
use par_core::frontend::{PrimitiveType, Type};
use par_core::source::Span;
use par_runtime::atom::sym;
use par_runtime::readback::Handle;
use par_runtime::registry::{DefinitionRef, ExternalDef, PackageRef};

inventory::submit!(ExternalTypeDef {
    path: DefinitionRef {
        package: PackageRef::CORE,
        path: &[],
        module: "Byte",
        name: "Byte"
    },
    doc: r"A primitive type representing a single byte.",
    typ: Type::Primitive(Span::None, PrimitiveType::Byte)
});

macro_rules! core_byte_external {
    ($name:literal, $f:path $(, $arg:expr)*) => {
        inventory::submit!(ExternalDef {
            path: DefinitionRef {
                package: PackageRef::CORE,
                path: &[],
                module: "Byte",
                name: $name,
            },
            f: |handle| Box::pin($f(handle $(, $arg)*)),
        });
    };
}

core_byte_external!("Code", byte_code);
core_byte_external!("FromCode", byte_from_code);
core_byte_external!("Is", byte_is);

async fn byte_code(mut handle: Handle) {
    let c = handle.receive().byte().await;
    handle.provide_nat(c.into())
}

async fn byte_from_code(mut handle: Handle) {
    let n = handle.receive().nat().await;
    let byte = (n % BigUint::from(256u32)).to_u8().unwrap();
    handle.provide_byte(byte);
}

async fn byte_is(mut handle: Handle) {
    let b = handle.receive().byte().await;
    let class = ByteClass::readback(handle.receive()).await;
    if class.contains(b) {
        handle.signal(sym::true_);
    } else {
        handle.signal(sym::false_);
    }
    handle.break_();
}

#[derive(Debug, Clone)]
pub(super) enum ByteClass {
    Any,
    Byte(u8),
    Range(u8, u8),
}

impl ByteClass {
    pub(super) async fn readback(mut handle: Handle) -> Self {
        match handle.case().await {
            sym::any => {
                handle.continue_();
                Self::Any
            }
            sym::byte => Self::Byte(handle.byte().await),
            sym::range => {
                let min = handle.receive().byte().await;
                let max = handle.receive().byte().await;
                handle.continue_();
                Self::Range(min, max)
            }
            _ => unreachable!(),
        }
    }

    pub(super) fn contains(&self, b: u8) -> bool {
        match self {
            Self::Any => true,
            Self::Byte(b1) => b == *b1,
            Self::Range(min, max) => *min <= b && b <= *max,
        }
    }
}
