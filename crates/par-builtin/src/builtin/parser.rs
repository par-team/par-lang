//package: core
use crate::builtin::{
    bytes::{BytesMachine, BytesPattern},
    string::{StringMachine, StringPattern},
};
use arcstr::literal;
use bytes::Bytes;
use par_core::frontend::ParString;
use par_runtime::readback::Handle;
use std::collections::VecDeque;

pub(super) trait BytesRemainder {
    type Err;
    type Iterator<'a>: AsyncByteIterator<Err = Self::Err>
    where
        Self: 'a;

    async fn provide_err(handle: Handle, err: Self::Err);

    async fn close(self) -> Result<(), Self::Err>;
    async fn peek_byte(&mut self) -> Result<Option<u8>, Self::Err>;
    fn bytes(&mut self) -> Self::Iterator<'_>;
    fn pop_bytes(&mut self, n: usize) -> Bytes;
    async fn remaining_bytes(&mut self) -> Result<Bytes, Self::Err>;
}

pub(super) trait CharsRemainder {
    type Err;
    type Iterator<'a>: AsyncCharIterator<Err = Self::Err>
    where
        Self: 'a;

    async fn provide_err(handle: Handle, err: Self::Err);

    async fn close(self) -> Result<(), Self::Err>;
    async fn peek_byte(&mut self) -> Result<Option<u8>, Self::Err>;
    fn chars(&mut self) -> Self::Iterator<'_>;
    fn pop_chars(&mut self, n: usize) -> ParString;
    async fn remaining_chars(&mut self) -> Result<ParString, Self::Err>;
}

pub(super) trait AsyncByteIterator {
    type Err;
    async fn next(&mut self) -> Result<Option<(usize, u8)>, Self::Err>;
}

pub(super) trait AsyncCharIterator {
    type Err;
    async fn next(&mut self) -> Result<Option<(usize, usize, char)>, Self::Err>;
}

pub(super) enum Never {}

impl ToString for Never {
    fn to_string(&self) -> String {
        "Never".to_string()
    }
}

// A generic remainder that adapts a runtime `Bytes.Reader<e>` handle
// into the `BytesRemainder` and `CharsRemainder` traits. Errors are forwarded
// opaquely by passing handles through without interpretation.
pub(super) struct ReaderRemainder {
    handle: Option<Handle>,
    buffer: VecDeque<u8>,
}

impl ReaderRemainder {
    pub(super) fn new(handle: Handle) -> Self {
        Self {
            handle: Some(handle),
            buffer: VecDeque::new(),
        }
    }
}

pub(super) struct ReaderRemainderByteIterator<'a> {
    remainder: &'a mut ReaderRemainder,
    index: usize,
}

impl<'a> AsyncByteIterator for ReaderRemainderByteIterator<'a> {
    // Forward opaque error values via raw Handle
    type Err = Handle;

    async fn next(&mut self) -> Result<Option<(usize, u8)>, Self::Err> {
        // Serve from buffer if available
        if let Some(b) = self.remainder.buffer.get(self.index) {
            self.index += 1;
            return Ok(Some((self.index - 1, *b)));
        }

        if self.remainder.handle.is_none() {
            return Ok(None);
        }

        // Request more bytes from the underlying reader
        loop {
            let handle = self.remainder.handle.as_mut().unwrap();
            handle.signal(literal!("read"));
            match handle.case().await.as_str() {
                "ok" => match handle.case().await.as_str() {
                    "chunk" => {
                        let chunk = handle.receive().bytes().await;
                        assert!(
                            !chunk.is_empty(),
                            "Bytes.Reader returned an empty chunk; implementation bug"
                        );
                        self.remainder.buffer.extend(chunk.as_ref());
                        // After extending by a non-empty chunk, the current index must be valid
                        let b = *self
                            .remainder
                            .buffer
                            .get(self.index)
                            .expect("buffer should contain data at current index");
                        self.index += 1;
                        return Ok(Some((self.index - 1, b)));
                    }
                    "end" => {
                        // Close the provider side of the 'end' branch
                        self.remainder.handle.take().unwrap().continue_();
                        return Ok(None);
                    }
                    _ => unreachable!(),
                },
                "err" => {
                    // Propagate the opaque error handle upward without receiving
                    let err = self.remainder.handle.take().unwrap();
                    return Err(err);
                }
                _ => unreachable!(),
            }
        }
    }
}

pub(super) struct ReaderRemainderCharIterator<'a> {
    bytes: ReaderRemainderByteIterator<'a>,
    tmp: Vec<u8>,
}

impl<'a> AsyncCharIterator for ReaderRemainderCharIterator<'a> {
    // Forward opaque error values via raw Handle
    type Err = Handle;

    async fn next(&mut self) -> Result<Option<(usize, usize, char)>, Self::Err> {
        loop {
            while self.tmp.len() < 4 {
                match self.bytes.next().await? {
                    Some((_, b)) => self.tmp.push(b),
                    None => break,
                }
            }
            if self.tmp.is_empty() {
                return Ok(None);
            }
            let pos = self.bytes.index - self.tmp.len();
            for len in 1..=self.tmp.len() {
                if let Ok(s) = std::str::from_utf8(&self.tmp[..len]) {
                    if let Some(c) = s.chars().next() {
                        self.tmp.drain(..len);
                        return Ok(Some((pos, len, c)));
                    }
                }
            }
            // Invalid UTF-8 prefix; consume one byte and return replacement char
            self.tmp.remove(0);
            return Ok(Some((pos, 1, char::REPLACEMENT_CHARACTER)));
        }
    }
}

impl BytesRemainder for ReaderRemainder {
    type Err = Handle;
    type Iterator<'a>
        = ReaderRemainderByteIterator<'a>
    where
        Self: 'a;

    async fn provide_err(handle: Handle, err: Self::Err) {
        // Forward the opaque error value
        handle.link(err);
    }

    async fn close(mut self) -> Result<(), Self::Err> {
        let Some(handle) = self.handle.as_mut() else {
            return Ok(());
        };
        handle.signal(literal!("close"));
        match handle.case().await.as_str() {
            "ok" => Ok(self.handle.take().unwrap().continue_()),
            "err" => Err(self.handle.take().unwrap()),
            _ => unreachable!(),
        }
    }

    async fn peek_byte(&mut self) -> Result<Option<u8>, Self::Err> {
        let mut bytes = self.bytes();
        bytes.next().await.map(|item| item.map(|(_, b)| b))
    }

    fn bytes(&mut self) -> Self::Iterator<'_> {
        ReaderRemainderByteIterator {
            remainder: self,
            index: 0,
        }
    }

    fn pop_bytes(&mut self, n: usize) -> Bytes {
        self.buffer.drain(..n).collect()
    }

    async fn remaining_bytes(&mut self) -> Result<Bytes, Self::Err> {
        let mut result = Vec::new();
        let mut iter = self.bytes();
        while let Some((_, b)) = iter.next().await? {
            result.push(b);
        }
        Ok(Bytes::from(result))
    }
}

impl CharsRemainder for ReaderRemainder {
    type Err = Handle;
    type Iterator<'a>
        = ReaderRemainderCharIterator<'a>
    where
        Self: 'a;

    async fn provide_err(handle: Handle, err: Self::Err) {
        // Forward the opaque error value
        handle.link(err);
    }

    async fn close(mut self) -> Result<(), Self::Err> {
        let Some(handle) = self.handle.as_mut() else {
            return Ok(());
        };
        handle.signal(literal!("close"));
        match handle.case().await.as_str() {
            "ok" => Ok(self.handle.take().unwrap().continue_()),
            "err" => Err(self.handle.take().unwrap()),
            _ => unreachable!(),
        }
    }

    async fn peek_byte(&mut self) -> Result<Option<u8>, Self::Err> {
        let mut bytes = ReaderRemainderByteIterator {
            remainder: self,
            index: 0,
        };
        bytes.next().await.map(|item| item.map(|(_, b)| b))
    }

    fn chars(&mut self) -> Self::Iterator<'_> {
        ReaderRemainderCharIterator {
            bytes: ReaderRemainderByteIterator {
                remainder: self,
                index: 0,
            },
            tmp: Vec::with_capacity(4),
        }
    }

    fn pop_chars(&mut self, n: usize) -> ParString {
        let popped = self.buffer.drain(..n).collect::<Vec<u8>>();
        ParString::from_utf8_lossy(popped.into())
    }

    async fn remaining_chars(&mut self) -> Result<ParString, Self::Err> {
        let mut result = String::new();
        let mut iter = self.chars();
        while let Some((_, _, ch)) = iter.next().await? {
            result.push(ch);
        }
        Ok(ParString::from(result))
    }
}

impl BytesRemainder for Bytes {
    type Err = Never;
    type Iterator<'a>
        = (usize, &'a Bytes)
    where
        Self: 'a;

    async fn provide_err(_: Handle, err: Self::Err) {
        match err {}
    }

    async fn close(self) -> Result<(), Self::Err> {
        Ok(())
    }

    async fn peek_byte(&mut self) -> Result<Option<u8>, Self::Err> {
        Ok(self.first().copied())
    }

    fn bytes(&mut self) -> Self::Iterator<'_> {
        (0, self)
    }

    fn pop_bytes(&mut self, n: usize) -> Bytes {
        let popped = self.slice(..n);
        *self = self.slice(n..);
        popped
    }

    async fn remaining_bytes(&mut self) -> Result<Bytes, Self::Err> {
        Ok(self.clone())
    }
}

impl<'a> AsyncByteIterator for (usize, &'a Bytes) {
    type Err = Never;

    async fn next(&mut self) -> Result<Option<(usize, u8)>, Self::Err> {
        let (index, bytes) = self;
        Ok(match bytes.get(*index) {
            Some(&byte) => Some((*index, {
                *index += 1;
                byte
            })),
            None => None,
        })
    }
}

impl CharsRemainder for ParString {
    type Err = Never;
    type Iterator<'a>
        = (usize, &'a ParString)
    where
        Self: 'a;

    async fn provide_err(_: Handle, err: Self::Err) {
        match err {}
    }

    async fn close(self) -> Result<(), Self::Err> {
        Ok(())
    }

    async fn peek_byte(&mut self) -> Result<Option<u8>, Self::Err> {
        Ok(self.as_str().as_bytes().first().copied())
    }

    fn chars(&mut self) -> Self::Iterator<'_> {
        (0, self)
    }

    fn pop_chars(&mut self, n: usize) -> ParString {
        let popped = self.substr(..n);
        *self = self.substr(n..);
        popped
    }

    async fn remaining_chars(&mut self) -> Result<ParString, Self::Err> {
        Ok(ParString::from(self.clone()))
    }
}

impl<'a> AsyncCharIterator for (usize, &'a ParString) {
    type Err = Never;

    async fn next(&mut self) -> Result<Option<(usize, usize, char)>, Self::Err> {
        let (index, chars) = self;
        Ok(match chars.as_str()[*index..].chars().next() {
            Some(ch) => Some((*index, ch.len_utf8(), {
                *index += ch.len_utf8();
                ch
            })),
            None => None,
        })
    }
}

enum ParserState<R, E> {
    Live(R),
    Poison(E),
}

pub(super) async fn provide_bytes_parser<R: BytesRemainder>(mut handle: Handle, remainder: R) {
    let mut state = ParserState::Live(remainder);

    'outer: loop {
        state = match state {
            ParserState::Live(mut remainder) => match remainder.peek_byte().await {
                Ok(Some(_)) => {
                    handle.signal(literal!("ready"));
                    ParserState::Live(remainder)
                }
                Ok(None) => {
                    handle.signal(literal!("empty"));
                    return handle.break_();
                }
                Err(err) => {
                    handle.signal(literal!("ready"));
                    ParserState::Poison(err)
                }
            },
            ParserState::Poison(err) => {
                handle.signal(literal!("ready"));
                ParserState::Poison(err)
            }
        };

        'inner: loop {
            state = match state {
                ParserState::Poison(err) => {
                    match handle.case().await.as_str() {
                        "close" | "#close" | "remainder" | "byte" => {}
                        "minMax" | "minMaxEnd" => {
                            let _ = BytesPattern::readback(handle.receive()).await;
                            let _ = BytesPattern::readback(handle.receive()).await;
                        }
                        _ => unreachable!(),
                    }
                    handle.signal(literal!("err"));
                    return R::provide_err(handle, err).await;
                }

                ParserState::Live(mut remainder) => match handle.case().await.as_str() {
                    "close" | "#close" => match remainder.close().await {
                        Ok(()) => {
                            handle.signal(literal!("ok"));
                            return handle.break_();
                        }
                        Err(err) => {
                            handle.signal(literal!("err"));
                            return R::provide_err(handle, err).await;
                        }
                    },

                    "byte" => {
                        let mut bytes = remainder.bytes();
                        match bytes.next().await {
                            Ok(Some((_, b))) => {
                                drop(bytes);
                                handle.signal(literal!("ok"));
                                handle.send().provide_byte(b);
                                remainder.pop_bytes(1);
                                ParserState::Live(remainder)
                            }
                            Ok(None) => unreachable!("parser attempt should be non-empty"),
                            Err(err) => {
                                handle.signal(literal!("err"));
                                return R::provide_err(handle, err).await;
                            }
                        }
                    }

                    "minMax" => {
                        let prefix = BytesPattern::readback(handle.receive()).await;
                        let suffix = BytesPattern::readback(handle.receive()).await;
                        let mut m =
                            BytesMachine::start(Box::new(BytesPattern::Concat(prefix, suffix)));

                        let mut best_match = None;
                        let mut bytes = remainder.bytes();
                        loop {
                            let (pos, b) = match bytes.next().await {
                                Ok(Some((pos, b))) => (pos, b),
                                Ok(None) => break,
                                Err(err) => {
                                    handle.signal(literal!("err"));
                                    return R::provide_err(handle, err).await;
                                }
                            };
                            match (m.leftmost_feasible_split(pos), best_match) {
                                (Some(fi), Some((bi, _))) if fi > bi => break,
                                (None, _) => break,
                                _ => {}
                            }
                            m.advance(pos, b);
                            match (m.leftmost_accepting_split(), best_match) {
                                (Some(ai), Some((bi, _))) if ai <= bi => {
                                    best_match = Some((ai, pos + 1))
                                }
                                (Some(ai), None) => best_match = Some((ai, pos + 1)),
                                _ => {}
                            }
                        }
                        drop(bytes);

                        match best_match {
                            Some((i, j)) => {
                                handle.signal(literal!("ok"));
                                handle.signal(literal!("match"));
                                handle.send().provide_bytes(remainder.pop_bytes(i));
                                handle.send().provide_bytes(remainder.pop_bytes(j - i));
                                ParserState::Live(remainder)
                            }
                            None => {
                                handle.signal(literal!("ok"));
                                handle.signal(literal!("fail"));
                                state = ParserState::Live(remainder);
                                continue 'inner;
                            }
                        }
                    }

                    "minMaxEnd" => {
                        let prefix = BytesPattern::readback(handle.receive()).await;
                        let suffix = BytesPattern::readback(handle.receive()).await;
                        let mut m =
                            BytesMachine::start(Box::new(BytesPattern::Concat(prefix, suffix)));

                        let mut bytes = remainder.bytes();
                        loop {
                            let (pos, b) = match bytes.next().await {
                                Ok(Some((pos, b))) => (pos, b),
                                Ok(None) => break,
                                Err(err) => {
                                    handle.signal(literal!("err"));
                                    return R::provide_err(handle, err).await;
                                }
                            };
                            if m.accepts() == None {
                                break;
                            }
                            m.advance(pos, b);
                        }
                        drop(bytes);

                        match m.leftmost_accepting_split() {
                            Some(i) => {
                                let left = remainder.pop_bytes(i);
                                let right = match remainder.remaining_bytes().await {
                                    Ok(bytes) => bytes,
                                    Err(err) => {
                                        handle.signal(literal!("err"));
                                        return R::provide_err(handle, err).await;
                                    }
                                };
                                handle.signal(literal!("ok"));
                                handle.signal(literal!("match"));
                                handle.send().provide_bytes(left);
                                handle.send().provide_bytes(right);
                                return handle.break_();
                            }
                            None => {
                                handle.signal(literal!("ok"));
                                handle.signal(literal!("fail"));
                                state = ParserState::Live(remainder);
                                continue 'inner;
                            }
                        }
                    }

                    "remainder" => match remainder.remaining_bytes().await {
                        Ok(bytes) => {
                            handle.signal(literal!("ok"));
                            return handle.provide_bytes(bytes);
                        }
                        Err(err) => {
                            handle.signal(literal!("err"));
                            return R::provide_err(handle, err).await;
                        }
                    },

                    _ => unreachable!(),
                },
            };

            continue 'outer;
        }
    }
}

pub(super) async fn provide_string_parser<R: CharsRemainder>(mut handle: Handle, remainder: R) {
    let mut state = ParserState::Live(remainder);

    'outer: loop {
        state = match state {
            ParserState::Live(mut remainder) => match remainder.peek_byte().await {
                Ok(Some(_)) => {
                    handle.signal(literal!("ready"));
                    ParserState::Live(remainder)
                }
                Ok(None) => {
                    handle.signal(literal!("empty"));
                    return handle.break_();
                }
                Err(err) => {
                    handle.signal(literal!("ready"));
                    ParserState::Poison(err)
                }
            },
            ParserState::Poison(err) => {
                handle.signal(literal!("ready"));
                ParserState::Poison(err)
            }
        };

        'inner: loop {
            state = match state {
                ParserState::Poison(err) => {
                    match handle.case().await.as_str() {
                        "close" | "#close" | "remainder" | "char" => {}
                        "minMax" | "minMaxEnd" => {
                            let _ = StringPattern::readback(handle.receive()).await;
                            let _ = StringPattern::readback(handle.receive()).await;
                        }
                        _ => unreachable!(),
                    }
                    handle.signal(literal!("err"));
                    return R::provide_err(handle, err).await;
                }

                ParserState::Live(mut remainder) => match handle.case().await.as_str() {
                    "close" | "#close" => match remainder.close().await {
                        Ok(()) => {
                            handle.signal(literal!("ok"));
                            return handle.break_();
                        }
                        Err(err) => {
                            handle.signal(literal!("err"));
                            return R::provide_err(handle, err).await;
                        }
                    },

                    "char" => {
                        let mut chars = remainder.chars();
                        match chars.next().await {
                            Ok(Some((_, len, ch))) => {
                                drop(chars);
                                handle.signal(literal!("ok"));
                                handle.send().provide_char(ch);
                                remainder.pop_chars(len);
                                ParserState::Live(remainder)
                            }
                            Ok(None) => unreachable!("parser attempt should be non-empty"),
                            Err(err) => {
                                handle.signal(literal!("err"));
                                return R::provide_err(handle, err).await;
                            }
                        }
                    }

                    "minMax" => {
                        let prefix = StringPattern::readback(handle.receive()).await;
                        let suffix = StringPattern::readback(handle.receive()).await;
                        let mut m =
                            StringMachine::start(Box::new(StringPattern::Concat(prefix, suffix)));

                        let mut best_match = None;
                        let mut chars = remainder.chars();
                        loop {
                            let (pos, len, ch) = match chars.next().await {
                                Ok(Some((pos, len, ch))) => (pos, len, ch),
                                Ok(None) => break,
                                Err(err) => {
                                    handle.signal(literal!("err"));
                                    return R::provide_err(handle, err).await;
                                }
                            };
                            match (m.leftmost_feasible_split(pos), best_match) {
                                (Some(fi), Some((bi, _))) if fi > bi => break,
                                (None, _) => break,
                                _ => {}
                            }
                            m.advance(pos, len, ch);
                            match (m.leftmost_accepting_split(), best_match) {
                                (Some(ai), Some((bi, _))) if ai <= bi => {
                                    best_match = Some((ai, pos + len))
                                }
                                (Some(ai), None) => best_match = Some((ai, pos + len)),
                                _ => {}
                            }
                        }
                        drop(chars);

                        match best_match {
                            Some((i, j)) => {
                                handle.signal(literal!("ok"));
                                handle.signal(literal!("match"));
                                handle.send().provide_string(remainder.pop_chars(i));
                                handle.send().provide_string(remainder.pop_chars(j - i));
                                ParserState::Live(remainder)
                            }
                            None => {
                                handle.signal(literal!("ok"));
                                handle.signal(literal!("fail"));
                                state = ParserState::Live(remainder);
                                continue 'inner;
                            }
                        }
                    }

                    "minMaxEnd" => {
                        let prefix = StringPattern::readback(handle.receive()).await;
                        let suffix = StringPattern::readback(handle.receive()).await;
                        let mut m =
                            StringMachine::start(Box::new(StringPattern::Concat(prefix, suffix)));

                        let mut chars = remainder.chars();
                        loop {
                            let (pos, len, ch) = match chars.next().await {
                                Ok(Some((pos, len, ch))) => (pos, len, ch),
                                Ok(None) => break,
                                Err(err) => {
                                    handle.signal(literal!("err"));
                                    return R::provide_err(handle, err).await;
                                }
                            };
                            if m.accepts() == None {
                                break;
                            }
                            m.advance(pos, len, ch);
                        }
                        drop(chars);

                        match m.leftmost_accepting_split() {
                            Some(i) => {
                                let left = remainder.pop_chars(i);
                                let right = match remainder.remaining_chars().await {
                                    Ok(string) => string,
                                    Err(err) => {
                                        handle.signal(literal!("err"));
                                        return R::provide_err(handle, err).await;
                                    }
                                };
                                handle.signal(literal!("ok"));
                                handle.signal(literal!("match"));
                                handle.send().provide_string(left);
                                handle.send().provide_string(right);
                                return handle.break_();
                            }
                            None => {
                                handle.signal(literal!("ok"));
                                handle.signal(literal!("fail"));
                                state = ParserState::Live(remainder);
                                continue 'inner;
                            }
                        }
                    }

                    "remainder" => match remainder.remaining_chars().await {
                        Ok(string) => {
                            handle.signal(literal!("ok"));
                            return handle.provide_string(string);
                        }
                        Err(err) => {
                            handle.signal(literal!("err"));
                            return R::provide_err(handle, err).await;
                        }
                    },

                    _ => unreachable!(),
                },
            };

            continue 'outer;
        }
    }
}
