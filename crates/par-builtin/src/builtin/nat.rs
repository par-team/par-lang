//package: core
use arcstr::literal;
use num_bigint::{BigInt, BigUint};

use num_integer::Integer;
use num_traits::Zero;
use par_core::frontend::{ExternalTypeDef, PrimitiveType, Type};
use par_core::source::Span;
use par_runtime::external_def;
use par_runtime::readback::Handle;
use par_runtime::registry::{DefinitionRef, PackageRef};

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

external_def! {
    @core/Nat.{
        Mod => nat_mod,
        Min => nat_min,
        Max => nat_max,
        Clamp => nat_clamp,
        Repeat => nat_repeat,
        RepeatLazy => nat_repeat_lazy,
        Range => nat_range,
        FromString => nat_from_string,
    }
}

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
    let (_sign, clamped) = int.min(max.into()).max(min.into()).into_parts();
    handle.provide_nat(clamped);
}

async fn nat_repeat(mut handle: Handle) {
    let mut n = handle.receive().nat().await;
    while n > BigUint::ZERO {
        handle.signal(literal!("step"));
        n.dec();
    }
    handle.signal(literal!("end"));
    handle.break_();
}

async fn nat_repeat_lazy(mut handle: Handle) {
    let n = handle.receive().nat().await;
    nat_repeat_lazy_inner(handle, n);
}

fn nat_repeat_lazy_inner(mut handle: Handle, n: BigUint) {
    if n > BigUint::ZERO {
        handle.signal(literal!("step"));
        handle.provide_box(move |mut handle| {
            let n = &n - 1u32;
            async move {
                match handle.case().await.as_str() {
                    "next" => nat_repeat_lazy_inner(handle, n.clone()),
                    _ => unreachable!(),
                }
            }
        });
    } else {
        handle.signal(literal!("end"));
        handle.break_();
    }
}

async fn nat_range(mut handle: Handle) {
    let lo = handle.receive().nat().await;
    let hi = handle.receive().nat().await;

    let mut i = lo;
    while i < hi {
        handle.signal(literal!("item"));
        handle.send().provide_nat(i.clone());
        i += 1u32;
    }
    handle.signal(literal!("end"));
    handle.break_();
}

async fn nat_from_string(mut handle: Handle) {
    let string = handle.receive().string().await;
    match string.as_str().parse::<BigUint>() {
        Ok(num) => {
            handle.signal(literal!("some"));
            handle.provide_nat(num);
        }
        Err(_) => {
            handle.signal(literal!("none"));
            handle.break_();
        }
    };
}
