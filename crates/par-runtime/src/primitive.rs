use std::{
    cmp::Ordering,
    fmt::{self, Write},
    ops::RangeBounds,
};

use bytes::Bytes;
use num_bigint::BigInt;
use serde::{Deserialize, Serialize};

fn scan_digit_run(input: &str, start: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    if !matches!(bytes.get(start), Some(b'0'..=b'9')) {
        return None;
    }

    let mut idx = start + 1;
    while let Some(&byte) = bytes.get(idx) {
        match byte {
            b'0'..=b'9' => idx += 1,
            b'_' if matches!(bytes.get(idx + 1), Some(b'0'..=b'9')) => idx += 1,
            _ => break,
        }
    }

    Some(idx)
}

fn parse_decimal_float(input: &str) -> Option<f64> {
    let bytes = input.as_bytes();
    let mut idx = 0;

    if matches!(bytes.get(idx), Some(b'+' | b'-')) {
        idx += 1;
    }

    idx = scan_digit_run(input, idx)?;
    if !matches!(bytes.get(idx), Some(b'.')) {
        return None;
    }
    idx += 1;
    idx = scan_digit_run(input, idx)?;

    if matches!(bytes.get(idx), Some(b'e' | b'E')) {
        idx += 1;
        if matches!(bytes.get(idx), Some(b'+' | b'-')) {
            idx += 1;
        }
        idx = scan_digit_run(input, idx)?;
    }

    if idx != bytes.len() {
        return None;
    }

    let normalized: String = input.chars().filter(|&c| c != '_').collect();
    normalized.parse::<f64>().ok()
}

pub fn parse_float_text(input: &str) -> Option<f64> {
    match input {
        "NaN" => Some(f64::NAN),
        "Inf" | "Infinity" => Some(f64::INFINITY),
        "-Inf" | "-Infinity" => Some(f64::NEG_INFINITY),
        _ => parse_decimal_float(input),
    }
}

pub fn format_float(value: f64) -> String {
    if value.is_nan() {
        return String::from("NaN");
    }
    if value == f64::INFINITY {
        return String::from("Inf");
    }
    if value == f64::NEG_INFINITY {
        return String::from("-Inf");
    }

    let mut text = format!("{value:?}");
    if let Some(exp_idx) = text.find(['e', 'E']) {
        if !text[..exp_idx].contains('.') {
            text.insert_str(exp_idx, ".0");
        }
    } else if !text.contains('.') {
        text.push_str(".0");
    }
    text
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Primitive {
    Number(Number),
    Sequence(ByteSequence),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ByteSequence(ByteSequenceRepr);

#[derive(Clone, Debug, Serialize, Deserialize)]
enum ByteSequenceRepr {
    String(ParString),
    Bytes(Bytes),
}

impl ByteSequence {
    pub fn string(value: ParString) -> Self {
        Self(ByteSequenceRepr::String(value))
    }

    pub fn bytes(value: Bytes) -> Self {
        Self(ByteSequenceRepr::Bytes(value))
    }

    pub fn as_bytes(&self) -> &[u8] {
        match &self.0 {
            ByteSequenceRepr::String(value) => value.as_str().as_bytes(),
            ByteSequenceRepr::Bytes(value) => value.as_ref(),
        }
    }

    pub fn into_bytes(self) -> Bytes {
        match self.0 {
            ByteSequenceRepr::String(value) => value.as_bytes(),
            ByteSequenceRepr::Bytes(value) => value,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match &self.0 {
            ByteSequenceRepr::String(value) => Some(value.as_str()),
            ByteSequenceRepr::Bytes(_) => None,
        }
    }

    pub fn into_string(self) -> Result<ParString, Self> {
        match self.0 {
            ByteSequenceRepr::String(value) => Ok(value),
            ByteSequenceRepr::Bytes(value) => Err(Self::bytes(value)),
        }
    }
}

impl PartialEq for ByteSequence {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for ByteSequence {}

impl PartialOrd for ByteSequence {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ByteSequence {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Number {
    Zero,
    Int(BigInt),
    Float(f64),
}

impl PartialEq for Number {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Number {}

impl PartialOrd for Number {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Number {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Zero, Self::Zero) => Ordering::Equal,
            (Self::Zero, Self::Int(right)) => BigInt::ZERO.cmp(right),
            (Self::Int(left), Self::Zero) => left.cmp(&BigInt::ZERO),
            (Self::Zero, Self::Float(right)) => 0.0f64.total_cmp(right),
            (Self::Float(left), Self::Zero) => left.total_cmp(&0.0),
            (Self::Int(left), Self::Int(right)) => left.cmp(right),
            (Self::Float(left), Self::Float(right)) => left.total_cmp(right),
            (left, right) => number_kind_rank(left).cmp(&number_kind_rank(right)),
        }
    }
}

impl Primitive {
    pub fn string(value: ParString) -> Self {
        Self::Sequence(ByteSequence::string(value))
    }

    pub fn bytes(value: Bytes) -> Self {
        Self::Sequence(ByteSequence::bytes(value))
    }

    pub fn pretty(&self, f: &mut impl Write, _indent: usize) -> fmt::Result {
        match self {
            Self::Number(Number::Zero) => write!(f, "0"),
            Self::Number(Number::Int(i)) => write!(f, "{}", i),
            Self::Number(Number::Float(value)) => write!(f, "{}", format_float(*value)),
            Self::Sequence(sequence) => match sequence.as_str() {
                Some(value) => write!(f, "{:?}", value),
                None => {
                    write!(f, "<<")?;
                    for (i, &byte) in sequence.as_bytes().iter().enumerate() {
                        if i > 0 {
                            write!(f, " ")?;
                        }
                        write!(f, "{}", byte as i64)?;
                    }
                    write!(f, ">>")
                }
            },
        }
    }

    #[cfg(feature = "playground")]
    pub fn pretty_string(&self) -> String {
        let mut buf = String::new();
        self.pretty(&mut buf, 0).unwrap();
        buf
    }
}

fn number_kind_rank(number: &Number) -> u8 {
    match number {
        Number::Zero => 0,
        Number::Int(_) => 1,
        Number::Float(_) => 2,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct ParString(bytes_utils::Str);

impl ParString {
    pub fn copy_from_slice(slice: &str) -> ParString {
        Self(slice.to_string().into())
    }

    pub fn from_utf8_lossy(v: Bytes) -> ParString {
        let s = bytes_utils::Str::from_inner(v)
            .unwrap_or_else(|e| String::from_utf8_lossy(&e.into_inner()).into_owned().into());
        Self(s)
    }

    pub fn from_owner<T>(owner: T) -> ParString
    where
        T: AsRef<str> + Send + 'static,
    {
        struct Wrapper<T>(T);
        impl<T: AsRef<str>> AsRef<[u8]> for Wrapper<T> {
            fn as_ref(&self) -> &[u8] {
                self.0.as_ref().as_bytes()
            }
        }
        let bytes = Bytes::from_owner(Wrapper(owner));
        // SAFETY: the bytes content of `bytes` is derived from a `str`,
        //         and thus must be valid UTF-8.
        Self(unsafe { bytes_utils::Str::from_inner_unchecked(bytes) })
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn as_bytes(&self) -> Bytes {
        self.0.inner().clone()
    }

    pub fn substr(&self, range: impl RangeBounds<usize>) -> Self {
        let start = range.start_bound().cloned();
        let end = range.end_bound().cloned();
        Self(self.0.slice((start, end)))
    }
}

impl From<&'static str> for ParString {
    fn from(s: &'static str) -> Self {
        Self(bytes_utils::Str::from_static(s))
    }
}

impl From<String> for ParString {
    fn from(s: String) -> Self {
        Self(s.into())
    }
}

impl From<char> for ParString {
    fn from(c: char) -> Self {
        ParString::copy_from_slice(c.encode_utf8(&mut [0u8; 4]))
    }
}
