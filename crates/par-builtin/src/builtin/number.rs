//package: core
use num_bigint::BigInt;
use par_runtime::external_def;
use par_runtime::primitive::Number;
use par_runtime::readback::Handle;

external_def! {
    @core/Number.{
        Zero_ => number_zero,
        Add => number_add,
        Sub => number_sub,
        Mul => number_mul,
        Div => number_div,
        Neg => number_neg,
    }
}

async fn number_zero(mut handle: Handle) {
    handle.receive().continue_();
    handle.provide_number(&Number::Zero);
}

async fn number_add(mut handle: Handle) {
    let mut pair = handle.receive();
    let left = pair.receive_number().await;
    let right = pair.number().await;

    let result = match (left, right) {
        (Number::Zero, Number::Zero) => Number::Zero,
        (Number::Zero, Number::Int(y)) => Number::Int(y),
        (Number::Int(x), Number::Zero) => Number::Int(x),
        (Number::Int(x), Number::Int(y)) => Number::Int(x + y),
        (Number::Zero, Number::Float(y)) => Number::Float(y),
        (Number::Float(x), Number::Zero) => Number::Float(x),
        (Number::Float(x), Number::Float(y)) => Number::Float(x + y),
        _ => panic!("Invalid generic number combination"),
    };

    handle.provide_number(&result);
}

async fn number_sub(mut handle: Handle) {
    let mut pair = handle.receive();
    let left = pair.receive_number().await;
    let right = pair.number().await;

    let result = match (left, right) {
        (Number::Zero, Number::Zero) => Number::Zero,
        (Number::Zero, Number::Int(y)) => Number::Int(-y),
        (Number::Int(x), Number::Zero) => Number::Int(x),
        (Number::Int(x), Number::Int(y)) => Number::Int(x - y),
        (Number::Zero, Number::Float(y)) => Number::Float(-y),
        (Number::Float(x), Number::Zero) => Number::Float(x),
        (Number::Float(x), Number::Float(y)) => Number::Float(x - y),
        _ => panic!("Invalid generic number combination"),
    };

    handle.provide_number(&result);
}

async fn number_mul(mut handle: Handle) {
    let mut pair = handle.receive();
    let left = pair.receive_number().await;
    let right = pair.number().await;

    let result = match (left, right) {
        (Number::Zero, Number::Zero) => Number::Zero,
        (Number::Zero, Number::Int(_)) | (Number::Int(_), Number::Zero) => {
            Number::Int(BigInt::ZERO)
        }
        (Number::Int(x), Number::Int(y)) => Number::Int(x * y),
        (Number::Zero, Number::Float(_)) | (Number::Float(_), Number::Zero) => Number::Float(0.0),
        (Number::Float(x), Number::Float(y)) => Number::Float(x * y),
        _ => panic!("Invalid generic number combination"),
    };

    handle.provide_number(&result);
}

async fn number_div(mut handle: Handle) {
    let mut pair = handle.receive();
    let left = pair.receive_number().await;
    let right = pair.number().await;

    let result = match (left, right) {
        (Number::Zero, Number::Zero) => Number::Zero,
        (Number::Zero, Number::Int(_)) => Number::Int(BigInt::ZERO),
        (Number::Int(_), Number::Zero) => Number::Int(BigInt::ZERO),
        (Number::Int(x), Number::Int(y)) => {
            if y == BigInt::ZERO {
                Number::Int(BigInt::ZERO)
            } else {
                Number::Int(x / y)
            }
        }
        (Number::Zero, Number::Float(y)) => Number::Float(0.0 / y),
        (Number::Float(x), Number::Zero) => Number::Float(x / 0.0),
        (Number::Float(x), Number::Float(y)) => Number::Float(x / y),
        _ => panic!("Invalid generic number combination"),
    };

    handle.provide_number(&result);
}

async fn number_neg(mut handle: Handle) {
    let value = handle.receive_number().await;

    let result = match value {
        Number::Zero => Number::Zero,
        Number::Int(value) => Number::Int(-value),
        Number::Float(value) => Number::Float(-value),
    };

    handle.provide_number(&result);
}
