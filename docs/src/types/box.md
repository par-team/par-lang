# Box

Par has a **linear type system.** By default, values must be used **exactly once.**

But not all values need that kind of discipline. Sometimes, you want to:
- Pass a function around multiple times.
- Discard an unused value.
- Compose higher-order utilities freely.

That’s where **box types** come in.

## Non-linear types — even without `box`

Even without box types, some types in Par are already non-linear. These are the **data types:**
- [Unit](./unit.md)
- [Either](./either.md)
- [Pair](./pair.md)
- [Recursive](./recursive.md)
- All the [primitives](../structure/primitive_types.md): `Int`, `Nat`, `Float`, `String`, `Char`, `Byte`, `Bytes`.

Any combination of these is always non-linear — they can be copied and discarded freely.

But types that contain [**functions,**](./function.md) [**choices,**](./choice.md) and other non-data types
are linear — no matter how deeply nested.

Now, consider this: what if you want to **apply a function to each element in a list?**

A plain function in Par is linear — it can only be used once. So applying it repeatedly requires a workaround.

## Reusable functions, the hard way

Without `box`, we can build reusable functions by encoding a usage protocol manually:

```par
type Mapper<a, b> = iterative choice {
  .close => !,
  .apply(a) => (b) self,
}
```

This protocol gives us:
- `.apply` to use the function.
- `.close` to clean up.

Here's a `Map` function that uses it:

```par
dec Map : [type a, type b, List<a>, Mapper<a, b>] List<b>
def Map = [type a, type b, list, mapper] list.begin.case {
  .end! => let ! = mapper.close in .end!,
  .item(x) xs => let (x1) mapper = mapper.apply(x) in .item(x1) xs.loop,
}
```

And using it:

```par
def NumberStrings = Map(type Int, type String, Int.Range(1, 100), begin case {
  .close => !,
  .apply(n) => (`#{n}`) loop,
})
```

This works — but it’s verbose.

Every reusable function needs to be manually encoded with a protocol like `Mapper`.
Copying, closing, and chaining all become manual work.

## Box types to the rescue

Instead of encoding reusability into the type manually, Par lets you `box` a value.

A `box T` is a non-linear version of **any** type `T`. You can:

- **Copy** a `box T`.
- **Drop** a `box T`.
- Pass it around freely.

You can construct boxed values using:

```par
box <expression>
```

This constructs a value of type `box T`, where `T` is the type of the expression.

The only rule is: **You can only capture non-linear variables in a `box` expression.**

That includes:
- Data types (`Int`, `String`, `List<Int>`, etc.)
- Other `box` values.

The word _**capture**_ here refers to _using local variables_ inside the expression that were
_created outside of that expression._

## A better `Map`

With `box`, we can rewrite the `Map` function much more cleanly:

```par
module Main

import @core/List

dec Map : [<a> List<a>, <b> box [a] b] List<b>
def Map = [<a> list, <b> f] list.begin.case {
  .end! => .end!,
  .item(x) xs => .item(f(x)) xs.loop,
}
```

Let’s try it out:

```par
def NumberStrings = Map(Int.Range(1, 100), box [n] `#{n}`)
```

No wrappers, no manual protocols. The boxed function can be used freely, because the `box` type makes
it non-linear. This is exactly what `box` was made for.

## Subtyping

Boxed types fit naturally into Par's subtyping.

**A `box T` can be used anywhere a `T` is expected.**

```par
def BoxInt: box Int = 42       // OK: Int is non-linear
def UseInt: Int = BoxInt       // OK: box Int can be used as Int
```

And **if `T` is already non-linear, then `T` can be used anywhere a `box T` is expected.**

```par
def Boxes: List<box Int> = *(1, 2, 3)
def Ints: List<Int> = Boxes
```

**For non-linear types, `T` and `box T` are effectively interchangeable.**

## Another example: Filtering a list

Let’s write a function that filters a list using a boxed predicate.

This example uses a `box` type constraint, written `a: box`. The next chapter covers constraints
properly; for now, read it as saying: "the element type must be non-linear, because rejected
elements are discarded."

```par
module Main

import {
  @core/Bool
  @core/Int
  @core/List
}

dec Filter : [<a: box> List<a>, box [a] Bool] List<a>

def Filter = [<a: box> list, predicate] list.begin.case {
  .end! => .end!,
  .item(x) xs => predicate(x).case {
    .true! => .item(x) xs.loop,
    .false! => xs.loop,
  }
}
```

Note the types:
- We accept a `List<a>`.
- The constraint `a: box` says elements may be discarded.
- The result is still a `List<a>`.

Let’s try it out:

```par
def Evens = Filter(Int.Range(1, 100), box [n] {Int.Mod(n, 2) == 0})
```

Here:
- `Int.Range(1, 100)` gives a `List<Int>`.
- `Int` satisfies the `box` constraint, because integers are non-linear.
- The result is inferred as `List<Int>`.

This keeps the list type clear. The constraint says what the implementation needs, without
wrapping every element in a redundant `box`.
