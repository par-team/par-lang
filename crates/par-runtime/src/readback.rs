pub use crate::data::Data;
pub use crate::primitive::Number;

use crate::primitive::{ParString, Primitive};
use arcstr::ArcStr;
use bytes::Bytes;
use num_bigint::{BigInt, BigUint};

use std::future::Future;

pub struct Handle {
    pub handle: super::flat::readback::Handle,
}

impl From<super::flat::readback::Handle> for Handle {
    fn from(value: super::flat::readback::Handle) -> Self {
        Self { handle: value }
    }
}

impl Handle {
    pub fn erase(self) {
        self.handle.erase()
    }

    pub fn break_(self) {
        self.handle.break_()
    }

    pub fn continue_(self) {
        self.handle.continue_()
    }

    pub async fn case(&mut self) -> ArcStr {
        self.handle.case().await
    }

    pub fn signal(&mut self, s: ArcStr) {
        self.handle.signal(s)
    }

    pub fn send(&mut self) -> Handle {
        Handle::from(self.handle.send())
    }

    pub fn receive(&mut self) -> Handle {
        Handle::from(self.handle.receive())
    }

    pub async fn receive_number(&mut self) -> Number {
        self.handle.receive_number().await.unwrap()
    }

    pub async fn receive_data(&mut self) -> Data {
        self.handle.receive_data().await.unwrap()
    }

    pub fn duplicate(&mut self) -> Handle {
        Handle::from(self.handle.duplicate())
    }

    pub fn provide_int(self, value: BigInt) {
        self.handle
            .provide_primitive(Primitive::Number(Number::Int(value)))
    }

    pub fn provide_nat(self, value: BigUint) {
        self.handle
            .provide_primitive(Primitive::Number(Number::Int(value.into())))
    }

    pub fn provide_float(self, value: f64) {
        self.handle
            .provide_primitive(Primitive::Number(Number::Float(value)))
    }

    pub fn provide_string(self, value: ParString) {
        self.handle.provide_primitive(Primitive::string(value))
    }

    pub fn provide_bytes(self, value: Bytes) {
        self.handle.provide_primitive(Primitive::bytes(value))
    }

    pub fn provide_char(self, value: char) {
        self.handle
            .provide_primitive(Primitive::string(value.into()))
    }

    pub fn provide_byte(self, value: u8) {
        self.handle
            .provide_primitive(Primitive::bytes(Bytes::copy_from_slice(&[value])))
    }

    pub fn send_number(&mut self, value: &Number) {
        self.handle.send_number(value)
    }

    pub fn provide_number(self, value: &Number) {
        self.handle.provide_number(value)
    }

    pub fn send_data(&mut self, value: &Data) {
        self.handle.send_data(value)
    }

    pub fn provide_data(self, value: &Data) {
        self.handle.provide_data(value)
    }

    pub async fn byte(self) -> u8 {
        let primitive = self.handle.primitive().await.unwrap();
        let Primitive::Sequence(value) = primitive else {
            panic!(
                "Unexpected primitive in Handle! Expected Bytes, got {:?}",
                primitive
            )
        };
        let value = value.into_bytes();
        assert!(value.len() == 1);
        value.as_ref()[0]
    }

    pub async fn char(self) -> char {
        let primitive = self.handle.primitive().await.unwrap();
        let Primitive::Sequence(value) = primitive else {
            panic!(
                "Unexpected primitive in Handle! Expected String, got {:?}",
                primitive
            )
        };
        let value = value.into_string().unwrap_or_else(|value| {
            panic!(
                "Unexpected byte sequence in Handle! Expected String, got {:?}",
                value
            )
        });
        let mut chars = value.as_str().chars();
        let ch = chars.next().unwrap();
        assert!(chars.next().is_none());
        ch
    }

    pub async fn string(self) -> ParString {
        let primitive = self.handle.primitive().await.unwrap();
        let Primitive::Sequence(value) = primitive else {
            panic!(
                "Unexpected primitive in Handle! Expected String, got {:?}",
                primitive
            )
        };
        value.into_string().unwrap_or_else(|value| {
            panic!(
                "Unexpected byte sequence in Handle! Expected String, got {:?}",
                value
            )
        })
    }

    pub async fn bytes(self) -> Bytes {
        match self.handle.primitive().await.unwrap() {
            Primitive::Sequence(value) => value.into_bytes(),
            primitive => panic!(
                "Unexpected primitive in Handle! Expected Bytes, got {:?}",
                primitive
            ),
        }
    }

    pub async fn int(self) -> BigInt {
        match self.number().await {
            Number::Zero => BigInt::ZERO,
            Number::Int(value) => value,
            number => panic!(
                "Unexpected number in Handle! Expected Int, got {:?}",
                number
            ),
        }
    }

    pub async fn float(self) -> f64 {
        match self.number().await {
            Number::Zero => 0.0,
            Number::Float(value) => value,
            number => panic!(
                "Unexpected number in Handle! Expected Float, got {:?}",
                number
            ),
        }
    }

    pub async fn nat(self) -> BigUint {
        use num_bigint::Sign::*;
        match self.number().await {
            Number::Zero => BigUint::ZERO,
            Number::Int(value) if matches!(value.sign(), NoSign | Plus) => value.into_parts().1,
            number => panic!(
                "Unexpected number in Handle! Expected Int, got {:?}",
                number
            ),
        }
    }

    pub async fn number(self) -> Number {
        self.handle.number().await.unwrap()
    }

    pub async fn data(self) -> Data {
        self.handle.data().await.unwrap()
    }

    pub fn link(self, dual: Handle) {
        self.handle.link_with(dual.handle)
    }

    pub fn concurrently<F>(self, f: impl FnOnce(Self) -> F)
    where
        F: 'static + Send + Future<Output = ()>,
    {
        self.handle.concurrently(move |handle| f(Handle { handle }))
    }

    pub fn provide_box<Fun, Fut>(self, f: Fun)
    where
        Fun: 'static + Send + Sync + Fn(Handle) -> Fut,
        Fut: 'static + Send + Future<Output = ()>,
    {
        self.handle
            .provide_external_closure(move |handle| f(Handle { handle }))
    }
}
