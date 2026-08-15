# Exists

An _exists type_ lets a value **hide a type inside itself** — while exposing only what can be done
with that type.

They’re the dual of the parametric types: [_forall_](./forall.md). A forall type `[type a] ...` says:
_"You can give me any type `a`, and I’ll work with it."_ On the other hand, an _exists type_ `(type a) ...`
says: _"I have chosen some specific type `a`, but I’m not going to tell you what it is."_

An exists type **consists of two parts:**
- A lowercase type variable enclosed in round parentheses, prefixed with the keyword `type`.
- The payload type — a type that may use the hidden type variable.

It’s similar to a [pair](./pair.md), but the first component is a type instead of a value.
The hidden type can also carry a constraint, such as `(type a: data)`, as covered in
[Type Constraints](./constraints.md).

Here are two simple examples of existential types:

```par
type Any = (type a) a

type DropMe = (type a: drop) a
```

The first one is completely opaque — a value of `Any` gives you a value of the hidden type, but no
operations to perform on it. That makes it useless, but it's the simplest example of an exists type.

The second one reveals one capability of the hidden type: its values may be dropped. That is enough
for Par to clean up the hidden value without revealing what it is.

Let’s now see how existential types are used.

## Construction

To construct an existential value, you must pick a concrete type and provide a matching payload.

The syntax is the same as for [pairs](./pair.md), with the addition of the `type` keyword.

Here’s a value of type `Any`:

```par
def Hidden: Any = (type Int) 42
```

But since the type is hidden inside `Any`, with no operations or constraints provided, this value is
completely useless — we can’t do anything with it. In fact, we can't even get rid of it if we were to
instantiate it into a variable.

Let’s now look at a slightly more interesting example:

```par
type DropMe = (type a: drop) a
```

The payload is a value of the hidden type. The constraint records that the value has a safe structural
cleanup, whether that means doing nothing for ordinary data or following a cleanup protocol.

Here’s how we can construct a value of `DropMe`:

```par
def Drop42: DropMe = (type Int) 42
```

## Destruction

To use an existential value, you must **unpack it**. The syntax is the same as unpacking a pair,
just with the type keyword.

Here’s an example that unpacks and uses a `DropMe`:

```par
def UseDrop: ! =
  let (type a: drop) x = Drop42
  in !
```
The pattern `(type a: drop) x` means:
- `a` becomes the name of the hidden type, a local type variable.
- `x` is the stored value of type `a`.

Because `x` is unused, Par cleans it up automatically.

We can also unpack existentials **in function parameters,** using patterns. Here’s a function that takes
a `DropMe` and drops its inner value:

```par
dec DropIt : [DropMe] !
def DropIt = [(type a: drop) x] !
```

## A Real Example

_Exists_ types become truly useful when combined with [box](./box.md) types — allowing you to hide
implementation details inside interfaces that can be passed around freely.

Here’s a boxed interface for working with sets:

```par
type SetModule<a> = (type set: share) box choice {
  .empty => set,
  .insert(a, set) => set,
  .contains(a, set) => Bool,
}
```

This type hides the implementation type of the set.
The interface is boxed, so it can be copied and discarded.
The hidden `set` type is constrained with [`share`](./constraints.md), so set values can be reused by
the operations that inspect them.
The hidden type `set` is never revealed — only its operations are exposed.

Let’s now implement an inefficient, but simple `SetModule` using lists. This version works for
[`data`](./constraints.md) elements, because it uses `==` to check whether an item is already in
the set:

```par
module Main

import {
  @core/Bool
  @core/Int
  @core/List
}

dec ListSet : [type a: data] SetModule<a>
def ListSet = [type a: data] (type List<a>) box case {
  .empty => .end!,

  .insert(x, set) => .item(x) set,

  .contains(y, set) => set.begin.case {
    .end! => .false!,
    .item(x) xs => {x == y}.case {
      .true! => .true!,
      .false! => xs.loop,
    },
  },
}
```

This implementation of sets can only be constructed for comparable data types. That's what
`a: data` ensures here.

We construct the existential by choosing `List<a>` as the hidden type:

```par
(type List<a>) ...
```

The consumer of the `SetModule` doesn’t know that these sets are implemented as lists.

Let’s now use that in a function:

```par
dec Deduplicate : [type a: data, SetModule<a>, List<a>] List<a>
def Deduplicate = [type a: data, (type set: share) mSet, list]
  let visited = mSet.empty
  in list.begin.case {
    .end! => .end!,
    .item(x) xs => mSet.contains(x, visited).case {
      .true! => xs.loop,
      .false! =>
        let visited = mSet.insert(x, visited)
        in .item(x) xs.loop,
    }
  }
```

This function deduplicates a list by tracking seen values using a hidden set implementation.
It doesn’t know how the set works — just that it supports `.empty`, `.insert`, and `.contains`.

Let's test it!

```par
def IntListSet = ListSet(type Int)

def TestDedup =
  Deduplicate(
    type Int,
    IntListSet,
    List.Map(Int.Range(1, 1000), box [n] Int.Mod(n, 7)),
  )
```

This deduplicates a list of numbers modulo 7 — using:
- `List.Map` to apply the modulo.
- `ListSet` to get an abstract set implementation.
- `Deduplicate` to do the filtering.

All without exposing how the set is represented.
