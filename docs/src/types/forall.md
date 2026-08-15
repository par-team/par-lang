# Forall

What about _generic_ functions? Or generic values?

We already know about [generic types](../structure/definitions_and_declarations.md). For example, here's
a typical optional type, as present in many languages and also supported by Par as built-in type:

```par
type Option<a> = either {
  .none!,
  .some a,
}
```

In Par, generic type definitions use the familiar angle bracket syntax. The parameters to those,
such as `a` for `Option<a>` may be replaced with anything, such as `Option<Int>`. The resulting type is,
however, always concrete.

Now consider these two definitions:

```par
module Main

import {
  @core/Int
  @core/String
  @core/Option
}

def None: Option<String> = .none!

dec Swap : [(String, Int)!] (Int, String)!
def Swap = [pair]
  let (first, second)! = pair
  in (second, first)!
```

Both are defined in terms of concrete types, but don't use them: `.none!` is a valid value for
any `Option<a>`, and swapping a pair works regardless of its content.

To make these work with anything, we employ _forall_ types!

In Par, **_forall_ types:**
- **Don't use angle brackets.** Instead they are functions taking types.
- **Are not inferred.** Calling a generic function requires specifying the types.
- **Are first-class!** It's possible to store and pass generic values around, without them losing
  their genericity.

If you want generics inferred at the use site, see [Implicit generics](./implicit_generics.md).

A forall type **consists of two parts:**
- A lower-case type variable enclosed in square brackets, and prefixed with the keyword `type`.
- The result type, which uses this type variable.

```par
dec None : [type a] Option<a>

dec Swap : [type a, type b, (a, b)!] (b, a)!
```

After erasing the previous concrete definitions for `None`, these will be their generic types. As we can
see, these look just like functions, but taking types!

If a forall continues into more explicit type binders or value arguments — like with `Swap` — we keep
them together in one pair of square brackets, repeating `type` for each explicit type binder:

```par
dec Swap : [type a, type b, (a, b)!] (b, a)!
```

This keeps the whole input list visible at once.

Explicit type binders can also carry constraints, such as `[type a: data]`. Those are covered in
[Type Constraints](./constraints.md).

Unlike functions, foralls are not always strictly linear. A forall type can be `drop` or `share` if its body is `drop` or `share`.

## Construction

Values of forall types are constructed the same way as functions, except the argument is a type
variable and prefixed with the keyword `type`.

Completing the definitions above:

```par
dec None : [type a] Option<a>
def None = [type a] .none!

dec Swap : [type a, type b, (a, b)!] (b, a)!
def Swap = [type a, type b, pair]
  let (first, second)! = pair
  in (second, first)!
```

> A common complaint at this point is:
> **Why do I have to write `type a` and `type b` in both the declaration, and the definition?** After all,
> it doesn't seem like they're used in the definition. However, they are! What's the type of `first`?
> It's `a`. And `second`? It's `b`. If you called them `type kek` and `type dek`, they would be `kek` and `dek`.
> Par's type checker never makes type names up.
>
> Additionally, if you do end up needing to use those type variables — for example, to call another
> generic function — they will be right at hand.

## Destruction

Using a _forall_ value looks the same as calling a function, except the argument is a concrete type,
prefixed with the keyword `type`.

```par
def NoneInt = None(type Int)  // type inferred as `Option<Int>`

def Pair    = ("Hello!", 42)!
def Swapped = Swap(type String, type Int, Pair)
//          = (42, "Hello!")!
```
