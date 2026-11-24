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
use super::lock::SpinLock;
use crate::fan_behavior::FanBehavior;
use crate::flat::stats::Rewrites;
use crate::linker::Linked;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::oneshot::Sender;

pub type PackagePtr<Ext> = Index<Ext, OnceLock<Package<Ext>>>;
pub(crate) type GlobalPtr<Ext> = Index<Ext, Global<Ext>>;
type Redex = (Node<Linked>, Node<Linked>);
type Str<Ext> = Index<Ext, str>;

#[derive(Debug)]
struct InstanceInner(Box<[Slot<Linked>]>);

/// A place for a single node, which can be filled and emptied concurrently.
///
/// Slots are the variables of [`Instance`]s, which are filled by the reducer,
/// but external handles may concurrently take nodes out of them (see
/// [`Linker::deref`]). They also serve as variables for external handles.
pub struct Slot<Ext: Clone>(SpinLock<Option<Node<Ext>>>);

impl<Ext: Clone> Slot<Ext> {
    pub(crate) fn new() -> Self {
        Self(SpinLock::new(None))
    }

    #[inline(always)]
    pub(crate) fn take(&self) -> Option<Node<Ext>> {
        self.0.lock().take()
    }

    /// Puts `node` into the slot if it's empty. If it's already full, empties
    /// it instead, and returns the previous node together with `node`.
    #[inline(always)]
    pub(crate) fn put(&self, node: Node<Ext>) -> Option<(Node<Ext>, Node<Ext>)> {
        let mut slot = self.0.lock();
        match slot.take() {
            Some(other) => Some((other, node)),
            None => {
                *slot = Some(node);
                None
            }
        }
    }
}

impl<Ext: Clone + Debug> Debug for Slot<Ext> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

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
/// They are created empty by `Instance::new`, with a fixed length. Initially, all variables are empty.
/// Then, calls to `Runtime::set_var` slowly fill up and empty the instance. Each variable slot is filled once and taken out of once.
/// (I'm not sure if it's possible for a variable slot to never be used, but it definitely can't be linked to twice)
/// At the end of the lifetime of Instances, all of them go out of scope and are eventually dropped. Because of the way
/// the runtime is designed, all slots inside of it must be empty. This is when the `Instance` is destroyed.
pub struct Instance {
    vars: Arc<InstanceInner>,
}

impl Instance {
    pub(crate) fn new(num_vars: usize) -> Self {
        Instance {
            vars: Arc::new(InstanceInner((0..num_vars).map(|_| Slot::new()).collect())),
        }
    }

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
/// A PackageBody is the inner body of a package, or of a case branch.
/// The difference with `Package` is that `PackageBody` does not contain `num_vars`
/// and captures (because a case branch is instantiated in the instance of its
/// choice, with its context wired in directly by the compiler).
pub struct PackageBody<Ext: Clone> {
    pub root: Index<Ext, Global<Ext>>,

    pub debug_name: String,
    // TODO: Store this inline in the arena.
    pub redexes: Index<Ext, [(Index<Ext, Global<Ext>>, Index<Ext, Global<Ext>>)]>,
}

/// Where the captures of an instantiated package come from.
pub(crate) enum Captures<'a> {
    /// The captures of a [`Global::Package`] node, in the node's instance.
    Global(Instance, Index<Linked, [Global<Linked>]>),
    /// The captures of a shared package.
    Shared(&'a [Shared<Linked>]),
}

#[derive(Clone, Copy)]
pub(crate) struct LinkedPackageBody {
    root: GlobalPtr<Linked>,
    redexes: Index<Linked, [(GlobalPtr<Linked>, GlobalPtr<Linked>)]>,
}

impl From<&PackageBody<Linked>> for LinkedPackageBody {
    fn from(body: &PackageBody<Linked>) -> Self {
        Self {
            root: body.root,
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
    /// The variables of the first slots of the package's instance, which
    /// receive its captures when it's instantiated.
    pub captures: Index<Ext, [Global<Ext>]>,
    /// How large the Instance must be.
    pub num_vars: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Global<Ext: Clone> {
    Variable(usize),
    /// Compiler-inserted structural disposal for a value satisfying the `drop` constraint.
    Close {
        signal: Str<Ext>,
        erase: GlobalPtr<Ext>,
    },
    /// A package with its captures, which get moved into the first variable
    /// slots of its instance when it's instantiated.
    Package(PackagePtr<Ext>, Index<Ext, [Global<Ext>]>, FanBehavior),
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
    Async(Arc<SpinLock<SharedHole<Ext>>>),
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
    Package(PackagePtr<Ext>, Box<[Shared<Ext>]>),
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
    ShareHole(Arc<SpinLock<SharedHole<Linked>>>),
    // This variant is similiar to Global::Variable in function
    // but is used by external tasks.
    Variable(Arc<Slot<Ext>>),
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
    /// The branches are instantiated in the instance of the choice.
    Choice(Index<Ext, [(Str<Ext>, PackageBody<Ext>)]>),
    /// Instantiate exactly one reusable box and continue with the resulting value.
    Unbox(GlobalPtr<Ext>),
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
    fn arena(&self) -> &Arc<Arena<Linked>>;

    fn show<'a, 'b>(&'b self, node: &'a Node<Linked>) -> String {
        format!("{}", Showable(node, &mut Shower::from_arena(self.arena())))
    }
    fn destruct(
        &mut self,
        node: Node<Linked>,
    ) -> Result<Value<Node<Linked>, Linked>, Node<Linked>> {
        match node {
            Node::Linear(Linear::Value(v)) => Ok(*v),
            Node::Shared(Shared::Sync(shared)) => match &*shared {
                SyncShared::Package(package, captures) => {
                    let Node::Global(instance, global_index) =
                        self.instantiate_package(*package, Captures::Shared(captures))
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
                            let slot = instance.vars.0.get(*i).unwrap().take();
                            match slot {
                                None => {
                                    return Node::Global(instance, index);
                                }
                                Some(node2) => {
                                    node = node2;
                                }
                            }
                        }
                        _ => {
                            return Node::Global(instance, index);
                        }
                    }
                }
                Node::Linear(Linear::Variable(slot)) => match slot.take() {
                    None => {
                        return Node::Linear(Linear::Variable(slot));
                    }
                    Some(node2) => {
                        node = node2;
                    }
                },
                node => {
                    return node;
                }
            }
        }
    }

    // Package-related methods
    /// Links the body's redexes in the instance, and returns its root.
    fn instantiate_linked_package_body(
        &mut self,
        instance: Instance,
        body: LinkedPackageBody,
    ) -> Node<Linked> {
        for redex in body.redexes {
            let (a, b) = *self.arena().get(redex);
            self.link(
                Node::Global(instance.clone(), a),
                Node::Global(instance.clone(), b),
            );
        }
        Node::Global(instance, body.root)
    }
    /// Instantiates a package, moving its captures straight into the first
    /// slots of its new instance, and returns its root.
    fn instantiate_package(
        &mut self,
        package: Index<Linked, OnceLock<Package<Linked>>>,
        captures: Captures<'_>,
    ) -> Node<Linked> {
        let package = self.arena().get(package).get().unwrap();
        let num_vars = package.num_vars;
        let body = LinkedPackageBody::from(&package.body);
        let slots = package.captures;
        let instance = Instance::new(num_vars);
        match captures {
            Captures::Global(outer, items) => {
                debug_assert_eq!(items.0.1, slots.0.1, "mismatched package captures");
                for (i, (item, slot)) in items.zip(slots).enumerate() {
                    let inner_slot = &instance.vars.0[i];
                    let variable = match self.arena().get(item) {
                        Global::Variable(index) => Some(*index),
                        _ => None,
                    };
                    match variable {
                        Some(index) => {
                            // Move the captured variable's node over if it's there already,
                            // otherwise make it go to the new slot once it arrives.
                            let outer_slot = &outer.vars.0[index];
                            match outer_slot.take() {
                                Some(node) => {
                                    inner_slot.put(node);
                                }
                                None => {
                                    let forward = Node::Global(instance.clone(), slot);
                                    if let Some((a, b)) = outer_slot.put(forward) {
                                        self.link(a, b);
                                    }
                                }
                            }
                        }
                        None => {
                            inner_slot.put(Node::Global(outer.clone(), item));
                        }
                    }
                }
            }
            Captures::Shared(shared) => {
                debug_assert_eq!(shared.len(), slots.0.1, "mismatched package captures");
                for (i, shared) in shared.iter().enumerate() {
                    instance.vars.0[i].put(Node::Shared(shared.clone()));
                }
            }
        }
        self.instantiate_linked_package_body(instance, body)
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
        let state = Arc::new(SpinLock::new(SharedHole::Unfilled(vec![])));
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
    #[inline(always)]
    fn link(&mut self, a: Node<Linked>, b: Node<Linked>) {
        self.redexes.push((a, b));
    }
    fn arena(&self) -> &Arc<Arena<Linked>> {
        &self.arena
    }
}

impl Runtime {
    // Misc methods.
    #[inline(always)]
    fn set_var(&mut self, instance: Instance, index: usize, value: Node<Linked>) {
        if let Some((a, b)) = self.set_var_next(instance, index, value) {
            self.link(a, b);
        }
    }
    /// Like [`Runtime::set_var`], but returns the resulting redex, if any,
    /// instead of pushing it.
    #[inline(always)]
    fn set_var_next(
        &mut self,
        instance: Instance,
        index: usize,
        value: Node<Linked>,
    ) -> Option<Redex> {
        let slot = instance
            .vars
            .0
            .get(index)
            .expect("Invalid index in variable!");
        slot.put(value)
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
        let mut next = self.redexes.pop();
        while let Some((a, b)) = next {
            next = match self.interact_fast(a, b) {
                // Process the last redex produced by the interaction right away,
                // as if it was pushed and popped.
                Ok(Some(redex)) => Some(redex),
                Ok(None) => self.redexes.pop(),
                Err((a, b)) => {
                    if let Some(v) = self.interact(a, b) {
                        return Some(v);
                    }
                    self.redexes.pop()
                }
            };
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
            Node::Global(instance, global_index) => match self.arena().clone().get(global_index) {
                Global::Destruct(..) => None,
                Global::Fanout(..) => None,
                Global::Close { .. } => None,
                Global::Package(package, captures, FanBehavior::Expand) => {
                    let root =
                        self.instantiate_package(*package, Captures::Global(instance, *captures));
                    self.share_inner(root)
                }
                Global::Package(package, captures, FanBehavior::Propagate) => {
                    self.rewrites.share_sync += 1;
                    let captures = captures
                        .map(|capture| self.share_inner(Node::Global(instance.clone(), capture)))
                        .collect::<Option<Box<[_]>>>()?;
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
                    match slot.take() {
                        Some(node) => {
                            self.rewrites.share_sync += 1;
                            Some(self.share_inner(node)?)
                        }
                        _ => {
                            self.rewrites.share_async += 1;
                            let (hole, shared) = self.create_share_hole();
                            if let Some((other, hole)) = slot.put(hole) {
                                self.link(other, hole);
                            }
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
            Node::Linear(Linear::Variable(slot)) => match slot.take() {
                Some(node) => {
                    self.rewrites.share_sync += 1;
                    Some(self.share_inner(node)?)
                }
                None => {
                    self.rewrites.share_async += 1;
                    let (hole, shared) = self.create_share_hole();
                    if let Some((other, hole)) = slot.put(hole) {
                        self.link(other, hole);
                    }
                    Some(shared)
                }
            },
            Node::Linear(Linear::ShareHole(..)) => None,
        }
    }

    fn fill_hole(&mut self, hole: Arc<SpinLock<SharedHole<Linked>>>, value: Node<Linked>) {
        let value = self.share(value).unwrap();
        match value {
            Shared::Async(value) => {
                self.enqueue_to_hole(&mut value.lock(), Node::Linear(Linear::ShareHole(hole)))
            }
            Shared::Sync(value) => {
                let mut lock = hole.lock();
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
    /// Returns the redex of the last destination instead of pushing it.
    fn interact_fanout(
        &mut self,
        instance: Instance,
        destinations: Index<Linked, [Global<Linked>]>,
        other: Node<Linked>,
    ) -> Option<Redex> {
        self.rewrites.fanout += 1;
        let other = self.share(other).unwrap();
        let Index((start, len)) = destinations;
        if len == 0 {
            return None;
        }
        let init: Index<Linked, [Global<Linked>]> = Index((start, len - 1));
        for dest in init {
            self.redexes.push((
                Node::Global(instance.clone(), dest),
                Node::Shared(other.clone()),
            ));
        }
        Some((
            Node::Global(instance, Index(start + len - 1)),
            Node::Shared(other),
        ))
    }
    fn interact_instantiate(
        &mut self,
        package: PackagePtr<Linked>,
        captures: Captures<'_>,
        other: Node<Linked>,
    ) {
        self.rewrites.instantiate += 1;
        let root = self.instantiate_package(package, captures);
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
    /// Handles the most frequent interactions without going through the general
    /// match in [`Runtime::interact`]. Each case here must agree with the
    /// priority order documented there; anything else is handed back.
    #[inline(always)]
    ///
    /// On success, returns the last redex produced by the interaction, which
    /// should be processed next, instead of pushing it.
    fn interact_fast(&mut self, a: Node<Linked>, b: Node<Linked>) -> Result<Option<Redex>, Redex> {
        let global = |node: &Node<Linked>| match node {
            Node::Global(_, index) => Some(self.arena.get(*index)),
            _ => None,
        };
        let is_expand = |g: Option<&Global<Linked>>| {
            matches!(g, Some(Global::Package(_, _, FanBehavior::Expand)))
        };
        // Whether a fanout interacting with this node gets to act as (3).
        let fanout_applies = |g: Option<&Global<Linked>>, node: &Node<Linked>| match node {
            Node::Global(..) => !matches!(
                g,
                Some(
                    Global::Package(_, _, FanBehavior::Expand)
                        | Global::Variable(_)
                        | Global::Close { .. }
                )
            ),
            Node::Linear(Linear::Variable(_)) | Node::Shared(Shared::Async(_)) | Node::Empty => {
                false
            }
            Node::Linear(_) | Node::Shared(Shared::Sync(_)) => true,
        };
        let is_break = |g: Option<&Global<Linked>>, node: &Node<Linked>| match node {
            Node::Global(..) => matches!(g, Some(Global::Value(Value::Break))),
            Node::Shared(Shared::Sync(shared)) => {
                matches!(**shared, SyncShared::Value(Value::Break))
            }
            _ => false,
        };
        let is_linear_variable =
            |node: &Node<Linked>| matches!(node, Node::Linear(Linear::Variable(_)));
        // Whether a global node gets to instantiate a shared package as (4),
        // instead of something with a higher priority happening.
        let instantiates_shared = |g: &Global<Linked>, node: &Node<Linked>| {
            matches!(
                g,
                Global::Value(_)
                    | Global::Destruct(
                        GlobalCont::Par(..) | GlobalCont::Choice(..) | GlobalCont::Continue
                    )
            ) && matches!(
                node,
                Node::Shared(Shared::Sync(shared)) if matches!(**shared, SyncShared::Package(..))
            )
        };
        let is_linear_either = |node: &Node<Linked>| match node {
            Node::Linear(Linear::Value(value)) => matches!(**value, Value::Either(..)),
            _ => false,
        };
        let (ga, gb) = (global(&a), global(&b));
        match (ga, gb) {
            // (1) Expanding box-like packages, such as loop bodies.
            (Some(Global::Package(package, captures, FanBehavior::Expand)), _) => {
                let (package, captures) = (*package, *captures);
                let Node::Global(instance, _) = a else {
                    unreachable!()
                };
                self.rewrites.instantiate += 1;
                let root = self.instantiate_package(package, Captures::Global(instance, captures));
                return Ok(Some((root, b)));
            }
            (_, Some(Global::Package(package, captures, FanBehavior::Expand))) => {
                let (package, captures) = (*package, *captures);
                let Node::Global(instance, _) = b else {
                    unreachable!()
                };
                self.rewrites.instantiate += 1;
                let root = self.instantiate_package(package, Captures::Global(instance, captures));
                return Ok(Some((root, a)));
            }
            // (2) Linking variables. Only box-like packages take precedence.
            (Some(Global::Variable(index)), gb) if !is_expand(gb) => {
                let index = *index;
                let Node::Global(instance, _) = a else {
                    unreachable!()
                };
                return Ok(self.set_var_next(instance, index, b));
            }
            (ga, Some(Global::Variable(index))) if !is_expand(ga) => {
                let index = *index;
                let Node::Global(instance, _) = b else {
                    unreachable!()
                };
                return Ok(self.set_var_next(instance, index, a));
            }
            (None, gb) if is_linear_variable(&a) && !is_expand(gb) => {
                let Node::Linear(Linear::Variable(slot)) = a else {
                    unreachable!()
                };
                return Ok(slot.put(b));
            }
            (ga, None) if is_linear_variable(&b) && !is_expand(ga) => {
                let Node::Linear(Linear::Variable(slot)) = b else {
                    unreachable!()
                };
                return Ok(slot.put(a));
            }
            // (3) Fanout.
            (Some(Global::Fanout(destinations)), gb) if fanout_applies(gb, &b) => {
                let destinations = *destinations;
                let Node::Global(instance, _) = a else {
                    unreachable!()
                };
                return Ok(self.interact_fanout(instance, destinations, b));
            }
            (ga, Some(Global::Fanout(destinations))) if fanout_applies(ga, &a) => {
                let destinations = *destinations;
                let Node::Global(instance, _) = b else {
                    unreachable!()
                };
                return Ok(self.interact_fanout(instance, destinations, a));
            }
            // (6) Destructuring global values.
            (
                Some(Global::Value(Value::Pair(a0, a1))),
                Some(Global::Destruct(GlobalCont::Par(b0, b1))),
            ) => {
                let (a0, a1, b0, b1) = (*a0, *a1, *b0, *b1);
                let (Node::Global(value_instance, _), Node::Global(par_instance, _)) = (a, b)
                else {
                    unreachable!()
                };
                return Ok(Some(self.interact_pair_par(
                    value_instance,
                    a0,
                    a1,
                    par_instance,
                    b0,
                    b1,
                )));
            }
            (
                Some(Global::Destruct(GlobalCont::Par(b0, b1))),
                Some(Global::Value(Value::Pair(a0, a1))),
            ) => {
                let (a0, a1, b0, b1) = (*a0, *a1, *b0, *b1);
                let (Node::Global(par_instance, _), Node::Global(value_instance, _)) = (a, b)
                else {
                    unreachable!()
                };
                return Ok(Some(self.interact_pair_par(
                    value_instance,
                    a0,
                    a1,
                    par_instance,
                    b0,
                    b1,
                )));
            }
            (
                Some(Global::Value(Value::Either(signal, payload))),
                Some(Global::Destruct(GlobalCont::Choice(options))),
            ) => {
                let (signal, payload, options) = (*signal, *payload, *options);
                let (Node::Global(value_instance, _), Node::Global(choice_instance, _)) = (a, b)
                else {
                    unreachable!()
                };
                return Ok(Some(self.interact_either_choice(
                    signal,
                    Node::Global(value_instance, payload),
                    choice_instance,
                    options,
                )));
            }
            (
                Some(Global::Destruct(GlobalCont::Choice(options))),
                Some(Global::Value(Value::Either(signal, payload))),
            ) => {
                let (signal, payload, options) = (*signal, *payload, *options);
                let (Node::Global(choice_instance, _), Node::Global(value_instance, _)) = (a, b)
                else {
                    unreachable!()
                };
                return Ok(Some(self.interact_either_choice(
                    signal,
                    Node::Global(value_instance, payload),
                    choice_instance,
                    options,
                )));
            }
            (Some(Global::Destruct(GlobalCont::Choice(options))), None) if is_linear_either(&b) => {
                let options = *options;
                let (Node::Global(choice_instance, _), Node::Linear(Linear::Value(value))) = (a, b)
                else {
                    unreachable!()
                };
                let Value::Either(signal, payload) = *value else {
                    unreachable!()
                };
                return Ok(Some(self.interact_either_choice(
                    signal,
                    payload,
                    choice_instance,
                    options,
                )));
            }
            (None, Some(Global::Destruct(GlobalCont::Choice(options)))) if is_linear_either(&a) => {
                let options = *options;
                let (Node::Linear(Linear::Value(value)), Node::Global(choice_instance, _)) = (a, b)
                else {
                    unreachable!()
                };
                let Value::Either(signal, payload) = *value else {
                    unreachable!()
                };
                return Ok(Some(self.interact_either_choice(
                    signal,
                    payload,
                    choice_instance,
                    options,
                )));
            }
            // (4) Instantiating shared packages.
            (None, Some(gb)) if instantiates_shared(gb, &a) => {
                let Node::Shared(Shared::Sync(shared)) = a else {
                    unreachable!()
                };
                let SyncShared::Package(package, captures) = &*shared else {
                    unreachable!()
                };
                self.rewrites.instantiate += 1;
                let root = self.instantiate_package(*package, Captures::Shared(captures));
                return Ok(Some((root, b)));
            }
            (Some(ga), None) if instantiates_shared(ga, &b) => {
                let Node::Shared(Shared::Sync(shared)) = b else {
                    unreachable!()
                };
                let SyncShared::Package(package, captures) = &*shared else {
                    unreachable!()
                };
                self.rewrites.instantiate += 1;
                let root = self.instantiate_package(*package, Captures::Shared(captures));
                return Ok(Some((root, a)));
            }
            (Some(Global::Destruct(GlobalCont::Continue)), gb) if is_break(gb, &b) => {
                self.rewrites.r#continue += 1;
            }
            (ga, Some(Global::Destruct(GlobalCont::Continue))) if is_break(ga, &a) => {
                self.rewrites.r#continue += 1;
            }
            _ => return Err((a, b)),
        }
        Ok(None)
    }

    /// Returns the redex of the payload with the branch instead of pushing it.
    fn interact_either_choice(
        &mut self,
        signal: Str<Linked>,
        payload: Node<Linked>,
        instance: Instance,
        options: Index<Linked, [(Str<Linked>, PackageBody<Linked>)]>,
    ) -> Redex {
        self.rewrites.r#match += 1;
        if let Some(package) = self.lookup_case_branch(options, signal) {
            let root = self.instantiate_linked_package_body(instance, package);
            (payload, root)
        } else {
            let branch = self.lookup_case_branch(options, self.arena.empty_string());
            let root = self.instantiate_linked_package_body(instance, branch.unwrap());
            // TODO: Optimize this; we're reconstructing the `Either` branch.
            // This could make us lose sharing.
            (
                Node::Linear(Linear::Value(Box::new(Value::Either(signal, payload)))),
                root,
            )
        }
    }

    /// Returns the redex of the second halves instead of pushing it.
    #[inline(always)]
    fn interact_pair_par(
        &mut self,
        value_instance: Instance,
        a0: GlobalPtr<Linked>,
        a1: GlobalPtr<Linked>,
        par_instance: Instance,
        b0: GlobalPtr<Linked>,
        b1: GlobalPtr<Linked>,
    ) -> Redex {
        self.rewrites.receive += 1;
        self.link(
            Node::Global(value_instance.clone(), a0),
            Node::Global(par_instance.clone(), b0),
        );
        (
            Node::Global(value_instance, a1),
            Node::Global(par_instance, b1),
        )
    }

    /// Carry out an interaction between two nodes.
    /// Returns Some if an external operation with `UserData` was attempted.
    /// and None otherwise
    ///
    /// The most frequent interactions are handled by [`Runtime::interact_fast`]
    /// before getting here, keeping this large function out of the hot loop.
    #[inline(never)]
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
        // (1) Expanding packages with FanBehavior::Expand (definition-like packages)
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
        // (4) Expanding packages with FanBehavior::Propagate (box-like packages)
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
                    Global::Package(package, captures, FanBehavior::Expand)
                ),
                other
            ) => {
                self.interact_instantiate(
                    *package,
                    Captures::Global(instance, *captures),
                    other.into_node(),
                );
            }
            sym!(NodeRef::Global(instance, _, Global::Variable(index)), value) => {
                self.set_var(instance, *index, value.into_node())
            }
            sym!(NodeRef::Linear(Linear::Variable(slot)), value) => {
                if let Some((a, b)) = slot.put(value.into_node()) {
                    self.link(a, b);
                }
            }
            sym!(
                NodeRef::Global(close_instance, close_index, Global::Close { .. }),
                NodeRef::Global(
                    value_instance,
                    _,
                    Global::Package(_, captures, FanBehavior::Propagate)
                )
            ) => {
                self.rewrites.fanout += 1;
                let captures = *captures;
                for capture in captures {
                    self.link(
                        Node::Global(value_instance.clone(), capture),
                        Node::Global(close_instance.clone(), close_index),
                    );
                }
            }
            sym!(
                NodeRef::Global(_, _, Global::Close { .. }),
                NodeRef::Shared(_)
            ) => {
                self.rewrites.fanout += 1;
            }
            sym!(NodeRef::Shared(Shared::Async(state)), other) => {
                let mut lock = state.lock();
                self.enqueue_to_hole(&mut *lock, other.into_node());
            }
            sym!(
                NodeRef::Global(close_instance, _, Global::Close { erase, .. }),
                NodeRef::Global(fanout_instance, _, Global::Fanout(destinations))
            ) => {
                self.rewrites.fanout += 1;
                let erase = *erase;
                for destination in *destinations {
                    self.link(
                        Node::Global(fanout_instance.clone(), destination),
                        Node::Global(close_instance.clone(), erase),
                    );
                }
            }
            sym!(
                NodeRef::Global(instance, _, Global::Fanout(destinations)),
                other
            ) => {
                if let Some((a, b)) =
                    self.interact_fanout(instance, *destinations, other.into_node())
                {
                    self.link(a, b);
                }
            }
            sym!(NodeRef::Linear(Linear::ShareHole(hole)), other) => {
                self.fill_hole(hole, other.into_node())
            }
            sym!(
                NodeRef::Global(close_instance, close_index, Global::Close { signal, .. }),
                NodeRef::Linear(Linear::Request(request))
            ) => {
                request
                    .send(Value::Either(
                        *signal,
                        Node::Global(close_instance, close_index),
                    ))
                    .unwrap();
                self.rewrites.ext_send += 1;
            }
            sym!(NodeRef::Linear(Linear::Request(request)), other) => {
                let node = other.into_node();
                let value = self.destruct(node).expect("Request expects a value");
                request.send(value).unwrap();
                self.rewrites.ext_send += 1;
            }
            sym!(
                NodeRef::Global(
                    package_instance,
                    _,
                    Global::Package(package, captures, FanBehavior::Propagate)
                ),
                NodeRef::Global(
                    unbox_instance,
                    _,
                    Global::Destruct(GlobalCont::Unbox(continuation))
                )
            ) => {
                let package = *package;
                let captures = *captures;
                let continuation = *continuation;
                self.rewrites.instantiate += 1;
                let root =
                    self.instantiate_package(package, Captures::Global(package_instance, captures));
                self.link(root, Node::Global(unbox_instance, continuation));
            }
            sym!(
                NodeRef::Shared(Shared::Sync(shared)),
                NodeRef::Global(
                    unbox_instance,
                    _,
                    Global::Destruct(GlobalCont::Unbox(continuation))
                )
            ) if matches!(shared.as_ref(), SyncShared::Package(..)) => {
                let SyncShared::Package(package, captures) = shared.as_ref() else {
                    unreachable!()
                };
                let continuation = *continuation;
                self.rewrites.instantiate += 1;
                let root = self.instantiate_package(*package, Captures::Shared(captures));
                self.link(root, Node::Global(unbox_instance, continuation));
            }
            sym!(
                NodeRef::Global(instance, _, Global::Package(package, captures, _)),
                other
            ) => {
                self.interact_instantiate(
                    *package,
                    Captures::Global(instance, *captures),
                    other.into_node(),
                );
            }
            sym!(NodeRef::Shared(Shared::Sync(x)), other)
                if matches!(&*x, SyncShared::Package(..)) =>
            {
                self.rewrites.instantiate += 1;
                let SyncShared::Package(package, captures) = &*x else {
                    unreachable!()
                };
                self.interact_instantiate(*package, Captures::Shared(captures), other.into_node());
            }

            sym!(
                NodeRef::Global(_, _, Global::Close { .. }),
                NodeRef::Linear(Linear::Value(value))
            ) if matches!(
                value.as_ref(),
                Value::Break | Value::Primitive(_) | Value::ExternalArc(_)
            ) =>
            {
                self.rewrites.fanout += 1;
            }
            sym!(
                NodeRef::Global(_, _, Global::Close { .. }),
                NodeRef::Global(
                    _,
                    _,
                    Global::Value(Value::Break | Value::Primitive(_) | Value::ExternalArc(_))
                )
            ) => {
                self.rewrites.fanout += 1;
            }
            sym!(
                NodeRef::Global(close_instance, close_index, Global::Close { .. }),
                NodeRef::Linear(Linear::Value(value))
            ) if matches!(value.as_ref(), Value::Pair(..)) => {
                let Value::Pair(left, right) = *value else {
                    unreachable!()
                };
                self.link(left, Node::Global(close_instance.clone(), close_index));
                self.link(right, Node::Global(close_instance, close_index));
                self.rewrites.fanout += 1;
            }
            sym!(
                NodeRef::Global(close_instance, close_index, Global::Close { .. }),
                NodeRef::Global(value_instance, _, Global::Value(Value::Pair(left, right)))
            ) => {
                let left = *left;
                let right = *right;
                self.link(
                    Node::Global(value_instance.clone(), left),
                    Node::Global(close_instance.clone(), close_index),
                );
                self.link(
                    Node::Global(value_instance, right),
                    Node::Global(close_instance, close_index),
                );
                self.rewrites.fanout += 1;
            }
            sym!(
                NodeRef::Global(close_instance, close_index, Global::Close { .. }),
                NodeRef::Linear(Linear::Value(value))
            ) if matches!(value.as_ref(), Value::Either(..)) => {
                let Value::Either(_, payload) = *value else {
                    unreachable!()
                };
                self.link(payload, Node::Global(close_instance, close_index));
                self.rewrites.fanout += 1;
            }
            sym!(
                NodeRef::Global(close_instance, close_index, Global::Close { .. }),
                NodeRef::Global(value_instance, _, Global::Value(Value::Either(_, payload)))
            ) => {
                self.link(
                    Node::Global(value_instance, *payload),
                    Node::Global(close_instance, close_index),
                );
                self.rewrites.fanout += 1;
            }
            sym!(
                NodeRef::Global(close_instance, close_index, Global::Close { signal, .. }),
                NodeRef::Global(
                    choice_instance,
                    _,
                    Global::Destruct(GlobalCont::Choice(options))
                )
            ) => {
                let package = self
                    .lookup_case_branch(*options, *signal)
                    .expect("Close interacted with a choice without a `.close` branch");
                let root = self.instantiate_linked_package_body(choice_instance, package);
                self.link(root, Node::Global(close_instance, close_index));
                self.rewrites.r#match += 1;
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
            sym!(
                NodeRef::Linear(Linear::Value(value)),
                NodeRef::Global(
                    unbox_instance,
                    _,
                    Global::Destruct(GlobalCont::Unbox(continuation))
                )
            ) if matches!(value.as_ref(), Value::ExternalArc(_)) => {
                let Value::ExternalArc(ext) = *value else {
                    unreachable!()
                };
                self.rewrites.ext_call += 1;
                return Some((
                    UserData::ExternalArc(ext),
                    Node::Global(unbox_instance, *continuation),
                ));
            }
            sym!(
                NodeRef::Global(_, _, Global::Value(Value::ExternalArc(ext))),
                NodeRef::Global(
                    unbox_instance,
                    _,
                    Global::Destruct(GlobalCont::Unbox(continuation))
                )
            ) => {
                self.rewrites.ext_call += 1;
                return Some((
                    UserData::ExternalArc(ext.clone()),
                    Node::Global(unbox_instance, *continuation),
                ));
            }
            sym!(
                NodeRef::Shared(Shared::Sync(shared)),
                NodeRef::Global(
                    unbox_instance,
                    _,
                    Global::Destruct(GlobalCont::Unbox(continuation))
                )
            ) if matches!(shared.as_ref(), SyncShared::Value(Value::ExternalArc(_))) => {
                let SyncShared::Value(Value::ExternalArc(ext)) = shared.as_ref() else {
                    unreachable!()
                };
                self.rewrites.ext_call += 1;
                return Some((
                    UserData::ExternalArc(ext.clone()),
                    Node::Global(unbox_instance, *continuation),
                ));
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
                        SyncShared::Package(package, captures) => {
                            let node =
                                self.instantiate_package(*package, Captures::Shared(captures));
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
                    (Value::Either(signal, payload), GlobalCont::Choice(options)) => {
                        let (a, b) =
                            self.interact_either_choice(signal, payload, instance, options);
                        self.link(a, b);
                    }
                    (_, GlobalCont::Unbox(_)) => {
                        unreachable!("unbox must interact with a box representation")
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
                        SyncShared::Package(package, captures) => {
                            let node =
                                self.instantiate_package(*package, Captures::Shared(captures));
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
            SyncShared::Package(_, captures) => format!("Package({} captures)", captures.len()),
            SyncShared::Value(v) => format!("Value({})", v.variant_name()),
        }
    }
}

impl Global<Linked> {
    pub fn variant_name(&self) -> String {
        match self {
            Global::Variable(_) => "Variable".into(),

            Global::Close { .. } => "Close".into(),

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

            GlobalCont::Choice(..) => "Choice".into(),

            GlobalCont::Unbox(_continuation) => "Unbox".into(),
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
