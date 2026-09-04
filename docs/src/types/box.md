# Box

A plain [function](./function.md) in Par is linear: if we store it in a local variable, we must
call it exactly once. But what if we want to call it for every element in a list?

For that, we can use a box. A box holds a suspended computation. Each time we unbox it, we start
a new instance of that computation, producing a new value.

The type is spelled `box T`, where `T` is the type of value the computation produces:

```par
type IntCalculation = box Int
type ReusableFunction = box [Int] String
```

Every box is [shareable](../types_and_expressions.md#linearity), regardless of its contents.
We can copy it, unbox it any number of times, or leave it unused.

## Construction

A box is constructed by putting `box` before an expression:

```par
module Main

import {
  @core/Int
  @core/List
  @core/String
}

def Sum : box Int = box Int.Range(0, 1_000_000)->List.Sum
```

Without `box`, the expression computes the sum of a million integers. With `box`, we get a
**suspended computation** instead. We haven't added up any numbers yet!

A box can also use local variables defined outside its body:

```par
dec MakeAdder : [Int] box [Int] Int
def MakeAdder = [amount] box [n] n + amount
```

Here, the box _captures_ `amount`. It contains both the suspended computation and its captured
variables. Each new computation gets its own copies of the captured variables.

That's why **all captured variables must be shareable.** An integer like `amount` is fine,
and so is another box. A plain linear function wouldn't be: capturing it would let us call
that same function repeatedly!

## Destruction

To start the computation, apply `.unbox`:

```par
def Total : Int = Sum.unbox
```

That's how we turn a `box Int` into an `Int`. In general, `.unbox` turns a `box T` into a `T`.
We call this _instantiating_ the box.

Suppose we want to use the sum twice:

```par
dec Add : [Int, Int] Int
def Add = [x, y] x + y

def Twice : Int =
  let n = box Int.Range(0, 1_000_000)->List.Sum
  in Add(n.unbox, n.unbox)
```

How many times do we add up the million integers? **Twice.** Each `.unbox` starts a new computation.
The box doesn't remember the result of an earlier use.

To compute the sum once, we can unbox it and give the result a name:

```par
def Once : Int =
  let n = box Int.Range(0, 1_000_000)->List.Sum
  in let value = n.unbox
  in Add(value, value)
```

Now we share the integer result of a single computation. Before, we copied the box; now, we copy
its result.

> Unboxing doesn't wait for the computation to finish. As usual in Par, the computation runs
> concurrently with the code using its result.

Could we skip the `.unbox` and just write `Add(n, n)`? No: **`box T` and `T` are distinct types.**
Neither is a subtype of the other, even when `T` is already shareable. `Add` expects integers,
so it can copy its arguments without accidentally repeating a computation. A function that
wants a repeatable computation can ask for `box Int` instead.

## Reusable functions

Let's use `MakeAdder`:

```par
def Explicit : Int =
  let addTen = MakeAdder(10)
  in Add(addTen.unbox(1), addTen.unbox(2))  // = 23
```

Each `.unbox` produces a fresh function, and each function is called once. But writing `.unbox`
before every call would get tedious, so Par lets us leave it out:

```par
def Implicit : Int =
  let addTen = MakeAdder(10)
  in Add(addTen(1), addTen(2))  // = 23, just like above
```

**Operations on the contents of a box automatically unbox it first.** Calling a boxed function
is one example. Selecting a branch on a boxed [choice](./choice.md), or matching a boxed
[either](./either.md) with `.case`, works the same way.

Passing a box as an argument doesn't operate on its contents, which is why `Add(n, n)` is
invalid. Calling `addTen(1)` does: it instantiates the boxed function and calls it.

Now we can write the list-mapping function we wanted at the start:

```par
dec Map : [<a> List<a>, <b> box [a] b] List<b>
def Map = [<a> list, <b> f] list.begin.case {
  .end! => .end!,
  .item(x) xs => .item(f(x)) xs.loop,
}

def NumberStrings = Map(Int.Range(1, 100), box [n] `#{n}`)
```

Each `f(x)` instantiates a function and calls it on the next element. When we reach the end of the
list, we can simply leave `f` unused, because it's a box.
