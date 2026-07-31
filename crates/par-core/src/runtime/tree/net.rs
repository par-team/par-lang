//! This module contains a simple implementation for Interaction Combinators.
//! It is not performant; it's mainly here to act as a storage and interchange format

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use arcstr::ArcStr;
use bytes::Bytes;
use futures::channel::oneshot;
use indexmap::IndexMap;
use num_bigint::BigInt;

use crate::frontend_impl::process::CLEANUP_BRANCH;
use par_runtime::fan_behavior::FanBehavior;
use par_runtime::primitive::{ParString, Primitive, format_float};
use par_runtime::readback::Number;

pub(crate) type VarId = usize;

pub(crate) fn number_to_string(mut number: usize) -> String {
    let mut result = String::new();
    number += 1;
    while number > 0 {
        let remainder = (number - 1) % 26;
        let character = (b'a' + remainder as u8) as char;
        result.insert(0, character);
        number = (number - 1) / 26;
    }
    result
}

/// A `Tree` corresponds to a port that is the root of a tree of interaction combinators.
/// The `Tree` enum itself contains the whole tree, although it some parts of it might be inside
/// half-linked `Tree::Var`s
pub enum Tree<Ext> {
    Break,
    Continue,
    Era,
    Close,
    Par(Box<Tree<Ext>>, Box<Tree<Ext>>),
    Times(Box<Tree<Ext>>, Box<Tree<Ext>>),
    Dup(Box<Tree<Ext>>, Box<Tree<Ext>>),
    Signal(ArcStr, Box<Tree<Ext>>),
    Choice(Box<Tree<Ext>>, Arc<HashMap<ArcStr, usize>>, Option<usize>),
    Var(usize),
    Package(usize, Box<Tree<Ext>>, FanBehavior),

    SignalRequest(oneshot::Sender<(ArcStr, Box<Tree<Ext>>)>),

    Primitive(Primitive),
    IntRequest(oneshot::Sender<BigInt>),
    StringRequest(oneshot::Sender<ParString>),
    BytesRequest(oneshot::Sender<Bytes>),

    External(Ext),
    ExternalBox(
        Arc<
            dyn Send
                + Sync
                + Fn(par_runtime::readback::Handle) -> Pin<Box<dyn Send + Future<Output = ()>>>,
        >,
    ),
}

impl<Ext> Tree<Ext> {
    pub fn map_vars(&mut self, m: &mut impl FnMut(VarId) -> VarId) {
        match self {
            Self::Var(x) => *x = m(*x),
            Self::Par(a, b) | Self::Times(a, b) => {
                a.map_vars(m);
                b.map_vars(m);
            }
            Self::Package(_, context, _) => {
                context.map_vars(m);
            }
            Self::Signal(_, payload) => {
                payload.map_vars(m);
            }
            Self::Choice(context, _, _) => {
                context.map_vars(m);
            }
            Self::Dup(a, b) => {
                a.map_vars(m);
                b.map_vars(m);
            }
            Self::Era
            | Self::Close
            | Self::Break
            | Self::Continue
            | Self::Primitive(_)
            | Self::SignalRequest(_)
            | Self::IntRequest(_)
            | Self::StringRequest(_)
            | Self::BytesRequest(_)
            | Self::External(_)
            | Self::ExternalBox(_) => {}
        }
    }
}

impl<Ext> core::fmt::Debug for Tree<Ext> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Era => f.debug_tuple("Era").finish(),
            Self::Close => f.debug_tuple("Close").finish(),
            Self::Break => f.debug_tuple("Break").finish(),
            Self::Continue => f.debug_tuple("Continue").finish(),
            Self::Times(a, b) => f.debug_tuple("Times").field(a).field(b).finish(),
            Self::Par(a, b) => f.debug_tuple("Par").field(a).field(b).finish(),
            Self::Dup(a, b) => f.debug_tuple("Dup").field(a).field(b).finish(),
            Self::Package(id, context, b) => f
                .debug_tuple("Package")
                .field(id)
                .field(context)
                .field(b)
                .finish(),
            Self::Signal(signal, payload) => f
                .debug_tuple("Signal")
                .field(signal)
                .field(payload)
                .finish(),
            Self::Choice(context, branches, else_branch) => f
                .debug_tuple("Choice")
                .field(context)
                .field(branches)
                .field(else_branch)
                .finish(),
            Self::Var(id) => f.debug_tuple("Var").field(id).finish(),
            Self::Primitive(p) => f.debug_tuple("Primitive").field(p).finish(),
            Self::SignalRequest(_) => f.debug_tuple("SignalRequest").field(&"<channel>").finish(),
            Self::IntRequest(_) => f.debug_tuple("IntRequest").field(&"<channel>").finish(),
            Self::StringRequest(_) => f.debug_tuple("StringRequest").field(&"<channel>").finish(),
            Self::BytesRequest(_) => f.debug_tuple("BytesRequest").field(&"<channel>").finish(),
            Self::External(_) => f.debug_tuple("External").field(&"<function>").finish(),
            Self::ExternalBox(_) => f.debug_tuple("ExternalBox").field(&"<closure>").finish(),
        }
    }
}

impl<Ext: Clone> Clone for Tree<Ext> {
    fn clone(&self) -> Self {
        match self {
            Self::Era => Self::Era,
            Self::Close => Self::Close,
            Self::Break => Self::Break,
            Self::Continue => Self::Continue,
            Self::Times(a, b) => Self::Times(a.clone(), b.clone()),
            Self::Par(a, b) => Self::Par(a.clone(), b.clone()),
            Self::Dup(a, b) => Self::Dup(a.clone(), b.clone()),
            Self::Package(package, context, b) => {
                Self::Package(package.clone(), context.clone(), b.clone())
            }
            Self::Signal(signal, payload) => Self::Signal(signal.clone(), payload.clone()),
            Self::Choice(context, branches, else_branch) => {
                Self::Choice(context.clone(), Arc::clone(branches), else_branch.clone())
            }
            Self::Var(id) => Self::Var(id.clone()),
            Self::Primitive(p) => Self::Primitive(p.clone()),
            Self::SignalRequest(_) => panic!("cannot clone Tree::SignalRequest"),
            Self::IntRequest(_) => panic!("cannot clone Tree::IntRequest"),
            Self::StringRequest(_) => panic!("cannot clone Tree::StringRequest"),
            Self::BytesRequest(_) => panic!("cannot clone Tree::BytesRequest"),
            Self::External(f) => Self::External(f.clone()),
            Self::ExternalBox(f) => Self::ExternalBox(Arc::clone(f)),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Rewrites {
    pub commute: u128,
    pub annihilate: u128,
    pub signal: u128,
    pub era: u128,
    pub resp: u128,
    pub expand: u128,
    pub derelict: u128,
    last_busy_start: Option<Instant>,
    pub busy_duration: Duration,
}

impl core::ops::Add<Rewrites> for Rewrites {
    type Output = Rewrites;

    fn add(self, rhs: Rewrites) -> Self::Output {
        Self {
            commute: self.commute + rhs.commute,
            annihilate: self.annihilate + rhs.annihilate,
            signal: self.signal + rhs.signal,
            era: self.era + rhs.era,
            expand: self.expand + rhs.expand,
            resp: self.resp + rhs.resp,
            derelict: self.derelict + rhs.derelict,
            last_busy_start: match (self.last_busy_start, rhs.last_busy_start) {
                (None, None) => None,
                (Some(t), None) | (None, Some(t)) => Some(t),
                (Some(t1), Some(t2)) => Some(t1.min(t2)),
            },
            busy_duration: self.busy_duration + rhs.busy_duration,
        }
    }
}

#[allow(unused)]
impl Rewrites {
    pub fn total(&self) -> u128 {
        self.commute + self.annihilate + self.signal + self.expand + self.era + self.resp
    }

    pub fn total_per_second(&self) -> u128 {
        let micros = self.busy_duration.as_micros();
        if micros == 0 {
            return 0;
        }
        (self.total() as u128 * 1_000_000) / micros
    }
}

#[derive(Clone)]
/// A Net represents the current state of the runtime
/// It contains a list of active pairs, as well as a list of free ports.
/// It also stores a map of variables, which records whether variables were linked by either of their sides
pub struct Net<Ext> {
    pub ports: VecDeque<Tree<Ext>>,
    pub redexes: VecDeque<(Tree<Ext>, Tree<Ext>)>,
    pub variables: Variables<Ext>,
    pub packages: Arc<IndexMap<usize, Net<Ext>>>,
    pub rewrites: Rewrites,
    pub waiting_for_reducer: Vec<(Tree<Ext>, Tree<Ext>)>,
    pub debug_name: String,
}

impl<Ext> Default for Net<Ext> {
    fn default() -> Self {
        Self {
            ports: VecDeque::new(),
            redexes: VecDeque::new(),
            variables: Variables::default(),
            packages: Arc::new(IndexMap::new()),
            rewrites: Rewrites::default(),
            waiting_for_reducer: Vec::new(),
            debug_name: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Variables<Ext> {
    vars: Vec<VarState<Ext>>,
    free: Vec<VarId>,
}

impl<Ext> Default for Variables<Ext> {
    fn default() -> Self {
        Self {
            vars: Vec::new(),
            free: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum VarState<Ext> {
    Free,
    Linked(Tree<Ext>),
}

impl<Ext> Variables<Ext> {
    pub fn get(&self, id: VarId) -> Option<&VarState<Ext>> {
        self.vars.get(id)
    }

    pub fn remove_linked(&mut self, id: VarId) -> Result<Tree<Ext>, &mut VarState<Ext>> {
        while self.vars.len() <= id {
            self.vars.push(VarState::Free);
        }

        match self.vars.get_mut(id) {
            Some(state) => match state {
                VarState::Linked(tree) => {
                    self.free.push(id);
                    let tree = std::mem::replace(tree, Tree::Era);
                    let _ = std::mem::replace(state, VarState::Free);
                    Ok(tree)
                }
                _ => Err(state),
            },
            None => unreachable!(),
        }
    }

    pub fn alloc(&mut self) -> VarId {
        match self.free.pop() {
            Some(id) => id,
            None => {
                self.vars.push(VarState::Free);
                self.vars.len() - 1
            }
        }
    }
}

impl<Ext: Clone> Net<Ext> {
    fn interact(&mut self, a: Tree<Ext>, b: Tree<Ext>) {
        macro_rules! sym {
            ($a: pat, $b: pat) => {
                ($a, $b) | ($b, $a)
            };
        }

        use Tree::*;
        match (a, b) {
            (a @ Var(..), b @ _) | (a @ _, b @ Var(..)) => {
                // link anyway
                self.link(a, b);
            }
            sym!(Era | Continue, Break) | sym!(Continue, Era) => {
                self.rewrites.era += 1;
            }
            sym!(Close, Break | Era) => {
                self.rewrites.era += 1;
            }
            sym!(Times(a0, a1), Close) => {
                self.link(*a0, Close);
                self.link(*a1, Close);
                self.rewrites.era += 1;
            }
            sym!(Signal(_, payload), Close) => {
                self.link(*payload, Close);
                self.rewrites.era += 1;
            }
            sym!(Dup(a0, a1), Close) => {
                self.link(*a0, Era);
                self.link(*a1, Era);
                self.rewrites.era += 1;
            }
            sym!(Times(a0, a1), Era) | sym!(Par(a0, a1), Era) | sym!(Dup(a0, a1), Era) => {
                self.link(*a0, Era);
                self.link(*a1, Era);
                self.rewrites.era += 1;
            }
            sym!(Times(a0, a1), Par(b0, b1)) | (Dup(a0, a1), Dup(b0, b1)) => {
                self.link(*a0, *b0);
                self.link(*a1, *b1);
                self.rewrites.annihilate += 1;
            }
            sym!(Dup(a0, a1), Break) => {
                self.link(*a0, Break);
                self.link(*a1, Break);
                self.rewrites.era += 1;
            }
            sym!(Dup(a0, a1), Continue) => {
                self.link(*a0, Continue);
                self.link(*a1, Continue);
                self.rewrites.era += 1;
            }
            sym!(Times(a0, a1), Dup(b0, b1)) => {
                let (a00, b00) = self.create_wire();
                let (a01, b01) = self.create_wire();
                let (a10, b10) = self.create_wire();
                let (a11, b11) = self.create_wire();
                self.link(*a0, Tree::Dup(Box::new(a00), Box::new(a01)));
                self.link(*a1, Tree::Dup(Box::new(a10), Box::new(a11)));
                self.link(*b0, Tree::Times(Box::new(b00), Box::new(b10)));
                self.link(*b1, Tree::Times(Box::new(b01), Box::new(b11)));
                self.rewrites.commute += 1;
            }
            sym!(Par(a0, a1), Dup(b0, b1)) => {
                let (a00, b00) = self.create_wire();
                let (a01, b01) = self.create_wire();
                let (a10, b10) = self.create_wire();
                let (a11, b11) = self.create_wire();
                self.link(*a0, Tree::Dup(Box::new(a00), Box::new(a01)));
                self.link(*a1, Tree::Dup(Box::new(a10), Box::new(a11)));
                self.link(*b0, Tree::Par(Box::new(b00), Box::new(b10)));
                self.link(*b1, Tree::Par(Box::new(b01), Box::new(b11)));
                self.rewrites.commute += 1;
            }
            (Signal(signal, payload), Choice(context, branches, else_branch))
            | (Choice(context, branches, else_branch), Signal(signal, payload)) => {
                match branches.get(&signal).copied() {
                    Some(package_id) => {
                        self.link(
                            *payload,
                            Tree::Package(package_id, context, FanBehavior::Expand),
                        );
                    }
                    None => self.link(
                        Signal(signal, payload),
                        Tree::Package(else_branch.unwrap(), context, FanBehavior::Expand),
                    ),
                }
                self.rewrites.signal += 1;
            }
            sym!(Close, Choice(context, branches, _)) => {
                let package = branches
                    .get(CLEANUP_BRANCH)
                    .copied()
                    .expect("Close interacted with a choice without a cleanup branch");
                self.link(Close, Tree::Package(package, context, FanBehavior::Expand));
                self.rewrites.signal += 1;
            }
            (Signal(_, payload), Era) | (Era, Signal(_, payload)) => {
                self.link(*payload, Era);
                self.rewrites.era += 1;
            }
            (Signal(signal, payload), Dup(b0, b1)) | (Dup(b0, b1), Signal(signal, payload)) => {
                let (a00, b00) = self.create_wire();
                let (a10, b10) = self.create_wire();
                self.link(*payload, Tree::Dup(Box::new(b00), Box::new(b10)));
                self.link(*b0, Tree::Signal(signal.clone(), Box::new(a00)));
                self.link(*b1, Tree::Signal(signal, Box::new(a10)));
                self.rewrites.commute += 1;
            }
            (Package(_, c, FanBehavior::Propagate), Era)
            | (Era, Package(_, c, FanBehavior::Propagate)) => {
                self.rewrites.era += 1;
                self.link(*c, Tree::Era);
            }
            (Package(_, c, FanBehavior::Propagate), Close)
            | (Close, Package(_, c, FanBehavior::Propagate)) => {
                self.rewrites.era += 1;
                self.link(*c, Tree::Era);
            }
            (Package(id, cx, FanBehavior::Propagate), Dup(d0, d1))
            | (Dup(d0, d1), Package(id, cx, FanBehavior::Propagate)) => {
                let (a0, b0) = self.create_wire();
                let (a1, b1) = self.create_wire();
                self.link(*cx, Tree::Dup(Box::new(a0), Box::new(a1)));
                self.link(*d0, Package(id, Box::new(b1), FanBehavior::Propagate));
                self.link(*d1, Package(id, Box::new(b0), FanBehavior::Propagate));
                self.rewrites.commute += 1;
            }
            (Package(id, cx, _), a) | (a, Package(id, cx, _)) => {
                let b = self.dereference_package(id, *cx);
                self.interact(a, b);
                self.rewrites.expand += 1;
            }
            (Primitive(p), a) | (a, Primitive(p)) => {
                self.primitive_interact(p, a);
            }
            sym!(SignalRequest(tx), Close) => {
                tx.send((ArcStr::from(CLEANUP_BRANCH), Box::new(Close)))
                    .expect("receiver dropped");
                self.rewrites.resp += 1;
            }
            (SignalRequest(tx), Signal(signal, payload))
            | (Signal(signal, payload), SignalRequest(tx)) => {
                tx.send((signal, payload)).expect("receiver dropped");
                self.rewrites.resp += 1;
            }
            (External(f), a) | (a, External(f)) => self.waiting_for_reducer.push((External(f), a)),
            (ExternalBox(_), Era) | (Era, ExternalBox(_)) => {
                self.rewrites.era += 1;
            }
            (ExternalBox(_), Close) | (Close, ExternalBox(_)) => {
                self.rewrites.era += 1;
            }
            (ExternalBox(f), Dup(a, b)) | (Dup(a, b), ExternalBox(f)) => {
                self.link(*a, ExternalBox(Arc::clone(&f)));
                self.link(*b, ExternalBox(f));
                self.rewrites.commute += 1;
            }
            (ExternalBox(f), a) | (a, ExternalBox(f)) => {
                self.waiting_for_reducer.push((ExternalBox(f), a))
            }
            (a, b) => panic!("Invalid combinator interaction: {:?} <> {:?}", a, b),
        }
    }

    fn primitive_interact(&mut self, p: Primitive, tree: Tree<Ext>) {
        match (p, tree) {
            (Primitive::Number(Number::Zero), Tree::IntRequest(resp)) => {
                resp.send(BigInt::ZERO).expect("receiver dropped");
                self.rewrites.resp += 1;
            }
            (Primitive::Number(Number::Int(i)), Tree::IntRequest(resp)) => {
                resp.send(i).expect("receiver dropped");
                self.rewrites.resp += 1;
            }
            (Primitive::String(s), Tree::StringRequest(resp)) => {
                resp.send(s).expect("receiver dropped");
                self.rewrites.resp += 1;
            }
            (Primitive::Bytes(b), Tree::BytesRequest(resp)) => {
                resp.send(b).expect("receiver dropped");
                self.rewrites.resp += 1;
            }
            (Primitive::String(s), Tree::BytesRequest(resp)) => {
                resp.send(s.as_bytes()).expect("receiver dropped");
                self.rewrites.resp += 1;
            }

            (_, Tree::Era) => {
                self.rewrites.era += 1;
            }
            (_, Tree::Close) => {
                self.rewrites.era += 1;
            }
            (p, Tree::Dup(a, b)) => {
                self.link(Tree::Primitive(p.clone()), *a);
                self.link(Tree::Primitive(p), *b);
                self.rewrites.commute += 1;
            }

            (p, tree) => unreachable!("Invalid primitive interaction of {:?} with {:?}", p, tree),
        }
    }

    fn dereference_package(&mut self, package: usize, cx: Tree<Ext>) -> Tree<Ext> {
        let mut net = self
            .packages
            .get(&package)
            .unwrap_or_else(|| panic!("Unknown package with ID {}", package))
            .clone();
        self.alter_net(&mut net);
        let cx_ = net.ports.pop_back().unwrap();
        let root = net.ports.pop_back().unwrap();
        self.link(cx, cx_);
        root
    }

    pub fn alter_net(&mut self, net: &mut Net<Ext>) {
        // Now, we have to freshen all variables in the tree
        let mut allocated = HashMap::new();
        net.map_vars(&mut |id| {
            *allocated
                .entry(id)
                .or_insert_with(|| self.variables.alloc())
        });
        self.redexes.append(&mut net.redexes);
        self.waiting_for_reducer
            .append(&mut net.waiting_for_reducer);
        for (id, state) in net.variables.vars.drain(..).enumerate() {
            if let Some(new_id) = allocated.get(&id) {
                self.variables.vars[*new_id] = state
            }
        }
        self.rewrites = core::mem::take(&mut self.rewrites) + net.rewrites.clone();
    }

    pub fn inject_net(&mut self, mut net: Net<Ext>) -> Tree<Ext> {
        assert!(net.ports.len() == 2);
        self.alter_net(&mut net);
        let cx = net.ports.pop_back().unwrap();
        let root = net.ports.pop_back().unwrap();
        self.link(cx, Tree::Break);
        root
    }

    /// Returns whether a reduction was carried out
    pub fn reduce_one(&mut self) -> bool {
        #[cfg(not(target_family = "wasm"))]
        if self.rewrites.last_busy_start.is_none() {
            self.rewrites.last_busy_start = Some(Instant::now());
        }

        let reduced = match self.redexes.pop_front() {
            Some((a, b)) => {
                self.interact(a, b);
                true
            }
            _ => false,
        };

        if !reduced {
            if let Some(last_busy_start) = self.rewrites.last_busy_start.take() {
                self.rewrites.busy_duration += last_busy_start.elapsed();
            }
        }

        reduced
    }

    /// Where vars occur in the given tree which already have been linked from the other side, finish linking them.
    pub fn substitute_tree(&mut self, tree: &mut Tree<Ext>) {
        match tree {
            Tree::Times(a, b) | Tree::Par(a, b) | Tree::Dup(a, b) => {
                self.substitute_tree(a);
                self.substitute_tree(b);
            }
            Tree::Package(_, context, _) => self.substitute_tree(context),
            Tree::Signal(_, payload) => self.substitute_tree(payload),
            Tree::Choice(context, _, _) => self.substitute_tree(context),
            Tree::Var(id) => {
                if let Ok(mut a) = self.variables.remove_linked(*id) {
                    self.substitute_tree(&mut a);
                    *tree = a;
                }
            }
            Tree::Era
            | Tree::Close
            | Tree::Continue
            | Tree::Break
            | Tree::Primitive(_)
            | Tree::SignalRequest(_)
            | Tree::IntRequest(_)
            | Tree::StringRequest(_)
            | Tree::BytesRequest(_)
            | Tree::External(_)
            | Tree::ExternalBox(_) => {}
        }
    }

    pub fn normal(&mut self, max_interactions: u32) {
        let mut interaction_count: u32 = 0;
        while self.reduce_one() {
            interaction_count += 1;
            if interaction_count > max_interactions {
                break;
            }
        }
        // dereference all variables
        let mut ports = core::mem::take(&mut self.ports);
        ports.iter_mut().for_each(|x| self.substitute_tree(x));
        self.ports = ports;
    }

    pub fn link(&mut self, a: Tree<Ext>, b: Tree<Ext>) {
        match (a, b) {
            (Tree::Var(mut id), y) | (y, Tree::Var(mut id)) => loop {
                match self.variables.remove_linked(id) {
                    Ok(Tree::Var(id2)) => {
                        id = id2;
                    }
                    Ok(x) => {
                        self.link(x, y);
                        break;
                    }
                    Err(state) => {
                        *state = VarState::Linked(y);
                        break;
                    }
                }
            },
            (Tree::Era, y) | (y, Tree::Era) => self.redexes.push_front((Tree::Era, y)),
            (x, y) => self.redexes.push_back((x, y)),
        }
    }

    pub fn create_wire(&mut self) -> (Tree<Ext>, Tree<Ext>) {
        let id = self.variables.alloc();
        (Tree::Var(id), Tree::Var(id))
    }

    pub fn map_vars(&mut self, m: &mut impl FnMut(VarId) -> VarId) {
        for port in &mut self.ports {
            port.map_vars(m);
        }
        for (a, b) in &mut self.redexes {
            a.map_vars(m);
            b.map_vars(m);
        }
        for (a, b) in &mut self.waiting_for_reducer {
            a.map_vars(m);
            b.map_vars(m);
        }
        for state in &mut self.variables.vars {
            if let VarState::Linked(tree) = state {
                tree.map_vars(m);
            }
        }
        //for free in &mut self.variables.free {
        //    *free = m(*free);
        //}
    }

    pub fn show(&self) -> String {
        self.show_indent(0)
    }

    pub fn show_indent(&self, indent: usize) -> String {
        use core::fmt::Write;
        let indent_string = "    ".repeat(indent);
        let mut s = String::new();
        for i in &self.ports {
            write!(&mut s, "{}{}\n", indent_string, self.show_tree(i)).unwrap();
        }
        for (a, b) in self.redexes.iter().chain(self.waiting_for_reducer.iter()) {
            write!(
                &mut s,
                "{}{} ~ {}\n",
                indent_string,
                self.show_tree(a),
                self.show_tree(b)
            )
            .unwrap();
        }
        s
    }

    pub fn show_tree(&self, t: &Tree<Ext>) -> String {
        match t {
            Tree::Var(id) => {
                if let Some(VarState::Linked(b)) = self.variables.get(*id) {
                    self.show_tree(b)
                } else {
                    number_to_string(*id)
                }
            }
            Tree::Era => format!("*"),
            Tree::Close => format!("close"),
            Tree::Break => format!("!"),
            Tree::Continue => format!("?"),
            Tree::Par(a, b) => format!("[{}] {}", self.show_tree(b), self.show_tree(a)),
            Tree::Times(a, b) => format!("({}) {}", self.show_tree(b), self.show_tree(a)),
            Tree::Dup(a, b) => format!("{{{} {}}}", self.show_tree(a), self.show_tree(b)),
            Tree::Package(id, cx, _) if matches!(cx.as_ref(), &Tree::Break) => format!("@{}", id),
            Tree::Package(id, cx, _) => format!("@{}${}", id, self.show_tree(cx)),
            Tree::Signal(signal, payload) => {
                format!("signal({} {})", signal, self.show_tree(payload))
            }
            Tree::Choice(context, branches, else_branch) => {
                format!(
                    "choice({} {:?} {:?})",
                    self.show_tree(context),
                    branches,
                    else_branch
                )
            }

            Tree::Primitive(Primitive::Number(Number::Zero)) => String::from("primitive(0)"),
            Tree::Primitive(Primitive::Number(Number::Int(i))) => format!("primitive({})", i),
            Tree::Primitive(Primitive::Number(Number::Float(value))) => {
                format!("primitive({})", format_float(*value))
            }
            Tree::Primitive(Primitive::String(s)) => format!("primitive({:?})", s),
            Tree::Primitive(Primitive::Bytes(b)) => format!("primitive({:?})", b),

            Tree::SignalRequest(_) => format!("<signal request>"),
            Tree::IntRequest(_) => format!("<int request>"),
            Tree::StringRequest(_) => format!("<string request>"),
            Tree::BytesRequest(_) => format!("<bytes request>"),

            Tree::External(_) => format!("<external>"),
            Tree::ExternalBox(_) => format!("<external box>"),
        }
    }

    fn assert_no_vicious(&self) {
        for (var, tree) in self.variables.vars.iter().enumerate() {
            if let VarState::Linked(tree) = tree {
                self.assert_tree_not_contains(tree, &var);
            }
        }
    }

    fn assert_tree_not_contains(&self, tree: &Tree<Ext>, idx: &usize) {
        match tree {
            Tree::Par(a, b) | Tree::Times(a, b) | Tree::Dup(a, b) => {
                self.assert_tree_not_contains(a, idx);
                self.assert_tree_not_contains(b, idx);
            }
            Tree::Package(_, context, _) => {
                self.assert_tree_not_contains(context, idx);
            }
            Tree::Signal(_, payload) => {
                self.assert_tree_not_contains(payload, idx);
            }
            Tree::Choice(context, _, _) => {
                self.assert_tree_not_contains(context, idx);
            }
            Tree::Var(id) => {
                if id == idx {
                    panic!("Vicious circle detected");
                }
                if let Some(VarState::Linked(tree)) = self.variables.get(*id) {
                    self.assert_tree_not_contains(tree, idx);
                }
            }
            Tree::Era
            | Tree::Close
            | Tree::Continue
            | Tree::Break
            | Tree::Primitive(_)
            | Tree::SignalRequest(_)
            | Tree::IntRequest(_)
            | Tree::StringRequest(_)
            | Tree::BytesRequest(_)
            | Tree::External(_)
            | Tree::ExternalBox(_) => {}
        }
    }

    pub fn assert_valid_with<'a>(&self, iter: impl Iterator<Item = &'a Tree<Ext>>)
    where
        Ext: 'a,
    {
        self.assert_no_vicious();

        let mut vars = vec![];
        for (a, b) in &self.redexes {
            vars.append(&mut self.assert_tree_valid(a));
            vars.append(&mut self.assert_tree_valid(b));
        }
        for (a, b) in &self.waiting_for_reducer {
            vars.append(&mut self.assert_tree_valid(a));
            vars.append(&mut self.assert_tree_valid(b));
        }
        for tree in &self.ports {
            vars.append(&mut self.assert_tree_valid(tree));
        }
        for tree in iter {
            vars.append(&mut self.assert_tree_valid(tree));
        }

        let vars_counter = vars.into_iter().fold(BTreeMap::new(), |mut acc, x| {
            *acc.entry(x).or_insert(0) += 1;
            acc
        });
        for (var, count) in vars_counter {
            if count != 2 {
                let name = number_to_string(var.clone());
                println!("net = {}", self.show());
                println!("num ports {}", self.ports.len());
                panic!("Variable {name} was used {count} times");
            }
        }

        // Perhaps we should check that all packages are valid too
        // Right now this creates nonsensical error messages
        // And in any case, each package is checked when it is created
    }

    pub fn assert_valid(&self) {
        self.assert_valid_with(std::iter::empty());
    }

    fn assert_tree_valid(&self, tree: &Tree<Ext>) -> Vec<usize> {
        match tree {
            Tree::Times(a, b) | Tree::Par(a, b) | Tree::Dup(a, b) => {
                let mut a = self.assert_tree_valid(a.as_ref());
                let mut b = self.assert_tree_valid(b.as_ref());
                a.append(&mut b);
                a
            }
            Tree::Package(idx, context, _) => {
                if self.packages.get(idx).is_some() {
                    self.assert_tree_valid(context)
                } else {
                    panic!("Package with id {idx} is not found")
                }
            }
            Tree::Signal(_, payload) => self.assert_tree_valid(payload),
            Tree::Choice(context, _, _) => self.assert_tree_valid(context),
            Tree::Era | Tree::Close | Tree::Continue | Tree::Break => {
                vec![]
            }
            Tree::Var(idx) => {
                if let Some(VarState::Linked(tree)) = self.variables.get(*idx) {
                    self.assert_tree_valid(tree)
                } else {
                    vec![idx.clone()]
                }
            }
            Tree::Primitive(_) => vec![],
            Tree::SignalRequest(_) => vec![],
            Tree::IntRequest(_) => vec![],
            Tree::StringRequest(_) => vec![],
            Tree::BytesRequest(_) => vec![],
            Tree::External(_) => vec![],
            Tree::ExternalBox(_) => vec![],
        }
    }
}
