//package: core
use num_bigint::{BigInt, BigUint};

use num_integer::Integer;
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
        module: "Nat",
        name: "Nat"
    },
    doc: r"A primitive type representing an arbitrary-precision natural number.",
    typ: Type::Primitive(Span::None, PrimitiveType::Nat)
});

macro_rules! core_nat_external {
    ($name:literal, $f:path $(, $arg:expr)*) => {
        inventory::submit!(ExternalDef {
            path: DefinitionRef {
                package: PackageRef::CORE,
                path: &[],
                module: "Nat",
                name: $name,
            },
            f: |handle| Box::pin($f(handle $(, $arg)*)),
        });
    };
}

core_nat_external!("Mod", nat_mod);
core_nat_external!("Min", nat_min);
core_nat_external!("Max", nat_max);
core_nat_external!("Clamp", nat_clamp);
core_nat_external!("Repeat", nat_repeat);
core_nat_external!("RepeatLazy", nat_repeat_lazy);
core_nat_external!("Range", nat_range);
core_nat_external!("FromString", nat_from_string);

async fn nat_mod(mut handle: Handle) {
    let x = handle.receive().nat().await;
    let y = handle.receive().nat().await;
    handle.provide_nat(if y.is_zero() { y } else { x % y });
}

async fn nat_min(mut handle: Handle) {
    let x = handle.receive().nat().await;
    let y = handle.receive().nat().await;
    handle.provide_nat(x.min(y));
}

async fn nat_max(mut handle: Handle) {
    let x = handle.receive().nat().await;
    let y = handle.receive().int().await;
    // max of int and nat is always nat, so we can ignore the sign.
    let (_sign, max) = BigInt::from(x).max(y).into_parts();
    handle.provide_nat(max);
}

async fn nat_clamp(mut handle: Handle) {
    let int = handle.receive().int().await;
    let min = handle.receive().nat().await;
    let max = handle.receive().nat().await;
    // int clamped to two nats is always nat, so we can ignore the sign.
    let (_sign, clamped) = int.clamp(min.into(), max.into()).into_parts();
    handle.provide_nat(clamped);
}

async fn nat_repeat(mut handle: Handle) {
    let mut n = handle.receive().nat().await;
    while n > BigUint::ZERO {
        handle.signal(sym::step);
        n.dec();
    }
    handle.signal(sym::end);
    handle.break_();
}

async fn nat_repeat_lazy(mut handle: Handle) {
    let n = handle.receive().nat().await;
    nat_repeat_lazy_inner(handle, n);
}

fn nat_repeat_lazy_inner(mut handle: Handle, n: BigUint) {
    if n > BigUint::ZERO {
        handle.signal(sym::step);
        handle.provide_box(move |mut handle| {
            let n = &n - 1u32;
            async move {
                match handle.case().await {
                    sym::next => nat_repeat_lazy_inner(handle, n.clone()),
                    _ => unreachable!(),
                }
            }
        });
    } else {
        handle.signal(sym::end);
        handle.break_();
    }
}

async fn nat_range(mut handle: Handle) {
    let lo = handle.receive().nat().await;
    let hi = handle.receive().nat().await;

    let mut i = lo;
    while i < hi {
        handle.signal(sym::item);
        handle.send().provide_nat(i.clone());
        i += 1u32;
    }
    handle.signal(sym::end);
    handle.break_();
}

async fn nat_from_string(mut handle: Handle) {
    let string = handle.receive().string().await;
    match string.as_str().parse::<BigUint>() {
        Ok(num) => {
            handle.signal(sym::some);
            handle.provide_nat(num);
        }
        Err(_) => {
            handle.signal(sym::none);
            handle.break_();
        }
    };
}
