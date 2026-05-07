//package: core
use num_bigint::{BigInt, BigUint};
use num_traits::Zero;
use par_core::frontend::{ExternalTypeDef, PrimitiveType, Type};
use par_core::source::Span;
use par_runtime::atom::sym;
use par_runtime::readback::Handle;
use par_runtime::registry::{DefinitionRef, ExternalDef, PackageRef};

inventory::submit!(ExternalTypeDef {
    path: DefinitionRef {
        package: PackageRef::CORE,
        path: &[],
        module: "Int",
        name: "Int"
    },
    doc: r"A primitive type representing an arbitrary-precision integer.",
    typ: Type::Primitive(Span::None, PrimitiveType::Int)
});

macro_rules! core_int_external {
    ($name:literal, $f:path $(, $arg:expr)*) => {
        inventory::submit!(ExternalDef {
            path: DefinitionRef {
                package: PackageRef::CORE,
                path: &[],
                module: "Int",
                name: $name,
            },
            f: |handle| Box::pin($f(handle $(, $arg)*)),
        });
    };
}

core_int_external!("Mod", int_mod);
core_int_external!("Min", int_min);
core_int_external!("Max", int_max);
core_int_external!("Abs", int_abs);
core_int_external!("Clamp", int_clamp);
core_int_external!("Range", int_range);
core_int_external!("FromString", int_from_string);

async fn int_mod(mut handle: Handle) {
    let x = handle.receive().int().await;
    let y = handle.receive().nat().await;
    let result = if y.is_zero() {
        BigUint::ZERO
    } else {
        let modulus = num_integer::mod_floor(x, BigInt::from(y));
        BigUint::try_from(modulus)
            .expect("y is always positive so the result should always be positive")
    };
    handle.provide_nat(result);
}

async fn int_min(mut handle: Handle) {
    let x = handle.receive().int().await;
    let y = handle.receive().int().await;
    handle.provide_int(x.min(y));
}

async fn int_max(mut handle: Handle) {
    let x = handle.receive().int().await;
    let y = handle.receive().int().await;
    handle.provide_int(x.max(y));
}

async fn int_clamp(mut handle: Handle) {
    let int = handle.receive().int().await;
    let min = handle.receive().int().await;
    let max = handle.receive().int().await;
    handle.provide_int(int.min(max).max(min));
}

async fn int_abs(mut handle: Handle) {
    let int = handle.receive().int().await;
    let (_sign, magnitude) = int.into_parts();
    handle.provide_nat(magnitude);
}

async fn int_range(mut handle: Handle) {
    let lo = handle.receive().int().await;
    let hi = handle.receive().int().await;

    let mut i = lo;
    while i < hi {
        handle.signal(sym::item);
        handle.send().provide_int(i.clone());
        i += 1;
    }
    handle.signal(sym::end);
    handle.break_();
}

async fn int_from_string(mut handle: Handle) {
    let string = handle.receive().string().await;
    match string.as_str().parse::<BigInt>() {
        Ok(num) => {
            handle.signal(sym::some);
            handle.provide_int(num);
        }
        Err(_) => {
            handle.signal(sym::none);
            handle.break_();
        }
    };
}
