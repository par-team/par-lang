# Type Constraints

Generic code often needs to know a little bit about an unknown type.

For example, this function can return its argument without knowing anything about `a`:

```par
dec Identity : [<a> a] a
def Identity = [<a> x] x
```

But this one needs to use its value twice:

```par
dec Duplicate : [<a: box> a] (a, a)!
def Duplicate = [<a: box> x] (x, x)!
```

The `: box` part is a **type constraint**. It says that the unknown type `a` must be non-linear,
so values of that type may be reused or dropped.

Par currently has four constraints:

- `box`
- `data`
- `number`
- `signed`

They form a chain from broadest to narrowest:

```text
signed -> number -> data -> box
```

So every `signed` type is also a `number`, every `number` is also `data`, and every `data` type is
also `box`.

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
type SomeData = (type a: data) a
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
type Boxed<a> = box a      // OK
type Bad<a: box> = box a   // Error
```

If a type definition needs constrained behavior, put the constraint on the functions that operate
on that type.

## The `box` Constraint

The `box` constraint means values are non-linear. A value whose type is known only as `a: box` may
be copied, reused, or dropped.

```par
dec KeepFirst : [<a: box> (a, a)!] a
def KeepFirst = [<a: box> (first, second)!] first
```

In the example, `second` is not used. That is allowed because `a: box`.

Types that satisfy `box` include:

- primitives
- `!`
- pairs, eithers, and recursive types whose parts also satisfy `box`
- explicit `box T`
- type aliases that expand to a `box` type
- type variables with a `box`, `data`, `number`, or `signed` constraint

Functions, choices, continuations, and unrestricted type variables do not satisfy `box` unless they
are wrapped in an explicit `box`.

## The `data` Constraint

The `data` constraint means values are ordinary data. Data values are non-linear, and additionally
support:

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

- all primitive types
- `!`
- pairs whose elements are data
- eithers whose payloads are data
- recursive types whose bodies are data
- type aliases that expand to data
- type variables with a `data`, `number`, or `signed` constraint

Adding `box` to a non-data type does not make it data:

```par
box [Int] Int  // box, but not data
```

If `T` is already data, then `box T` can still be used as data, because boxed data can be used as
the data value inside.

## The `number` Constraint

The `number` constraint is for generic numeric code. A `number` type supports:

- `+`
- `*`
- `/`
- `Number.Zero(type a)`

```par
module Main

import @core/Number

dec SumPair : [<a: number> (a) a] a
def SumPair = [<a: number> (left) right] left + right

dec Zero : [type a: number] a
def Zero = [type a: number] Number.Zero(type a)
```

The number types are:

- `Nat`
- `Int`
- `Float`

Type aliases that expand to one of these types, and type variables constrained with `number` or
`signed`, also satisfy `number`.

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

Type aliases that expand to `Int` or `Float`, and type variables constrained with `signed`, also
satisfy `signed`.

`Nat` is intentionally not signed.

## Choosing a Constraint

Use the weakest constraint that gives the function what it needs:

- Use `box` when you only need to copy, reuse, or drop values.
- Use `data` when you need comparison or `#{...}` display.
- Use `number` when you need generic zero, addition, multiplication, or division.
- Use `signed` when you also need subtraction or negation.

This keeps generic APIs as flexible as possible while still making their capabilities clear.
