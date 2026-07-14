//package: core
use std::f64::consts;

use arcstr::literal;
use num_bigint::{BigInt, Sign};
use num_traits::{FromPrimitive, ToPrimitive};
use par_core::frontend::{ExternalTypeDef, PrimitiveType, Type};
use par_core::source::Span;
use par_runtime::external_def;
use par_runtime::primitive::parse_float_text;
use par_runtime::readback::Handle;
use par_runtime::registry::{DefinitionRef, PackageRef};

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

external_def! {
    @core/Float.{
        NaN_ => float_nan,
        Inf_ => float_inf,
        NegInf_ => float_neg_inf,
        Pi_ => float_pi,
        E_ => float_e,
        IsNaN => float_is_nan,
        IsFinite => float_is_finite,
        IsInfinite => float_is_infinite,
        FromInt => float_from_int,
        ToInt => float_to_int,
        FromString => float_from_string,
        Neg => float_neg,
        Abs => float_abs,
        Floor => float_floor,
        Ceil => float_ceil,
        Round => float_round,
        Pow => float_pow,
        Min => float_min,
        Max => float_max,
        Clamp => float_clamp,
        Equals => float_equals,
        Sqrt => float_sqrt,
        Exp => float_exp,
        Ln => float_ln,
        Sin => float_sin,
        Cos => float_cos,
        Tan => float_tan,
        Atan2 => float_atan2,
    }
}

fn signal_bool(mut handle: Handle, value: bool) {
    if value {
        handle.signal(literal!("true"));
    } else {
        handle.signal(literal!("false"));
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
            handle.signal(literal!("some"));
            handle.provide_float(value);
        }
        None => {
            handle.signal(literal!("none"));
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
