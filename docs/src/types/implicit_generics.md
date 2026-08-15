# Implicit generics

Par already supports explicit generics via [`forall`](./forall.md):

```par
module Main

import {
  @core/Int
  @core/String
}

dec Swap : [type a, type b, (a, b)!] (b, a)!
def Swap = [type a, type b, pair]
  let (first, second)! = pair
  in (second, first)!
```

With `forall`, callers pass types explicitly when they use `Swap`:

```par
def Pair = ("Hello!", 42)!
def Swapped = Swap(type String, type Int, Pair)
```

That is precise and fully explicit. But in many everyday cases, the types are
obvious from the value you are passing. For those cases, Par also has **implicit
generics**.

## Syntax

Implicit generics are not a standalone type. The `<a, ...>` binder is part of
one value item inside a **[function](./function.md) type**. It appears immediately
before the argument type whose value will determine those type variables.

```par
[<a, b> Arg, Other] Res
```

An item may have no binder, one binder, or a binder with several variables. This
means implicit and ordinary arguments can share one pair of brackets:

```par
[A, <b> B, C] Res
```

You cannot use `<a>` outside a function or pair item list, or put it in front of
`either`, `choice`, `box`, or another complete type.

The key rule is local inference:

**Implicit type variables are inferred only from the single argument
immediately following the `<...>` binder.**

The binder does not infer from the other items in the same brackets. For
example, this function infers `a` from `left` only; `right` is checked afterward
using the inferred `a`:

```par
dec Concat : [<a> List<a>, List<a>] List<a>
```

Different items may introduce different variables:

```par
dec Map : [<a> List<a>, <b> box [a] b] List<b>
```

Callers pass the value arguments normally:

```par
Concat(left, right)
```

The surface list still represents sequential, nested function types. Attaching
the binder to an item records exactly which one argument drives each local
inference step.

One more important point: there is no syntax for “manually specifying” implicit
type arguments. When designing an API you choose, for each type variable,
whether it is implicit or explicit:

- If it is implicit (`<a>`), it is always inferred.
- If it is explicit (`[type a]`), it is always specified by the caller.

This is intentional: it keeps call sites predictable.

Implicit type parameters can also carry constraints, such as `<a: share>` or
`<a: data>`. The constraints themselves are covered in [Type Constraints](./constraints.md).

## Construction

Start with the `Swap` example, but make it implicit:

```par
dec Swap : [<a, b> (a, b)!] (b, a)!
```

To construct a value of this type, you also introduce the implicit type names
at the term level, and then receive the value argument:

```par
dec Swap : [<a, b> (a, b)!] (b, a)!
def Swap = [<a, b> pair]
  let (first, second)! = pair
  in (second, first)!
```

This is intentionally different from `forall`: an implicit generic and an
explicit `forall` generic are different types, and they do not subtype into one
another.

You can also use implicit generics when the type variable is nested inside a
larger argument:

```par
dec Flatten : [<a> List<List<a>>] List<a>
```

## Destruction

To use an implicit generic, you call it like an ordinary function:

```par
def Swapped = Swap(Pair)
```

Here `a = String` and `b = Int` are inferred from the type of `Pair` (the
argument immediately following `<a, b>`).

If you want to steer inference, you can still add annotations locally, for
example by ascribing a type to an expression:

```par
type T in expr
```

For example (this is a bit contrived, but it shows the point):

```par
Concat(type List<Nat> in *(), numbers)
```

If you wrote `Concat(*(), numbers)`, the empty list `*()` would lead `a` to be
inferred as `either {}` (an impossible type), so inference would not pick `Nat`
for you.

## Higher-order arguments

Implicit generics infer their type variables from the *argument value*. This
works great when that argument already has a clear type, and Par can also use
the expected argument type to check expressions whose type is only partially
known.

A classic example is an anonymous function. The `box` here is not the point —
it just happens that `Map` takes a boxed function.

```par
dec Map : [<a> List<a>, <b> box [a] b] List<b>
```

The list argument infers `a`. When checking the mapper, `b` is still unknown,
but the checker can still use the partial expected type `box [a] b`. That means
the lambda parameter gets type `a`, while constraints from the lambda body solve
`b`:

```par
Map(numbers, box [x] `#{x}`)
```

Here `x` is checked using the element type of `numbers`, and the string
interpolation result infers `b = String`.

You may still need a local annotation when the argument does not constrain the
type enough. Empty structures are the usual example:

```par
Concat(type List<Nat> in *(), numbers)
```

The annotation is on the value being passed, not on the implicit type parameter.
There is still no syntax for manually passing an implicit type argument.

(`Map` is discussed in [Box](./box.md).)

## Implicit generics and pairs

Implicit generics also exist for [pair](./pair.md) types. This is the implicit
counterpart of [`exists`](./exists.md).

Just like with functions, the `<a, ...>` binder belongs to one value item, this
time inside the pair's parentheses. Ordinary and implicit items can be mixed,
as in `(Header, <a> a) Rest`.

Here is a simple example. `AnyDroppable` stores a value of some unknown type,
while remembering that the hidden type satisfies [`drop`](./constraints.md#the-drop-constraint):

```par
type AnyDroppable = (<a: drop> a)!
```

This is the implicit counterpart of the existential type:

```par
type DropMe = (type a: drop) a
```

### Construction

Notice that you do not specify the hidden type when constructing an `AnyDroppable`:

```par
def Example: AnyDroppable = (7)!
```

### Destruction

Dually, when you unpack an implicit-generic pair, you introduce a local type
variable:

```par
let (<a: drop> value)! = Example
```

This is the “mirror image” of implicit-generic functions: there you introduce
`<a, ...>` at construction time and inference happens at call time; here you
construct without naming `a`, and introduce `a` when unpacking.
