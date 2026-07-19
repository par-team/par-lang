# Channels & Linking

In the previous sections, we built up values using the `do`/`in` expressions:

```par
def Message = do {
  let builder = String.Builder
  builder.add("Hello")
  builder.add(", ")
  builder.add("World")
} in builder.build
```

This form lets you run a process — a sequence of commands — and then return a final expression
at the end.

But what if you want to **return early,** based on some logic? Par has no _return_ keyword.
But it does have a deeper mechanism that allows exactly this — and more.

The `do`/`in` syntax is just **sugar** for something more fundamental: the `chan` expression,
combined with a command called linking. This:

```par
do {
  <process>
} in <value>
```

is exactly the same as:

```par
chan result {
  <process>
  result <> <value>
}
```

Let’s unpack what this means.

## The `chan` Expression

```par
chan inner {
  // commands here
}
```

This expression creates a new channel, and spawns a process that runs the commands inside the
curly braces. That process is given access to one endpoint of the new channel —
named here as `inner`.

The `chan` expression itself evaluates to the other endpoint of the channel — that’s the value
the expression returns.

**The key is this:** If the whole chan expression has type `T`, then inside the block, `inner`
has type `dual T`.

We’ll explore the details of dual types in the next section, but for now, here’s the intuition:
**A type and its dual are two opposite side of communication.** If one side offers something,
the other one requires it, and vice versa.

For example, if the whole `chan` expression should evaluate to an `Int`, then `dual Int` is
a _requirement_ of an integer.

## Linking — the `<>` command

The link command is the simplest way to fulfill that requirement. If `x` has type `T`
and `y` has type `dual T`, then:

```par
x <> y
```

**Connects the two endpoints,** and ends the process.

For example:

```par
def FortyTwo: Int = chan out {
  out <> 42
}
```

This returns the number `42`, using a full process.

Like all commands, `<>` operates on values — here, two channels — not processes.
You’re not linking a process to a value. You’re linking one channel to another.

> **Processes in `chan` must end.** Every process inside a `chan` must end with one of two
> commands:
> - A **link command** (x <> y), or
> - A **break command** (x!), which we’ll meet in the next section.
> 
> These are the only ways a process can terminate. If you reach the end of a `chan` block
> without either one, it’s an error. This may seem a strange condition, but it's important for
> some concurrency invariants Par provides.
> 
> In contrast, `do`/`in` blocks don’t end themselves — they continue into the expression after `in`.

Let’s write a function that can use that **early return**.

It takes a natural number (`Nat`) and returns a `String`.

- If the number is zero, it returns the string `"<nothing>"`.
- Otherwise, it returns a string of exactly `n` hash marks (`"#"`).

Here’s how that looks:

```par
module Main

import {
  @core/Nat
  @core/String
}

dec HashesOrNothing : [Nat] String
def HashesOrNothing = [n] chan result {
  {n == 0}.case {
    .true! => {
      result <> "<nothing>"
    }
    .false! => {}
  }

  let builder = String.Builder
  Nat.Repeat(n).begin.case {
    .end! => {}
    .step remaining => {
      builder.add("#")
      remaining.loop
    }
  }

  result <> builder.build
}
```

Let’s break this down:

- `n == 0` returns a `Bool`, which is an [`either`](../types/either.md) with branches `.true!` and
  `.false!`. The braces let that temporary result be used directly as the subject of `.case`.
- In the `.true!` branch, we immediately perform a link — returning `"<nothing>"`. Note, that
  this _ends_ the process.
- The `.false!` branch does nothing on its own — the real work happens after the `.case`.
- We build the result using a `String.Builder`, looping exactly `n` times using `Nat.Repeat`.
- Finally, we link the builder’s result to `result`, ending the process.

In the next section, we'll learn how to exploit duality, to not just return, but sequentially
build on the opposing channels!
