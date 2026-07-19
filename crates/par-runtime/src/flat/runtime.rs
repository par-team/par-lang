//! This is the V2 runtime for the Par language
//!
//! Unlike with the old runtime, program state is strictly separated with
//! immutable global program code. The global nodes are stored in an [`Arena`].
//! Global nodes are attached to a piece of state which is a pointer to its [`Instance`]
//!
//! An `Instance` is an array of variable slots; which might be either empty or filled with a [`Value`]
//! Instances are used to store program state within each global package, and also
//! for communication within each global packages.
//!
//! Global nodes are all stored contiguously, and a definition's state is usually within the same Instance, which means
//! that the runtime enjoys cache locality.
//!
//! Additionally, state can be stored in either [`Shared`] nodes or [`Linear`] nodes.
//!
//! `Linear` nodes are temporarily created by readback to interact with the program. They are usually stored in `Box`es
//!  Additionally, ShareHoles are a type of linear value which is created when attempting to duplicate a global variable.
//!  It is roughly the dual of a fanout node.
//!
//! `Shared` nodes are created when a node is duplicated. They are reference-counted and thus, copying them is cheap. They are destroyed
//! by interacting them with a Global node that destructures values, such as a Par node or a Choice node. This destructures the shared node,
//! but the children will still be reference-counted. This allows cheap copying of runtime values, which is something the old
//! runtime was not able to do.
//!
//! The runtime is handled by the [`Runtime`] struct. It tracks the program's redexes, which
//! are pairs of nodes that interact with each other, and reduces them until the program is finished.
//! The `Runtime` does not know what to do with IO operations; that is handled by the `Reducer`

use core::panic;
use std::fmt::Debug;
use std::sync::OnceLock;

use crate::flat::show::Showable;
use crate::flat::show::Shower;
use crate::primitive::Primitive;

use super::arena::*;
use crate::fan_behavior::FanBehavior;
use crate::flat::stats::Rewrites;
use crate::linker::Linked;
use atomicbox::AtomicOptionBox;
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering::AcqRel;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot::Sender;

pub type PackagePtr<Ext> = Index<Ext, OnceLock<Package<Ext>>>;
pub(crate) type GlobalPtr<Ext> = Index<Ext, Global<Ext>>;
type Str<Ext> = Index<Ext, str>;

#[derive(Debug)]
struct InstanceInner(Box<[AtomicOptionBox<Node<Linked>>]>);

#[derive(Clone, Debug)]
/// An `Instance` stores the state associated to an instance of a Global node.
///
/// Instances can be cheaply cloned shallowly because they are reference counter; this creates another instance pointing to the same underyling
/// [`InstanceInner`].
///
/// They act as the backing store for Global nodes. Specifically, [`Global::Variable`] nodes
/// are indices into the array inside the `Instance`. Each instance of a global node
/// also holds an Instance. It is in this instance where the state of the variables is stored
///
/// ## Lifetime of an `Instance`
///
/// They are created empty by `Linker::create_package_instance`, with a fixed length. Initially, all variables are empty.
/// Then, calls to `Runtime::set_var` slowly fill up and empty the instance. Each variable slot is filled once and taken out of once.
/// (I'm not sure if it's possible for a variable slot to never be used, but it definitely can't be linked to twice)
/// At the end of the lifetime of Instances, all of them go out of scope and are eventually dropped. Because of the way
/// the runtime is designed, all slots inside of it must be empty. This is when the `Instance` is destroyed.
pub struct Instance {
    vars: Arc<InstanceInner>,
}

impl Instance {
    fn identifier(&self) -> usize {
        (Arc::as_ptr(&self.vars) as usize >> 3) & 0xFF
    }
}

impl Drop for InstanceInner {
    fn drop(&mut self) {
        // This is a debugging tool to detect leaks.
        // However it causes a panic in certain cases when cancelling a run so it's commented for now
        // see issue: https://github.com/par-team/par-lang/issues/165 as an example
        // for i in self.0.lock().unwrap().as_mut().iter() {
        //     if !i.is_none() {
        //         panic!(
        //             "Data was leaked in an Instance:
        //             {i:?}
        //         "
        //         )
        //     }
        // }
    }
}

#[derive(Debug)]
/// User data; this is data created by the external environment.
pub enum UserData {
    /// An external function without captures. This is created externally
    /// and the function takes in a handle to the value it interacts with
    ExternalFn(ExternalFn),
    /// An external function with shared captured. This is created externally
    ExternalArc(ExternalArc),
}

pub(crate) type ExternalFnRet = std::pin::Pin<Box<dyn Send + std::future::Future<Output = ()>>>;
pub(crate) type ExternalFn = fn(crate::readback::Handle) -> ExternalFnRet;

#[derive(Clone)]
pub struct ExternalArc(pub Arc<dyn Send + Sync + Fn(crate::readback::Handle) -> ExternalFnRet>);

impl Serialize for ExternalArc {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        unreachable!(
            "ExternalArc should never be serialized, are you trying to serialize after reduction?"
        )
    }
}

impl<'de> Deserialize<'de> for ExternalArc {
    fn deserialize<D>(_deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        unreachable!(
            "ExternalArc should never be deserialized, are you trying to deserialize after reduction?"
        )
    }
}

impl Debug for ExternalArc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExternalArc").finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
/// A PackageBody is the inner body of a package.
/// The difference with `Package` is that `PackageBody` does not contain `num_vars`
/// (because a package inside a PackageBody might not require a new instance)
pub struct PackageBody<Ext: Clone> {
    pub root: Index<Ext, Global<Ext>>,
    pub captures: Index<Ext, Global<Ext>>,

    pub debug_name: String,
    // TODO: Store this inline in the arena.
    pub redexes: Index<Ext, [(Index<Ext, Global<Ext>>, Index<Ext, Global<Ext>>)]>,
}

#[derive(Clone, Copy)]
pub(crate) struct LinkedPackageBody {
    root: GlobalPtr<Linked>,
    captures: GlobalPtr<Linked>,
    redexes: Index<Linked, [(GlobalPtr<Linked>, GlobalPtr<Linked>)]>,
}

impl From<&PackageBody<Linked>> for LinkedPackageBody {
    fn from(body: &PackageBody<Linked>) -> Self {
        Self {
            root: body.root,
            captures: body.captures,
            redexes: body.redexes,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
/// A package is a `Global` subgraph that is isolated from the rest of the program
/// It does not use the same Instance as its environment, and so it requires a
/// separate Instance to be created when it is expanded.
pub struct Package<Ext: Clone> {
    pub body: PackageBody<Ext>,
    /// How large the Instance must be.
    pub num_vars: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Global<Ext: Clone> {
    Variable(usize),
    Package(PackagePtr<Ext>, GlobalPtr<Ext>, FanBehavior),
    /// Destruct attempts to convert the interacting node into a value,
    /// and then carries out a negative operation on it according to its variant
    /// This node is created from the Continue, Case, and Receive commands.
    Destruct(GlobalCont<Ext>),
    /// Value carries out a positive operation.
    /// This node is created from the Break, Signal, and Send commands.
    Value(GlobalValue<Ext>),
    /// Fanout; turn the value it interacts with into a nonlinear value.
    /// This node is created whenever a Par variable is used
    /// nonlinearly. Fanout subsumes both erasure and duplication.
    Fanout(Index<Ext, [Global<Ext>]>),
}

#[derive(Debug)]
pub enum Node<Ext: Clone> {
    Empty,
    Linear(Linear<Ext>),
    Shared(Shared<Ext>),
    Global(Instance, Index<Ext, Global<Ext>>),
}

#[derive(Clone, Debug)]
/// Shared nodes are created by Fanout to make duplication fast.
/// They are reference-counted internally and can be cheaply cloned.
pub enum Shared<Ext: Clone> {
    /// Async values are created when sharing
    /// something that is not yet ready; a variable or a request.
    Async(Arc<Mutex<SharedHole<Ext>>>),
    /// Sync values are created when sharing
    /// a value or a package.
    Sync(Arc<SyncShared<Ext>>),
}

/// A "Value" parametrized by a "pointer type P". This is used to avoid code duplication between
/// different value containers
/// Values are either positive types or copyable external functions. They can all be duplicated if their
/// subnodes are duplicable too.
///
/// P is the type of children. Usually, this will be `GlobalPtr`, `Shared`, or `Node`
///
/// There are [`Linear`] values, [`Shared`] values, and [`Global`] values.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Value<P, Ext: Clone> {
    /// The break node; created in break commands (`value!`)
    Break,
    /// The pair node; created in send commands (`value(a)`)
    Pair(P, P),
    /// The either node; created in signal commands (`value.name`)
    Either(Str<Ext>, P),
    /// ExternalFns contain `fn` nodes which are called with a handle to value they interact with
    ExternalFn(Ext),
    /// ExternalFns contain `fn` nodes which are called with a handle to value they interact with
    /// They internally contain a `Fn` dyn object which allows this type of functions to have captures.
    ExternalArc(ExternalArc),
    /// Primitive values are data provided externally.
    Primitive(Primitive),
}

impl<P, Ext: Clone> Value<P, Ext> {
    pub fn map_leaves<Q>(self, mut f: impl FnMut(P) -> Option<Q>) -> Option<Value<Q, Ext>> {
        Some(match self {
            Value::Break => Value::Break,
            Value::Pair(a, b) => Value::Pair(f(a)?, f(b)?),
            Value::Either(s, v) => Value::Either(s, f(v)?),
            Value::ExternalFn(e) => Value::ExternalFn(e),
            Value::ExternalArc(e) => Value::ExternalArc(e),
            Value::Primitive(primitive) => Value::Primitive(primitive),
        })
    }

    pub fn map_ref_leaves<Q>(&self, mut f: impl FnMut(&P) -> Option<Q>) -> Option<Value<Q, Ext>> {
        Some(match self {
            Value::Break => Value::Break,
            Value::Pair(a, b) => Value::Pair(f(a)?, f(b)?),
            Value::Either(s, v) => Value::Either(*s, f(v)?),
            Value::ExternalFn(e) => Value::ExternalFn(e.clone()),
            Value::ExternalArc(e) => Value::ExternalArc(e.clone()),
            Value::Primitive(primitive) => Value::Primitive(primitive.clone()),
        })
    }
}

#[derive(Clone, Debug)]
pub enum SyncShared<Ext: Clone> {
    Package(PackagePtr<Ext>, Shared<Ext>),
    Value(Value<Shared<Ext>, Ext>),
}

pub type GlobalValue<Ext> = Value<GlobalPtr<Ext>, Ext>;
#[derive(Debug)]
/// Linear nodes are not stored in the global arena; instead, they
/// are created by the runtime and by the external dynamically, as needed
pub enum Linear<Ext: Clone> {
    Value(Box<Value<Node<Ext>, Ext>>),
    /// This variant is created by external
    /// tasks. Whatever node it interacts with will get sent to `Node`
    /// This is not true for variable, package, or fanout nodes, which
    /// are of a higher priority than Request nodes.
    Continue,
    Par(Box<Node<Ext>>, Box<Node<Ext>>),
    Request(Sender<Value<Node<Ext>, Ext>>),
    /// This variant is created on `Fanout` ~ `Variable` interactions
    /// and is substituted into the variable's slot
    /// It is a "hole" that will get filled with whatever
    /// value the variable is filled with
    /// This allows "suspending" the duplication
    /// until we know what to duplicate.
    ///
    /// This is also created in Fanout ~ Request interactions
    ShareHole(Arc<Mutex<SharedHole<Linked>>>),
    // This variant is similiar to Global::Variable in function
    // but is used by external tasks.
    Variable(Arc<Mutex<Option<Node<Ext>>>>),
}

impl From<UserData> for Linear<Linked> {
    fn from(this: UserData) -> Linear<Linked> {
        match this {
            UserData::ExternalFn(p) => Linear::Value(Box::new(Value::ExternalFn(p))),
            UserData::ExternalArc(p) => Linear::Value(Box::new(Value::ExternalArc(p))),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
/// A "global continuation"; a negative node stored in the global array
/// When it interacts with a value, it attempts to destructure it.
pub enum GlobalCont<Ext: Clone> {
    /// The continue node; created in continue commands (`value?`)
    Continue,
    /// The par node; created in receive commands (`value[a]`)
    Par(GlobalPtr<Ext>, GlobalPtr<Ext>),
    /// The choice node; created in case commands (`value.case { ... }`)
    Choice(GlobalPtr<Ext>, Index<Ext, [(Str<Ext>, PackageBody<Ext>)]>),
}

#[derive(Debug)]
pub enum SharedHole<Ext: Clone> {
    Filled(SyncShared<Ext>),
    Unfilled(Vec<Node<Ext>>),
}
pub struct Runtime {
    pub arena: Arc<Arena<Linked>>,
    pub redexes: Vec<(Node<Linked>, Node<Linked>)>,
    pub rewrites: Rewrites,
}

/// This trait is implemented by everything that knows how to link two nodes together
/// and that holds a pointer to the arena. This prevents duplication between [`Runtime`] and
/// [`Handle`], which are the two implementors of this trait.
pub(crate) trait Linker {
    fn link(&mut self, a: Node<Linked>, b: Node<Linked>);
    fn arena(&self) -> Arc<Arena<Linked>>;

    fn show<'a, 'b>(&'b self, node: &'a Node<Linked>) -> String {
        let arena_ref = self.arena();
        format!("{}", Showable(node, &mut Shower::from_arena(&arena_ref)))
    }
    fn destruct(
        &mut self,
        node: Node<Linked>,
    ) -> Result<Value<Node<Linked>, Linked>, Node<Linked>> {
        match node {
            Node::Linear(Linear::Value(v)) => Ok(*v),
            Node::Shared(Shared::Sync(shared)) => match &*shared {
                SyncShared::Package(package, shared) => {
                    let Node::Global(instance, global_index) =
                        self.instantiate_package_captures(*package, Node::Shared(shared.clone()))
                    else {
                        unreachable!("should be a global")
                    };
                    match self.arena().get(global_index) {
                        Global::Value(v) => Ok(v
                            .map_ref_leaves(|x| Some(Node::Global(instance.clone(), *x)))
                            .unwrap()),
                        _ => Err(Node::Global(instance, global_index)),
                    }
                }
                SyncShared::Value(shared) => Ok(shared
                    .clone()
                    .map_leaves(|x| Some(Node::Shared(x)))
                    .unwrap()),
            },
            Node::Global(instance, global_index) => match self.arena().get(global_index) {
                Global::Value(v) => Ok(v
                    .map_ref_leaves(|x| Some(Node::Global(instance.clone(), *x)))
                    .unwrap()),
                _ => Err(Node::Global(instance, global_index)),
            },
            node => Err(node),
        }
    }

    fn deref(&mut self, node: Node<Linked>) -> Node<Linked> {
        let arena = self.arena();
        let mut node = node;
        loop {
            match node {
                Node::Global(instance, index) => {
                    let global = arena.get(index);
                    match global {
                        Global::Variable(i) => {
                            let slot = instance.vars.0.get(*i).unwrap().take(AcqRel);
                            match slot {
                                None => {
                                    return Node::Global(instance, index);
                                }
                                Some(node2) => {
                                    node = *node2;
                                }
                            }
                        }
                        _ => {
                            return Node::Global(instance, index);
                        }
                    }
                }
                Node::Linear(Linear::Variable(mutex)) => {
                    let mut lock = mutex.lock().unwrap();
                    let slot = lock.take();
                    drop(lock);
                    match slot {
                        None => {
                            return Node::Linear(Linear::Variable(mutex));
                        }
                        Some(node2) => {
                            node = node2;
                        }
                    }
                }
                node => {
                    return node;
                }
            }
        }
    }

    // Package-related methods
    fn create_package_instance(&mut self, package: &Package<Linked>) -> Instance {
        let num_vars = package.num_vars;
        let mut vars = Vec::with_capacity(num_vars);
        for _ in 0..num_vars {
            vars.push(AtomicOptionBox::none());
        }
        Instance {
            vars: Arc::new(InstanceInner(vars.into_boxed_slice())),
        }
    }

    fn instatiate_linked_package_body(
        &mut self,
        instance: Instance,
        body: LinkedPackageBody,
    ) -> (Node<Linked>, Node<Linked>) {
        self.arena().get(body.redexes).iter().for_each(|(a, b)| {
            self.link(
                Node::Global(instance.clone(), *a),
                Node::Global(instance.clone(), *b),
            );
        });
        (
            Node::Global(instance.clone(), body.root),
            Node::Global(instance.clone(), body.captures),
        )
    }
    fn instantiate_package_captures_direct(
        &mut self,
        package: &Package<Linked>,
        captures: Node<Linked>,
    ) -> Node<Linked> {
        let instance = self.create_package_instance(package);
        self.instantiate_package_body_captures(instance, &package.body, captures)
    }
    fn instantiate_package_captures(
        &mut self,
        package: Index<Linked, OnceLock<Package<Linked>>>,
        captures: Node<Linked>,
    ) -> Node<Linked> {
        let arena = self.arena();
        let package = arena.get(package).get().unwrap();
        self.instantiate_package_captures_direct(package, captures)
    }
    fn instantiate_package_body_captures(
        &mut self,
        instance: Instance,
        package: &PackageBody<Linked>,
        captures: Node<Linked>,
    ) -> Node<Linked> {
        self.instantiate_linked_package_body_captures(instance, package.into(), captures)
    }
    fn instantiate_linked_package_body_captures(
        &mut self,
        instance: Instance,
        package: LinkedPackageBody,
        captures: Node<Linked>,
    ) -> Node<Linked> {
        let (root, captures_in) = self.instatiate_linked_package_body(instance, package);
        self.link(captures_in, captures);
        root
    }

    // Share-related methods
    fn enqueue_to_hole(&mut self, hole: &mut SharedHole<Linked>, cont: Node<Linked>) {
        match hole {
            SharedHole::Filled(sync_shared_value) => {
                self.link(
                    Node::Shared(Shared::Sync(Arc::new(sync_shared_value.clone()))),
                    cont,
                );
            }
            SharedHole::Unfilled(values) => values.push(cont),
        }
    }
    fn create_share_hole(&self) -> (Node<Linked>, Shared<Linked>) {
        let state = Arc::new(Mutex::new(SharedHole::Unfilled(vec![])));
        let hole = Node::Linear(Linear::ShareHole(state.clone()));
        (hole, Shared::Async(state))
    }
}

impl From<Arc<Arena<Linked>>> for Runtime {
    fn from(arena: Arc<Arena<Linked>>) -> Self {
        Self {
            arena,
            redexes: vec![],
            rewrites: Rewrites::default(),
        }
    }
}

macro_rules! sym {
    ($a: pat, $b: pat) => {
        ($a, $b) | ($b, $a)
    };
}

impl Linker for Runtime {
    fn link(&mut self, a: Node<Linked>, b: Node<Linked>) {
        self.redexes.push((a, b));
    }
    fn arena(&self) -> Arc<Arena<Linked>> {
        self.arena.clone()
    }
}

impl Runtime {
    // Misc methods.
    fn set_var(&mut self, instance: Instance, index: usize, value: Node<Linked>) {
        let slot = instance
            .vars
            .0
            .get(index)
            .expect("Invalid index in variable!");
        let other = slot.swap(Some(Box::new(value)), AcqRel);
        match other {
            Some(other) => {
                let value = slot.take(AcqRel).unwrap();
                self.link(*other, *value);
            }
            None => {}
        }
    }
    pub fn status(&self) {
        println!("Runtime status");
        for (a, b) in &self.redexes {
            println!("  {} ~ {}", self.show(&a), self.show(&b));
        }
    }
    /// Reduce all redexes in the net until a redex requires external action.
    /// Returns `Some` if there is such redex; returns `None` if no
    /// external action is needed and the net is in normal form.
    ///
    /// This function is analogous to a "VM enter"
    pub fn reduce(&mut self) -> Option<(UserData, Node<Linked>)> {
        while let Some((a, b)) = self.redexes.pop() {
            if let Some(v) = self.interact(a, b) {
                return Some(v);
            }
        }
        None
    }

    // Share-related methods

    /// Recusrively turn a node into a `Shared` node which allows duplication
    /// This is done whenever a node needs to be duplicated. This function may return None if the node can't be duplicated.
    /// This is the case for linear nodes and negative types.
    fn share(&mut self, node: Node<Linked>) -> Option<Shared<Linked>> {
        self.share_inner(node)
    }
    fn share_inner(&mut self, node: Node<Linked>) -> Option<Shared<Linked>> {
        match node {
            Node::Empty => unreachable!(),
            Node::Shared(shared) => Some(shared),
            Node::Global(instance, global_index) => match self.arena().get(global_index) {
                Global::Destruct(..) => None,
                Global::Fanout(..) => None,
                Global::Package(package, captures, FanBehavior::Expand) => {
                    let root = self.instantiate_package_captures(
                        package.clone(),
                        Node::Global(instance, captures.clone()),
                    );
                    self.share_inner(root)
                }
                Global::Package(package, captures, FanBehavior::Propagate) => {
                    self.rewrites.share_sync += 1;
                    let captures = self.share_inner(Node::Global(instance, *captures))?;
                    Some(Shared::Sync(Arc::new(SyncShared::Package(
                        *package, captures,
                    ))))
                }
                Global::Value(value) => {
                    self.rewrites.share_sync += 1;
                    Some(Shared::Sync(Arc::new(SyncShared::Value(
                        value.map_ref_leaves(|p| {
                            self.share_inner(Node::Global(instance.clone(), *p))
                        })?,
                    ))))
                }
                Global::Variable(id) => {
                    let slot = &instance.vars.0[*id];
                    match slot.take(AcqRel) {
                        Some(node) => {
                            self.rewrites.share_sync += 1;
                            Some(self.share_inner(*node)?)
                        }
                        _ => {
                            self.rewrites.share_async += 1;
                            let (hole, shared) = self.create_share_hole();
                            slot.swap(Some(Box::new(hole)), AcqRel);
                            Some(shared)
                        }
                    }
                }
            },
            Node::Linear(Linear::Value(value)) => {
                self.rewrites.share_sync += 1;
                Some(Shared::Sync(Arc::new(SyncShared::Value(
                    value.map_leaves(|p| self.share_inner(p))?,
                ))))
            }
            Node::Linear(Linear::Request(..)) => None,
            Node::Linear(Linear::Continue) => None,
            Node::Linear(Linear::Par(..)) => None,
            Node::Linear(Linear::Variable(mutex)) => {
                let mut lock = mutex.lock().unwrap();
                match lock.take() {
                    Some(slot) => {
                        drop(lock);
                        self.rewrites.share_sync += 1;
                        Some(self.share_inner(slot)?)
                    }
                    _ => {
                        self.rewrites.share_async += 1;
                        let (hole, shared) = self.create_share_hole();
                        lock.replace(hole);
                        Some(shared)
                    }
                }
            }
            Node::Linear(Linear::ShareHole(..)) => None,
        }
    }

    fn fill_hole(&mut self, hole: Arc<Mutex<SharedHole<Linked>>>, value: Node<Linked>) {
        let value = self.share(value).unwrap();
        match value {
            Shared::Async(value) => self.enqueue_to_hole(
                &mut value.lock().unwrap(),
                Node::Linear(Linear::ShareHole(hole)),
            ),
            Shared::Sync(value) => {
                let mut lock = hole.lock().unwrap();
                let SharedHole::Unfilled(continuations) =
                    core::mem::replace(&mut *lock, SharedHole::Filled((*value).clone()))
                else {
                    unreachable!()
                };
                for i in continuations {
                    self.link(i, Node::Shared(Shared::Sync(value.clone())));
                }
            }
        }
    }

    // Interact-related methods
    fn interact_fanout(
        &mut self,
        instance: Instance,
        destinations: Index<Linked, [Global<Linked>]>,
        other: Node<Linked>,
    ) {
        self.rewrites.fanout += 1;
        let other = self.share(other).unwrap();
        for dest in destinations {
            self.redexes.push((
                Node::Global(instance.clone(), dest),
                Node::Shared(other.clone()),
            ));
        }
    }
    fn interact_instantiate(
        &mut self,
        package: PackagePtr<Linked>,
        captures_in: Node<Linked>,
        other: Node<Linked>,
    ) {
        self.rewrites.instantiate += 1;
        let root = self.instantiate_package_captures(package, captures_in);
        self.link(root, other);
    }
    fn lookup_case_branch(
        &self,
        options: Index<Linked, [(Str<Linked>, PackageBody<Linked>)]>,
        variant: Str<Linked>,
    ) -> Option<LinkedPackageBody> {
        self.arena
            .get(options)
            .iter()
            .find(|(a, _)| a.clone() == variant)
            .map(|(_, b)| b.into())
    }
    /// Carry out an interaction between two nodes.
    /// Returns Some if an external operation with `UserData` was attempted.
    /// and None otherwise
    fn interact(&mut self, a: Node<Linked>, b: Node<Linked>) -> Option<(UserData, Node<Linked>)> {
        /// NodeRef is an internal structure to make matching on Nodes easier.
        /// It is like a Node but includes a reference to the Global in the Global branch
        /// to allow matching on it
        enum NodeRef<'a> {
            Linear(Linear<Linked>),
            Shared(Shared<Linked>),
            Global(Instance, Index<Linked, Global<Linked>>, &'a Global<Linked>),
        }
        impl<'a> NodeRef<'a> {
            fn from_node(arena: &'a Arena<Linked>, node: Node<Linked>) -> NodeRef<'a> {
                match node {
                    Node::Empty => unreachable!(),
                    Node::Linear(linear) => NodeRef::Linear(linear),
                    Node::Shared(shared) => NodeRef::Shared(shared),
                    Node::Global(instance, index) => {
                        NodeRef::Global(instance, index, arena.get(index))
                    }
                }
            }
            fn into_node(self) -> Node<Linked> {
                match self {
                    NodeRef::Linear(linear) => Node::Linear(linear),
                    NodeRef::Shared(shared) => Node::Shared(shared),
                    NodeRef::Global(instance, index, _) => Node::Global(instance, index),
                }
            }
        }
        let a_ref = NodeRef::from_node(&self.arena, a);
        let b_ref = NodeRef::from_node(&self.arena, b);
        // This is the match expression that is the core of the runtime
        // The priority order is very important and is probably
        // one of the most complex parts of the V3 runtime.
        //
        //
        // (1) Expanding packages with FanBehavior::Propagate (definition-like packages)
        //
        // Definitions are meant to be just aliases; so they have to behave exactly as if they
        // were inlined. This is why this is has highest priority
        //
        // (2) Linking variables
        //
        // This is the second step in reducing {[x] x}(1); the first one is
        // destructuring the pair the application compiles to with the par the function
        // compiles to. Linking variables is a fast operation that involves no duplication
        // that's why it is high priority. It is higher priority than (3) because otherwise
        // all duplication of variables would create ShareHoles
        //
        // (3) Fanout & Filling ShareHoles
        //
        // An interaction can't match both at the same time; that's why they're listed together
        // These two are the interactions that involve calling `share()` on the other side.
        // This is higher priority than (4) because share() on packages is what allows duplicating
        // boxes. It is higher priority than (5) to simplify external code.
        //
        // (4) Expanding packages with FanBehavior::Expand (box-like packages)
        //
        // This happens when the box interacts with Request, ExtFns, ExtArcs, and GlobalContinuations (and Values)
        // Since none of these are duplications, the only way to progress is to expand the box (dereliction)
        // and expose the inner value.
        //
        // (5) External calls: Requests, ExtFns and ExtArcs
        //
        // External calls could be higher priority if the external code knew how to handle
        // the nodes that are currently of a higher priority.
        //
        // When we implement readback duplication, this will have to get swapped with (4)
        //
        // (6) Destructuring values
        //
        // At this point, the only possible valid interaction left is Value ~ GlobalContinuation
        // This is the part of the runtime that corresponds to linear logic cut elimination
        // it does the "real" computation (unless you include duplication as computation);
        // everything else is just bookkeeping.
        //
        // This doesn't have to be this low priority, but knowing what variants we'll have simplifies
        // the matching code.
        match (a_ref, b_ref) {
            sym!(
                NodeRef::Global(
                    instance,
                    _,
                    Global::Package(package, captures_in, FanBehavior::Expand)
                ),
                other
            ) => {
                self.interact_instantiate(
                    *package,
                    Node::Global(instance, *captures_in),
                    other.into_node(),
                );
            }
            sym!(NodeRef::Global(instance, _, Global::Variable(index)), value) => {
                self.set_var(instance, *index, value.into_node())
            }
            sym!(NodeRef::Linear(Linear::Variable(mutex)), value) => {
                let mut lock = mutex.lock().unwrap();
                match lock.take() {
                    Some(node) => {
                        self.link(node, value.into_node());
                    }
                    None => {
                        lock.replace(value.into_node());
                    }
                }
            }
            sym!(NodeRef::Shared(Shared::Async(state)), other) => {
                let mut lock = state.lock().unwrap();
                self.enqueue_to_hole(&mut *lock, other.into_node());
            }
            sym!(
                NodeRef::Global(instance, _, Global::Fanout(destinations)),
                other
            ) => {
                self.interact_fanout(instance, *destinations, other.into_node());
            }
            sym!(NodeRef::Linear(Linear::ShareHole(hole)), other) => {
                self.fill_hole(hole, other.into_node())
            }
            sym!(NodeRef::Linear(Linear::Request(request)), other) => {
                let node = other.into_node();
                let value = self.destruct(node).expect("Request expects a value");
                request.send(value).unwrap();
                self.rewrites.ext_send += 1;
            }
            sym!(
                NodeRef::Global(instance, _, Global::Package(package, captures_in, _)),
                other
            ) => {
                self.interact_instantiate(
                    *package,
                    Node::Global(instance, *captures_in),
                    other.into_node(),
                );
            }
            sym!(NodeRef::Shared(Shared::Sync(x)), other)
                if matches!(&*x, SyncShared::Package(..)) =>
            {
                self.rewrites.instantiate += 1;
                let SyncShared::Package(package, captures_in) = &*x else {
                    unreachable!()
                };
                self.interact_instantiate(
                    *package,
                    Node::Shared(captures_in.clone()),
                    other.into_node(),
                );
            }

            sym!(NodeRef::Linear(Linear::Value(value)), other)
                if matches!(value.as_ref(), Value::ExternalFn(_)) =>
            {
                let Value::ExternalFn(ext) = *value else {
                    unreachable!()
                };
                self.rewrites.ext_call += 1;
                return Some((UserData::ExternalFn(ext), other.into_node()));
            }
            sym!(
                NodeRef::Global(_, _, Global::Value(Value::ExternalFn(ext))),
                other
            ) => {
                self.rewrites.ext_call += 1;
                return Some((UserData::ExternalFn(*ext), other.into_node()));
            }
            sym!(NodeRef::Shared(Shared::Sync(shared)), other)
                if matches!(shared.as_ref(), SyncShared::Value(Value::ExternalFn(_))) =>
            {
                let SyncShared::Value(Value::ExternalFn(ext)) = shared.as_ref() else {
                    unreachable!()
                };
                self.rewrites.ext_call += 1;
                return Some((UserData::ExternalFn(*ext), other.into_node()));
            }
            sym!(NodeRef::Linear(Linear::Value(value)), other)
                if matches!(value.as_ref(), Value::ExternalArc(_)) =>
            {
                let Value::ExternalArc(ext) = *value else {
                    unreachable!()
                };
                self.rewrites.ext_call += 1;
                return Some((UserData::ExternalArc(ext), other.into_node()));
            }
            sym!(
                NodeRef::Global(_, _, Global::Value(Value::ExternalArc(ext))),
                other
            ) => {
                self.rewrites.ext_call += 1;
                return Some((UserData::ExternalArc(ext.clone()), other.into_node()));
            }
            sym!(NodeRef::Shared(Shared::Sync(shared)), other)
                if matches!(shared.as_ref(), SyncShared::Value(Value::ExternalArc(_))) =>
            {
                let SyncShared::Value(Value::ExternalArc(ext)) = shared.as_ref() else {
                    unreachable!()
                };
                self.rewrites.ext_call += 1;
                return Some((UserData::ExternalArc(ext.clone()), other.into_node()));
            }
            sym!(
                NodeRef::Global(instance, _, Global::Destruct(destructor)),
                node
            ) => {
                if let GlobalCont::Continue = destructor {
                    self.rewrites.r#continue += 1;
                    return None;
                }
                let destructor = destructor.clone();
                // let node = node.into_node();
                let value = match node {
                    NodeRef::Linear(Linear::Value(v)) => Ok(*v),
                    NodeRef::Shared(Shared::Sync(shared)) => match &*shared {
                        SyncShared::Package(package, shared) => {
                            let node = self.instantiate_package_captures(
                                *package,
                                Node::Shared(shared.clone()),
                            );
                            let Node::Global(instance, global_index) = node else {
                                unreachable!("should be global")
                            };
                            match self.arena().get(global_index) {
                                Global::Value(v) => Ok(v
                                    .map_ref_leaves(|x| Some(Node::Global(instance.clone(), *x)))
                                    .unwrap()),
                                _ => Err(Node::Global(instance, global_index)),
                            }
                        }
                        SyncShared::Value(shared) => Ok(shared
                            .clone()
                            .map_leaves(|x| Some(Node::Shared(x)))
                            .unwrap()),
                    },
                    NodeRef::Global(instance, _, Global::Value(v)) => Ok(v
                        .map_ref_leaves(|x| Some(Node::Global(instance.clone(), *x)))
                        .unwrap()),
                    node => Err(node.into_node()),
                };
                let value = value.expect("Continue expects a value");
                match (value, destructor) {
                    (Value::Pair(a0, a1), GlobalCont::Par(b0, b1)) => {
                        self.rewrites.receive += 1;
                        self.link(a0, Node::Global(instance.clone(), b0));
                        self.link(a1, Node::Global(instance, b1));
                    }
                    (Value::Either(signal, payload), GlobalCont::Choice(context, options)) => {
                        self.rewrites.r#match += 1;
                        if let Some(package) =
                            self.lookup_case_branch(options.clone(), signal.clone())
                        {
                            let root = self.instantiate_linked_package_body_captures(
                                instance.clone(),
                                package,
                                Node::Global(instance, context),
                            );
                            self.link(payload, root);
                        } else {
                            let branch =
                                self.lookup_case_branch(options, self.arena.empty_string());
                            let root = self.instantiate_linked_package_body_captures(
                                instance.clone(),
                                branch.unwrap(),
                                Node::Global(instance, context),
                            );
                            // TODO: Optimize this; we're reconstructing the `Either` branch.
                            // This could make us lose sharing.
                            self.link(
                                Node::Linear(Linear::Value(Box::new(Value::Either(
                                    signal, payload,
                                )))),
                                root,
                            );
                        }
                    }
                    (a, b) => {
                        panic!("Unimplemented destruction between: {:?} {:?}", a, b)
                    }
                }
            }
            sym!(NodeRef::Linear(Linear::Continue), _other) => {
                // we can just drop it
                self.rewrites.r#continue += 1;
            }
            sym!(NodeRef::Linear(Linear::Par(a1, b1)), other) => {
                let (a2, b2) = match other {
                    NodeRef::Linear(Linear::Value(v)) => {
                        let Value::Pair(a, b) = *v else {
                            unreachable!("Expected Pair")
                        };
                        (a, b)
                    }
                    NodeRef::Shared(Shared::Sync(shared)) => match &*shared {
                        SyncShared::Package(package, shared) => {
                            let node = self.instantiate_package_captures(
                                *package,
                                Node::Shared(shared.clone()),
                            );
                            let value = self.destruct(node).expect("Expected value");
                            let Value::Pair(a, b) = value else {
                                unreachable!("Expected pair")
                            };
                            (a, b)
                        }
                        SyncShared::Value(shared) => {
                            let Value::Pair(a, b) = shared else {
                                unreachable!("Expected pair")
                            };
                            (Node::Shared(a.clone()), Node::Shared(b.clone()))
                        }
                    },
                    NodeRef::Global(instance, _, Global::Value(v)) => {
                        let Value::Pair(a, b) = v else {
                            unreachable!("Expected pair")
                        };
                        (
                            Node::Global(instance.clone(), *a),
                            Node::Global(instance, *b),
                        )
                    }
                    _node => unreachable!("Expected pair"),
                };
                // let value = value.expect("Continue expects a value");

                // let value = self
                //     .destruct(other.into_node())
                //     .expect("Continue expects a value");
                // let Value::Pair(a2, b2) = value else {
                //     panic!("Unimplemented destruction between Par and {:?}", value);
                // };
                self.rewrites.r#receive += 1;
                self.link(*a1, a2);
                self.link(*b1, b2);
            }
            (a, b) => {
                panic!(
                    "Unimplemented reduction: {:?} {:?}",
                    a.into_node().variant_name(),
                    b.into_node().variant_name()
                )
            }
        };
        None
    }
}

impl Node<Linked> {
    pub fn variant_name(&self) -> String {
        match self {
            Node::Empty => unreachable!(),
            Node::Linear(l) => format!("Linear.{}", l.variant_name()),
            Node::Shared(s) => format!("Shared.{}", s.variant_name()),
            /*Node::Global(i, g) => {
                format!("Global@{:x}.{}", i.identifier(), g.variant_name())
            }*/
            Node::Global(i, _g) => {
                format!("Global@{:x}.{}", i.identifier(), "REF")
            }
        }
    }
}

impl Linear<Linked> {
    pub fn variant_name(&self) -> String {
        match self {
            Linear::Value(v) => format!("Value({})", v.variant_name()),
            Linear::Continue => "Continue".to_owned(),
            Linear::Par(a, b) => format!("Par({}, {})", a.variant_name(), b.variant_name()),
            Linear::Request(_) => "Request".to_owned(),
            Linear::ShareHole(_) => "ShareHole".to_owned(),
            Linear::Variable(_) => "Variable".to_owned(),
        }
    }
}

impl Shared<Linked> {
    pub fn variant_name(&self) -> String {
        match self {
            Shared::Async(_) => "Async".to_owned(),
            Shared::Sync(sync) => format!("Sync({})", sync.variant_name()),
        }
    }
}

impl SyncShared<Linked> {
    pub fn variant_name(&self) -> String {
        match self {
            SyncShared::Package(_, inner) => format!("Package({})", inner.variant_name()),
            SyncShared::Value(v) => format!("Value({})", v.variant_name()),
        }
    }
}

impl Global<Linked> {
    pub fn variant_name(&self) -> String {
        match self {
            Global::Variable(_) => "Variable".into(),

            Global::Package(_, _, _) => "Package".into(),

            Global::Destruct(c) => format!("Destruct({})", c.variant_name()),

            Global::Value(v) => format!("Value({})", v.variant_name()),

            Global::Fanout(_idx) => "Fanout".into(),
        }
    }
}

impl<Ext: Clone> GlobalCont<Ext> {
    pub fn variant_name(&self) -> String {
        match self {
            GlobalCont::Continue => "Continue".into(),

            GlobalCont::Par(_a, _b) => "Par".into(),

            GlobalCont::Choice(_ptr, ..) => "Choice".into(),
        }
    }
}

impl<P> Value<P, Linked> {
    pub fn variant_name(&self) -> String {
        match self {
            Value::Break => "Break".into(),

            Value::Pair(_a, _b) => "Pair".into(),

            Value::Either(_, _p) => "Either".into(),

            Value::ExternalFn(_) => "ExternalFn".into(),
            Value::ExternalArc(_) => "ExternalArc".into(),

            Value::Primitive(_) => "Primitive".into(),
        }
    }
}
