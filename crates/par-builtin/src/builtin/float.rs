//package: core
use std::f64::consts;

use num_bigint::{BigInt, Sign};
use num_traits::{FromPrimitive, ToPrimitive};
use par_core::frontend::{ExternalTypeDef, PrimitiveType, Type};
use par_core::source::Span;
use par_runtime::atom::sym;
use par_runtime::primitive::parse_float_text;
use par_runtime::readback::Handle;
use par_runtime::registry::{DefinitionRef, ExternalDef, PackageRef};

inventory::submit!(ExternalTypeDef {
    path: DefinitionRef {
        package: PackageRef::CORE,
        path: &[],
        module: "Float",
        name: "Float"
    },
    doc: r"A primitive type representing a 64-bit floating point number.",
    typ: Type::Primitive(Span::None, PrimitiveType::Float)
});

macro_rules! core_float_external {
    ($name:literal, $f:path $(, $arg:expr)*) => {
        inventory::submit!(ExternalDef {
            path: DefinitionRef {
                package: PackageRef::CORE,
                path: &[],
                module: "Float",
                name: $name,
            },
            f: |handle| Box::pin($f(handle $(, $arg)*)),
        });
    };
}

core_float_external!("NaN_", float_nan);
core_float_external!("Inf_", float_inf);
core_float_external!("NegInf_", float_neg_inf);
core_float_external!("Pi_", float_pi);
core_float_external!("E_", float_e);
core_float_external!("IsNaN", float_is_nan);
core_float_external!("IsFinite", float_is_finite);
core_float_external!("IsInfinite", float_is_infinite);
core_float_external!("FromInt", float_from_int);
core_float_external!("ToInt", float_to_int);
core_float_external!("FromString", float_from_string);
core_float_external!("Neg", float_neg);
core_float_external!("Abs", float_abs);
core_float_external!("Floor", float_floor);
core_float_external!("Ceil", float_ceil);
core_float_external!("Round", float_round);
core_float_external!("Pow", float_pow);
core_float_external!("Min", float_min);
core_float_external!("Max", float_max);
core_float_external!("Clamp", float_clamp);
core_float_external!("Equals", float_equals);
core_float_external!("Sqrt", float_sqrt);
core_float_external!("Exp", float_exp);
core_float_external!("Ln", float_ln);
core_float_external!("Sin", float_sin);
core_float_external!("Cos", float_cos);
core_float_external!("Tan", float_tan);
core_float_external!("Atan2", float_atan2);

fn signal_bool(mut handle: Handle, value: bool) {
    if value {
        handle.signal(sym::true_);
    } else {
        handle.signal(sym::false_);
    }
    handle.break_();
}

fn min_float(left: f64, right: f64) -> f64 {
    if left.is_nan() || right.is_nan() {
        f64::NAN
    } else {
        left.min(right)
    }
}

fn max_float(left: f64, right: f64) -> f64 {
    if left.is_nan() || right.is_nan() {
        f64::NAN
    } else {
        left.max(right)
    }
}

fn bigint_to_float(value: BigInt) -> f64 {
    match value.to_f64() {
        Some(value) => value,
        None if value.sign() == Sign::Minus => f64::NEG_INFINITY,
        None => f64::INFINITY,
    }
}

fn float_to_bigint(value: f64) -> BigInt {
    BigInt::from_f64(value).unwrap_or_default()
}

async fn float_nan(mut handle: Handle) {
    handle.receive().continue_();
    handle.provide_float(f64::NAN);
}

async fn float_inf(mut handle: Handle) {
    handle.receive().continue_();
    handle.provide_float(f64::INFINITY);
}

async fn float_neg_inf(mut handle: Handle) {
    handle.receive().continue_();
    handle.provide_float(f64::NEG_INFINITY);
}

async fn float_pi(mut handle: Handle) {
    handle.receive().continue_();
    handle.provide_float(consts::PI);
}

async fn float_e(mut handle: Handle) {
    handle.receive().continue_();
    handle.provide_float(consts::E);
}

async fn float_is_nan(mut handle: Handle) {
    let value = handle.receive().float().await;
    signal_bool(handle, value.is_nan());
}

async fn float_is_finite(mut handle: Handle) {
    let value = handle.receive().float().await;
    signal_bool(handle, value.is_finite());
}

async fn float_is_infinite(mut handle: Handle) {
    let value = handle.receive().float().await;
    signal_bool(handle, value.is_infinite());
}

async fn float_from_int(mut handle: Handle) {
    let value = handle.receive().int().await;
    handle.provide_float(bigint_to_float(value));
}

async fn float_to_int(mut handle: Handle) {
    let value = handle.receive().float().await;
    handle.provide_int(float_to_bigint(value));
}

async fn float_from_string(mut handle: Handle) {
    let string = handle.receive().string().await;
    match parse_float_text(string.as_str()) {
        Some(value) => {
            handle.signal(sym::some);
            handle.provide_float(value);
        }
        None => {
            handle.signal(sym::none);
            handle.break_();
        }
    }
}

async fn float_neg(mut handle: Handle) {
    let value = handle.receive().float().await;
    handle.provide_float(-value);
}

async fn float_abs(mut handle: Handle) {
    let value = handle.receive().float().await;
    handle.provide_float(value.abs());
}

async fn float_floor(mut handle: Handle) {
    let value = handle.receive().float().await;
    handle.provide_float(value.floor());
}

async fn float_ceil(mut handle: Handle) {
    let value = handle.receive().float().await;
    handle.provide_float(value.ceil());
}

async fn float_round(mut handle: Handle) {
    let value = handle.receive().float().await;
    handle.provide_float(value.round());
}

async fn float_pow(mut handle: Handle) {
    let x = handle.receive().float().await;
    let y = handle.receive().float().await;
    handle.provide_float(x.powf(y));
}

async fn float_min(mut handle: Handle) {
    let x = handle.receive().float().await;
    let y = handle.receive().float().await;
    handle.provide_float(min_float(x, y));
}

async fn float_max(mut handle: Handle) {
    let x = handle.receive().float().await;
    let y = handle.receive().float().await;
    handle.provide_float(max_float(x, y));
}

async fn float_clamp(mut handle: Handle) {
    let value = handle.receive().float().await;
    let lo = handle.receive().float().await;
    let hi = handle.receive().float().await;
    handle.provide_float(max_float(min_float(value, hi), lo));
}

async fn float_equals(mut handle: Handle) {
    let x = handle.receive().float().await;
    let y = handle.receive().float().await;
    let tolerance = handle.receive().float().await;
    let equal = !x.is_nan()
        && !y.is_nan()
        && !tolerance.is_nan()
        && (x == y || (x - y).abs() <= tolerance.abs());
    signal_bool(handle, equal);
}

async fn float_sqrt(mut handle: Handle) {
    let value = handle.receive().float().await;
    handle.provide_float(value.sqrt());
}

async fn float_exp(mut handle: Handle) {
    let value = handle.receive().float().await;
    handle.provide_float(value.exp());
}

async fn float_ln(mut handle: Handle) {
    let value = handle.receive().float().await;
    handle.provide_float(value.ln());
}

async fn float_sin(mut handle: Handle) {
    let value = handle.receive().float().await;
    handle.provide_float(value.sin());
}

async fn float_cos(mut handle: Handle) {
    let value = handle.receive().float().await;
    handle.provide_float(value.cos());
}

async fn float_tan(mut handle: Handle) {
    let value = handle.receive().float().await;
    handle.provide_float(value.tan());
}

async fn float_atan2(mut handle: Handle) {
    let y = handle.receive().float().await;
    let x = handle.receive().float().await;
    handle.provide_float(y.atan2(x));
}
