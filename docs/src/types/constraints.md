# Type Constraints

Generic code often needs to know a little bit about an unknown type.

For example, this function can return its argument without knowing anything about `a`:

```par
dec Identity : [<a> a] a
def Identity = [<a> x] x
```

But this one needs permission to leave one value unused:

```par
dec KeepFirst : [<a: drop> (a, a)!] a
def KeepFirst = [<a: drop> (first, second)!] first
```

The `: drop` part is a **type constraint.** It says that the unknown type `a` has a safe way to be
disposed of, so `second` can be cleaned up automatically.

Par provides five type constraints:

- `drop`
- `share`
- `data`
- `number`
- `signed`

They form a chain from narrowest to broadest:

```text
signed -> number -> data -> share -> drop
```

Every `signed` type is also a `number`; every `number` is also `data`; every `data` type is also
`share`; and every `share` type is also `drop`.

The farther right we go, the less generic code may assume. A `drop` value can be disposed of, but
not necessarily copied, compared, displayed, or added.

## Syntax

Constraints are written after type parameters with a colon.

Explicit generic functions use `type` binders:

```par
dec ZeroOr : [type a: number, Bool, a] a
```

Implicit generic functions use angle-bracket binders:

```par
dec Sum : [<a: number> (a) a] a
```

Existential types can constrain the hidden type:

```par
type SomeDroppable = (type a: drop) a
```

Implicit generic pairs can do the same:

```par
type DataWithText = (<a: data> a) String
```

When you construct a value with a constrained explicit binder, the checked type must satisfy the
same constraint:

```par
dec ShowTwice : [type a: data, a] String
def ShowTwice = [type a: data, x] `#{x} #{x}`
```

Named type definitions are the one place where parameters are not constrained:

```par
type Boxed<a> = box a        // OK
type Bad<a: share> = box a   // Error
```

If a type definition needs constrained behavior, put the constraint on the functions that operate
on that type.

## The `drop` Constraint

The `drop` constraint means a value may be left unused. For a shareable value this requires no
action. For a linear value, Par performs its [structural cleanup](./auto_cleanup.md).

```par
dec KeepFirst : [<a: drop> (a, a)!] a
def KeepFirst = [<a: drop> (first, second)!] first
```

`KeepFirst` never needs another copy of either value, so `drop` is exactly the capability it needs.
This works for ordinary data as well as cleanup-capable resources.

The standard `List.Length` has the same shape of requirement:

```par
dec List.Length : [<a: drop> List<a>] Nat
```

It walks through the list and counts its nodes without keeping their elements. A strict linear
element type would make that impossible; a droppable one is sufficient.

Types that satisfy `drop` include:

- primitives, `!`, and every `share` type
- an explicit `box T`
- pairs and eithers whose parts all satisfy `drop`
- recursive types whose bodies satisfy `drop`, where `self` is assumed `drop`
- iterative types whose bodies satisfy `drop`, but where `self` itself is not assumed `drop`
- choices with a cleanup branch whose result satisfies `drop`
- generic types whose bodies satisfy `drop`
- type variables constrained by `drop` or any narrower constraint

Functions, continuations, choices without a usable cleanup branch, or unrestricted type variables
do not satisfy `drop`.

## The `share` Constraint

The `share` constraint means values may be copied, reused, or dropped.

```par
dec Duplicate : [<a: share> a] (a, a)!
def Duplicate = [<a: share> x] (x, x)!
```

`drop` would not be enough here. Constructing the pair uses `x` twice, so `Duplicate` needs
`share`.

The distinction can be subtler when the two uses appear on different paths. Consider `List.Filter`:

```par
dec Filter : [<a: share> List<a>, box [a] Bool] List<a>
def Filter = [<a: share> list, predicate] list.begin.case {
  .end! => .end!,
  .item(x) xs => predicate(x).case {
    .true! => .item(x) xs.loop,
    .false! => xs.loop,
  }
}
```

The predicate consumes one use of `x`. If it returns `.true!`, the output list needs another use.
That is copying, so the correct constraint is `share`, even though the `.false!` path merely drops
the item.

Types that satisfy `share` include:

- primitives and `!`
- pairs, eithers, recursive and iterative types whose parts satisfy `share`
- every `box T`, regardless of `T`
- explicit and implicit generic types whose bodies satisfy `share`
- type variables constrained by `share`, `data`, `number`, or `signed`

Functions, choices, continuations, and unrestricted type variables do not satisfy `share`
unless they are wrapped in a `box`.

## The `data` Constraint

The `data` constraint means values are ordinary comparable and displayable data. Data values are
shareable, and additionally support:

- comparison operators: `<`, `>`, `<=`, `>=`, `==`, `!=`
- data interpolation in template strings: `#{...}`

```par
dec Min : [<a: data> (a) a] a
def Min = [<a: data> (left) right] if {
  left <= right => left,
  else => right,
}

dec Label : [<a: data> a] String
def Label = [<a: data> value] `value = #{value}`
```

The comparison operators use `@core/Data.Compare` under the hood. The `#{...}` template form uses
`@core/Data.ToString`.

Types that satisfy `data` include:

- all primitive types and `!`
- pairs whose elements are data
- eithers whose payloads are data
- recursive types whose bodies are data
- type variables with a `data`, `number`, or `signed` constraint

A box is shareable, but does not satisfy `data`, even if its contents do:

```par
box [Int] Int  // share, but not data
box Int        // also share, but not data
```

A box holds a suspended computation. Use [`.unbox`](./box.md#destruction) to instantiate it
and obtain the value inside before using data operations. Boxes do not satisfy `number` or
`signed` either.

## The `number` Constraint

The `number` constraint is for generic numeric code. A `number` type supports:

- `+`
- `*`
- `/`
- `Number.Zero(type a)`

```par
module Main

import {
  @core/List
  @core/Number
}

dec Sum : [<a: number> List<a>] a
def Sum = [<a: number> list] list.begin.case {
  .end! => Number.Zero(type a),
  .item(x) xs => x + xs.loop,
}
```

The number types are:

- `Nat`
- `Int`
- `Float`

`number` does not provide subtraction or negation, because `Nat` is a number but not signed.

## The `signed` Constraint

The `signed` constraint is the numeric constraint for types that support negative values. It has
everything from `number`, plus:

- `-`
- `neg`

```par
dec Difference : [<a: signed> (a) a] a
def Difference = [<a: signed> (left) right] left - right

dec Negate : [<a: signed> a] a
def Negate = [<a: signed> value] neg value
```

The signed types are:

- `Int`
- `Float`

`Nat` is intentionally not signed.

## Choosing a Constraint

When accepting a generic argument, use the weakest constraint that gives the function what it needs:

- Use `drop` when you only need to leave values unused.
- Use `share` when you need to copy or reuse values.
- Use `data` when you need comparison or `#{...}` display.
- Use `number` when you need generic zero, addition, multiplication, or division.
- Use `signed` when you also need subtraction or negation.

On the other hand, when constructing an existential value, use the strongest constraint.