use super::reducer::{NetHandle, ReducerMessage};
use super::runtime::{ExternalFn, Global, GlobalCont, Linear, Node, PackagePtr, Value};
use crate::atom::Atom;
use crate::data::Data;
use crate::flat::arena::Arena;
use crate::flat::runtime::Linker;
use crate::primitive::{Number, Primitive};
use futures::task::FutureObj;
use std::future::Future;
use std::mem;
use std::sync::{Arc, Mutex};

use crate::linker::Linked;
use tokio::sync::oneshot;

#[derive(Debug)]
pub enum Error {
    InvalidNode(Node<Linked>),
    InvalidValue(Value<Node<Linked>, Linked>),
    InvalidPrimitive(Primitive),
    Panicked,
}

pub(crate) type Result<T> = core::result::Result<T, Error>;

#[derive(Clone)]
struct HandleLinker {
    net: NetHandle,
    arena: Arc<Arena<Linked>>,
}

pub struct Handle {
    linker: HandleLinker,
    node: Box<Node<Linked>>,
}

fn linked_pair() -> (Node<Linked>, Node<Linked>) {
    let mutex = Arc::new(Mutex::new(None));
    (
        Node::Linear(Linear::Variable(mutex.clone())),
        Node::Linear(Linear::Variable(mutex)),
    )
}

impl Handle {
    fn new(&self, node: Node<Linked>) -> Self {
        Self {
            linker: self.linker.clone(),
            node: Box::new(node),
        }
    }

    pub fn from_node(arena: Arc<Arena<Linked>>, net: NetHandle, node: Node<Linked>) -> Self {
        Self {
            linker: HandleLinker { arena, net },
            node: Box::new(node),
        }
    }

    pub fn from_package(
        arena: Arc<Arena<Linked>>,
        net: NetHandle,
        package: PackagePtr<Linked>,
    ) -> Result<Handle> {
        let mut linker = HandleLinker { arena, net };
        let root = linker.instantiate_package_captures(
            package,
            Node::Linear(Linear::Value(Box::new(Value::Break))),
        );
        Ok(Self {
            linker,
            node: Box::new(root),
        })
    }

    pub fn link_with(mut self, dual: Handle) {
        self.linker.link(self.node, dual.node);
    }

    pub fn provide_external(mut self, ext: ExternalFn) {
        // TODO add fast variant.
        self.linker.link(
            self.node,
            Box::new(Node::Linear(Linear::Value(Box::new(Value::ExternalFn(
                ext,
            ))))),
        );
    }

    pub fn concurrently<F>(self, f: impl FnOnce(Self) -> F)
    where
        F: 'static + Send + Future<Output = ()>,
    {
        self.linker
            .net
            .0
            .clone()
            .send(ReducerMessage::Spawn(FutureObj::from(Box::new(f(self)))))
            .unwrap();
    }

    pub async fn await_ready(mut self) -> Self {
        let value = self.destruct().await;
        self.new(Node::Linear(Linear::Value(Box::new(value))))
    }

    pub fn provide_external_closure<Fun, Fut>(mut self, f: Fun)
    where
        Fun: 'static + Send + Sync + Fn(Handle) -> Fut,
        Fut: 'static + Send + Future<Output = ()>,
    {
        self.linker.link(
            self.node,
            Box::new(Node::Linear(Linear::Value(Box::new(Value::ExternalArc(
                super::runtime::ExternalArc(Arc::new(move |handle| Box::pin(f(handle.handle)))),
            ))))),
        );
    }

    pub fn provide_primitive(mut self, primitive: Primitive) {
        // TODO add fast variant.
        self.linker.link(
            self.node,
            Box::new(Node::Linear(Linear::Value(Box::new(Value::Primitive(
                primitive,
            ))))),
        );
    }

    pub fn provide_number(self, number: &Number) {
        self.provide_primitive(match number {
            Number::Zero => Primitive::Number(Number::Zero),
            Number::Int(value) => Primitive::Number(Number::Int(value.clone())),
            Number::Float(value) => Primitive::Number(Number::Float(*value)),
        })
    }

    pub fn provide_data(mut self, data: &Data) {
        let node = self.data_node(data);
        self.linker.link(self.node, Box::new(node));
    }

    pub async fn primitive(mut self) -> Result<Primitive> {
        let primitive = match self.destruct().await {
            Value::Primitive(p) => p,
            node => return Err(Error::InvalidValue(node)),
        };
        Ok(primitive)
    }

    pub async fn number(self) -> Result<Number> {
        let primitive = self.primitive().await?;
        match primitive {
            Primitive::Number(n) => Ok(n),
            primitive => Err(Error::InvalidPrimitive(primitive)),
        }
    }

    pub async fn data(mut self) -> Result<Data> {
        let value = self.destruct().await;
        self.data_from_value(value).await
    }

    pub fn send(&mut self) -> Self {
        let (left, left_h) = linked_pair();
        let (right, right_h) = linked_pair();
        let par = core::mem::replace(&mut self.node, Box::new(left_h));
        let times = Node::Linear(Linear::Value(Box::new(Value::Pair(left, right))));
        self.linker.link(par, Box::new(times));
        self.new(right_h)
    }

    pub fn send_data(&mut self, data: &Data) {
        self.send().provide_data(data);
    }

    pub fn send_number(&mut self, number: &Number) {
        self.send().provide_number(number);
    }

    pub fn receive(&mut self) -> Self {
        let node = mem::replace(self.node.as_mut(), Node::Empty);
        match self.linker.destruct(node) {
            Ok(value) => {
                let Value::Pair(a, b) = value else {
                    unreachable!()
                };
                let _ = mem::replace(self.node.as_mut(), a);
                return self.new(b);
            }
            Err(node) => self.node = Box::new(node),
        }
        let (left, left_h) = linked_pair();
        let (right, right_h) = linked_pair();
        let times = core::mem::replace(&mut self.node, Box::new(left_h));
        let par = Node::Linear(Linear::Par(Box::new(left), Box::new(right)));
        self.linker.link(times, Box::new(par));
        self.new(right_h)
    }

    pub async fn receive_data(&mut self) -> Result<Data> {
        let value = self.destruct().await;
        let Value::Pair(left, right) = value else {
            return Err(Error::InvalidValue(value));
        };
        self.node = Box::new(left);
        self.new(right).data().await
    }

    pub async fn receive_number(&mut self) -> Result<Number> {
        let value = self.destruct().await;
        let Value::Pair(left, right) = value else {
            return Err(Error::InvalidValue(value));
        };
        self.node = Box::new(left);
        self.new(right).number().await
    }

    pub fn signal(&mut self, chosen: Atom) {
        let (payload, payload_h) = linked_pair();
        let either = Node::Linear(Linear::Value(Box::new(Value::Either(chosen, payload))));
        let choice = core::mem::replace(&mut self.node, Box::new(payload_h));
        self.linker.link(choice, Box::new(either));
    }

    pub async fn case(&mut self) -> Atom {
        let linker = self.linker.clone();

        let Value::Either(name, payload) = (*self).destruct().await else {
            unreachable!()
        };
        *self = Handle {
            linker,
            node: Box::new(payload),
        };
        name
    }

    pub fn break_(mut self) {
        match self.node.as_ref() {
            Node::Global(_, global_index)
                if matches!(
                    self.linker.arena.get(*global_index),
                    Global::Destruct(GlobalCont::Continue)
                ) =>
            {
                ()
            }
            Node::Linear(Linear::Continue) => (),
            _ => {
                let other = Node::Linear(Linear::Value(Box::new(Value::Break)));
                self.linker.link(self.node, Box::new(other));
            }
        }
    }

    pub fn continue_(mut self) {
        match self.node.as_ref() {
            Node::Global(_, global_index)
                if matches!(
                    self.linker.arena.get(*global_index),
                    Global::Value(Value::Break)
                ) =>
            {
                ()
            }
            Node::Linear(Linear::Value(value)) if matches!(value.as_ref(), Value::Break) => (),
            _ => {
                let other = Node::Linear(Linear::Continue);
                self.linker.link(self.node, Box::new(other));
            }
        }
    }

    pub fn erase(mut self) -> () {
        let (other, _) = self.linker.create_share_hole();
        self.linker.link(self.node, Box::new(other))
    }

    pub fn duplicate(&mut self) -> Handle {
        let (other, shared) = self.linker.create_share_hole();
        let node = core::mem::replace(
            &mut self.node,
            Box::new(Node::Shared(shared.clone())).into(),
        );
        self.linker.link(node, Box::new(other));
        self.new(Node::Shared(shared).into())
    }

    async fn destruct(&mut self) -> Value<Node<Linked>, Linked> {
        let node = mem::replace(self.node.as_mut(), Node::Empty);
        let node = self.linker.deref(node);
        match self.linker.destruct(node) {
            Ok(value) => {
                return value;
            }
            Err(node) => {
                let (tx, rx) = oneshot::channel();
                self.linker
                    .link(Box::new(Node::Linear(Linear::Request(tx))), Box::new(node));
                rx.await.unwrap()
            }
        }
    }

    fn data_node(&self, data: &Data) -> Node<Linked> {
        match data {
            Data::Unit => Node::Linear(Linear::Value(Box::new(Value::Break))),
            Data::Pair(left, right) => Node::Linear(Linear::Value(Box::new(Value::Pair(
                self.data_node(right),
                self.data_node(left),
            )))),
            Data::Either(label, payload) => Node::Linear(Linear::Value(Box::new(Value::Either(
                label.clone(),
                self.data_node(payload),
            )))),
            Data::Primitive(primitive) => {
                Node::Linear(Linear::Value(Box::new(Value::Primitive(primitive.clone()))))
            }
        }
    }

    async fn data_from_value(&self, value: Value<Node<Linked>, Linked>) -> Result<Data> {
        match value {
            Value::Break => Ok(Data::Unit),
            Value::Pair(left_node, right_node) => {
                let right = Box::pin(self.new(left_node).data()).await?;
                let left = Box::pin(self.new(right_node).data()).await?;
                Ok(Data::Pair(Box::new(left), Box::new(right)))
            }
            Value::Either(name, payload) => {
                let payload = Box::pin(self.new(payload).data()).await?;
                Ok(Data::Either(name, Box::new(payload)))
            }
            Value::Primitive(primitive) => Ok(Data::Primitive(primitive)),
            other => Err(Error::InvalidValue(other)),
        }
    }
}

impl Linker for HandleLinker {
    fn link(&mut self, a: Box<Node<Linked>>, b: Box<Node<Linked>>) {
        self.net.0.send(ReducerMessage::Redex(a, b)).unwrap()
    }

    fn arena(&self) -> Arc<Arena<Linked>> {
        self.arena.clone()
    }
}
