//package: core
use std::cmp::Ordering;

use num_bigint::BigUint;

use crate::builtin::{
    char_::CharClass,
    list::readback_list,
    parser::{ReaderRemainder, provide_string_parser},
};
use par_core::frontend::{ExternalTypeDef, PrimitiveType, Type};
use par_core::source::Span;
use par_runtime::readback::Handle;
use par_runtime::registry::{DefinitionRef, PackageRef};
use par_runtime::{external_def, primitive::ParString};

inventory::submit!(ExternalTypeDef {
    path: DefinitionRef {
        package: PackageRef::CORE,
        path: &[],
        module: "String",
        name: "String"
    },
    doc: r"A primitive type representing a UTF-8 encoded string.",
    typ: Type::Primitive(Span::None, PrimitiveType::String)
});

external_def! {
    @core/String.{
        Builder => string_builder,
        Parse => string_parser,
        ParseReader => string_parser_from_reader,
        Quote => string_quote,
        FromBytes => string_from_bytes,
        ToLower => string_to_lower,
        ToUpper => string_to_upper,
    }
}

async fn string_builder(mut handle: Handle) {
    let mut buf = String::new();
    loop {
        match handle.case().await.as_str() {
            "add" => {
                buf += handle.receive().string().await.as_str();
            }
            "build" => {
                handle.provide_string(ParString::from(buf));
                break;
            }
            _ => unreachable!(),
        }
    }
}

async fn string_quote(mut handle: Handle) {
    let s = handle.receive().string().await;
    handle.provide_string(ParString::from(format!("{:?}", s)));
}

async fn string_parser(mut handle: Handle) {
    let remainder = handle.receive().string().await;
    provide_string_parser(handle, remainder).await;
}

async fn string_parser_from_reader(mut handle: Handle) {
    let reader = handle.receive();
    provide_string_parser(handle, ReaderRemainder::new(reader)).await;
}

async fn string_from_bytes(mut handle: Handle) {
    let bytes = handle.receive().bytes().await;
    handle.provide_string(ParString::from_utf8_lossy(bytes))
}

async fn string_to_lower(mut handle: Handle) {
    let string = handle.receive().string().await;
    handle.provide_string(ParString::from(string.as_str().to_lowercase()));
}

async fn string_to_upper(mut handle: Handle) {
    let string = handle.receive().string().await;
    handle.provide_string(ParString::from(string.as_str().to_uppercase()));
}

#[derive(Debug, Clone)]
pub(super) enum StringPattern {
    Nil,
    All,
    Empty,
    Min(BigUint),
    Max(BigUint),
    Str(ParString),
    One(CharClass),
    Non(CharClass),
    Concat(Box<Self>, Box<Self>),
    And(Box<Self>, Box<Self>),
    Or(Box<Self>, Box<Self>),
    Repeat(Box<Self>),
    Repeat1(Box<Self>),
}

impl StringPattern {
    pub(super) async fn readback(mut handle: Handle) -> Box<Self> {
        match handle.case().await.as_str() {
            "and" => {
                // .and List<self>
                let mut conj = Box::new(Self::All);
                let patterns =
                    readback_list(handle, |handle| Box::pin(Self::readback(handle))).await;
                for p in patterns.into_iter().rev() {
                    conj = Box::new(Self::And(p, conj));
                }
                conj
            }
            "concat" => {
                // .concat List<self>
                let mut conc = Box::new(Self::Empty);
                let patterns =
                    readback_list(handle, |handle| Box::pin(Self::readback(handle))).await;
                for p in patterns.into_iter().rev() {
                    conc = Box::new(Self::Concat(p, conc));
                }
                conc
            }
            "empty" => {
                // .empty!
                handle.continue_();
                Box::new(Self::Empty)
            }
            "min" => {
                // .min Nat
                let n = handle.nat().await;
                Box::new(Self::Min(n))
            }
            "max" => {
                // .max Nat
                let n = handle.nat().await;
                Box::new(Self::Max(n))
            }
            "non" => {
                // .non Char.Class
                let class = CharClass::readback(handle).await;
                Box::new(Self::Non(class))
            }
            "one" => {
                // .one Char.Class
                let class = CharClass::readback(handle).await;
                Box::new(Self::One(class))
            }
            "or" => {
                // .or List<self>,
                let mut disj = Box::new(Self::Nil);
                let patterns =
                    readback_list(handle, |handle| Box::pin(Self::readback(handle))).await;
                for p in patterns.into_iter().rev() {
                    disj = Box::new(Self::Or(p, disj));
                }
                disj
            }
            "repeat" => {
                // .repeat self
                let p = Box::pin(Self::readback(handle)).await;
                Box::new(Self::Repeat(p))
            }
            "repeat1" => {
                // .repeat1 self
                let p = Box::pin(Self::readback(handle)).await;
                Box::new(Self::Repeat1(p))
            }
            "str" => {
                // .str String
                let s = handle.string().await;
                Box::new(Self::Str(s))
            }
            _ => unreachable!(),
        }
    }
}

#[derive(Debug)]
pub(super) struct StringMachine {
    pattern: Box<StringPattern>,
    inner: MachineInner,
}

impl StringMachine {
    pub(super) fn start(pattern: Box<StringPattern>) -> Self {
        let inner = MachineInner::start(&pattern, 0);
        Self { pattern, inner }
    }

    pub(super) fn accepts(&self) -> Option<bool> {
        self.inner.accepts(&self.pattern)
    }

    pub(super) fn advance(&mut self, pos: usize, len: usize, ch: char) {
        self.inner.advance(&self.pattern, pos, len, ch);
    }

    pub(super) fn leftmost_accepting_split(&self) -> Option<usize> {
        let StringPattern::Concat(_, p2) = self.pattern.as_ref() else {
            return None;
        };
        let State::Concat(_, heap) = &self.inner.state else {
            return None;
        };
        heap.iter()
            .filter(|m2| m2.accepts(p2) == Some(true))
            .map(|m2| m2.start)
            .min()
    }

    pub(super) fn leftmost_feasible_split(&self, pos: usize) -> Option<usize> {
        let State::Concat(_, heap) = &self.inner.state else {
            return None;
        };
        heap.iter().map(|m2| m2.start).min().or(Some(pos))
    }
}

#[derive(Debug)]
struct MachineInner {
    state: State,
    start: usize,
}

impl MachineInner {
    fn start(pattern: &StringPattern, start: usize) -> Self {
        let state = match pattern {
            StringPattern::Nil => State::Halt,

            StringPattern::All => State::Init,

            StringPattern::Empty => State::Init,

            StringPattern::Min(_) => State::Index(0),
            StringPattern::Max(_) => State::Index(0),

            StringPattern::Str(_) => State::Index(0),

            StringPattern::One(_) => State::Index(0),
            StringPattern::Non(_) => State::Index(0),

            StringPattern::Concat(p1, p2) => {
                let prefix = Self::start(p1, start);
                let suffixes = if prefix.accepts(p1) == Some(true) {
                    vec![Self::start(p2, start)]
                } else {
                    vec![]
                };
                State::Concat(Box::new(prefix), suffixes)
            }

            StringPattern::And(p1, p2) | StringPattern::Or(p1, p2) => State::Pair(
                Box::new(Self::start(p1, start)),
                Box::new(Self::start(p2, start)),
            ),

            StringPattern::Repeat(_) => State::Init,
            StringPattern::Repeat1(p) => State::Heap(vec![Self::start(p, start)]),
        };

        Self { state, start }
    }

    fn accepts(&self, pattern: &StringPattern) -> Option<bool> {
        match (pattern, &self.state) {
            (_, State::Halt) => None,

            (StringPattern::All, State::Init) => Some(true),

            (StringPattern::Empty, State::Init) => Some(true),

            (StringPattern::Min(n), State::Index(i)) => Some(&BigUint::from(*i) >= n),
            (StringPattern::Max(n), State::Index(i)) => Some(&BigUint::from(*i) <= n),

            (StringPattern::Str(s), State::Index(i)) => Some(s.as_str().len() == *i),

            (StringPattern::One(_), State::Index(i)) => Some(*i == 1),
            (StringPattern::Non(_), State::Index(i)) => Some(*i == 1),

            (StringPattern::Concat(p1, p2), State::Concat(m1, heap)) => heap
                .iter()
                .filter_map(|m2| m2.accepts(p2))
                .max()
                .or_else(|| m1.accepts(p1).map(|_| false)),

            (StringPattern::And(p1, p2), State::Pair(m1, m2)) => {
                match (m1.accepts(p1), m2.accepts(p2)) {
                    (Some(a1), Some(a2)) => Some(a1 && a2),
                    (None, _) | (_, None) => None,
                }
            }

            (StringPattern::Or(p1, p2), State::Pair(m1, m2)) => {
                match (m1.accepts(p1), m2.accepts(p2)) {
                    (Some(a1), Some(a2)) => Some(a1 || a2),
                    (None, a) | (a, None) => a,
                }
            }

            (StringPattern::Repeat(_), State::Init) => Some(true),
            (StringPattern::Repeat(p), State::Heap(heap)) => {
                heap.iter().filter_map(|m| m.accepts(p)).max()
            }

            (StringPattern::Repeat1(p), State::Heap(heap)) => {
                heap.iter().filter_map(|m| m.accepts(p)).max()
            }

            (p, s) => unreachable!("invalid combination of pattern {:?} and state {:?}", p, s),
        }
    }

    fn advance(&mut self, pattern: &StringPattern, pos: usize, len: usize, ch: char) {
        match (pattern, &mut self.state) {
            (_, State::Halt) => {}

            (StringPattern::All, State::Init) => {}

            (StringPattern::Empty, State::Init) => self.state = State::Halt,

            (StringPattern::Min(_), State::Index(i)) => *i += 1,
            (StringPattern::Max(n), State::Index(i)) => {
                if &BigUint::from(*i) < n {
                    *i += 1;
                } else {
                    self.state = State::Halt;
                }
            }

            (StringPattern::Str(s), State::Index(i)) => {
                if s.substr(*i..).as_str().chars().next() == Some(ch) {
                    *i += ch.len_utf8();
                } else {
                    self.state = State::Halt;
                }
            }

            (StringPattern::One(class), State::Index(i)) => {
                if *i == 0 && class.contains(ch) {
                    *i = 1;
                } else {
                    self.state = State::Halt;
                }
            }
            (StringPattern::Non(class), State::Index(i)) => {
                if *i == 0 && !class.contains(ch) {
                    *i = 1;
                } else {
                    self.state = State::Halt;
                }
            }

            (StringPattern::Concat(p1, p2), State::Concat(m1, heap)) => {
                m1.advance(p1, pos, len, ch);
                for m2 in heap.iter_mut() {
                    m2.advance(p2, pos, len, ch);
                }
                heap.retain(|m2| m2.state != State::Halt);
                if m1.accepts(p1) == Some(true) {
                    heap.push(Self::start(p2, pos + len));
                }
                heap.sort_by_key(|m| m.start);
                heap.sort();
                heap.dedup();
                if m1.state == State::Halt && heap.is_empty() {
                    self.state = State::Halt;
                }
            }

            (StringPattern::And(p1, p2), State::Pair(m1, m2)) => {
                m1.advance(p1, pos, len, ch);
                m2.advance(p2, pos, len, ch);
                if m1.state == State::Halt || m2.state == State::Halt {
                    self.state = State::Halt;
                }
            }

            (StringPattern::Or(p1, p2), State::Pair(m1, m2)) => {
                m1.advance(p1, pos, len, ch);
                m2.advance(p2, pos, len, ch);
                if m1.state == State::Halt && m2.state == State::Halt {
                    self.state = State::Halt;
                }
            }

            (StringPattern::Repeat(p), State::Init) => {
                let mut m = Self::start(p, pos);
                m.advance(p, pos, len, ch);
                self.state = State::Heap(vec![m])
            }
            (StringPattern::Repeat(p) | StringPattern::Repeat1(p), State::Heap(heap)) => {
                if heap.iter().any(|m| m.accepts(p) == Some(true)) {
                    heap.push(Self::start(p, pos));
                }
                for m in heap.iter_mut() {
                    m.advance(p, pos, len, ch);
                }
                heap.retain(|m| m.state != State::Halt);
                heap.sort_by_key(|m| m.start);
                heap.sort();
                heap.dedup();
                if heap.is_empty() {
                    self.state = State::Halt;
                }
            }

            (p, s) => unreachable!("invalid combination of pattern {:?} and state {:?}", p, s),
        }
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum State {
    Init,
    Halt,
    Index(usize),
    Pair(Box<MachineInner>, Box<MachineInner>),
    Heap(Vec<MachineInner>),
    Concat(Box<MachineInner>, Vec<MachineInner>),
}

impl PartialEq for MachineInner {
    fn eq(&self, other: &Self) -> bool {
        self.state == other.state
    }
}
impl Eq for MachineInner {}
impl PartialOrd for MachineInner {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.state.partial_cmp(&other.state)
    }
}
impl Ord for MachineInner {
    fn cmp(&self, other: &Self) -> Ordering {
        self.state.cmp(&other.state)
    }
}
