# Receiving, Where It Shines

The previous section showed the `value[variable]` receive command — but it didn’t feel essential.
Let’s now explore a use-case where this command really shines: polling values from an
infinite sequence.

You may remember the `Sequence<a>` type from earlier. Here's the definition again:

```par
type Sequence<a> = iterative choice {
  .close* => !,
  .next => (a) self,
}
```

A `Sequence` can generate an unbounded number of values, one by one, and eventually be closed.

## A Fibonacci sequence, as a value

Let’s start by building a sequence of Fibonacci numbers.

```par
dec Fibonacci : Sequence<Nat>
def Fibonacci =
  let (a) b = (0) 1
  in begin case {
    .close* => !
    .next =>
      let (a) b = (b) {a + b}
      in (a) loop
  }
```

The internal state of the sequence is a pair `(a) b`. On each `.next`, we emit `a` and update
the pair. This is a clean and elegant use of corecursive iteration, as we've covered it in the
section on [iterative types](../../types/iterative.md).

So far so good — but how do we use this value?

## Goal: print the first 30 Fibonacci numbers

Let’s say we want to print the first 30 Fibonacci numbers to the terminal.

That means:

1. Repeating an action 30 times,
2. Receiving a number from the sequence each time,
3. Converting it to a string,
4. Sending it to the output.

We’ll need a way to loop **exactly 30 times.** But there’s a catch: In Par,
**looping is only possible on recursive types.** There’s no built-in recursion on `Nat`, so we need
to convert the number 30 into a [recursive](../../types/recursive.md).

Good news: there’s a built-in helper for that! It's called `Nat.Repeat`, here's its type:

```par
dec Nat.Repeat : [Nat] recursive either {
  .end!,
  .step self,
}
```

Calling `Nat.Repeat(30)` gives us a recursive value that can be stepped through exactly 30 times.
Precisely what we need.

Now for the second part: printing.

## `Console` output

Par comes with a built-in definition for working with standard output:

```par
def Console.Open : Console
```

The `Console` type itself is defined like this:

```par
type Console = iterative choice {
  .close* => !,
  .print(String) => self,
}
```

You can think of it as a "sink" object that receives strings and sends them to the terminal.
Much like `String.Builder`, but the output appears directly in your console.

Let’s tie it all together!

```par
module Main

import {
  @basic/Console
  @core/Nat
}

def Program: ! = do {
  let console = Console.Open
  let fib = Fibonacci

  Nat.Repeat(30).begin.case {
    .end! => {}
    .step remaining => {
      fib.next[n]
      console.print(`#{n}`)
      remaining.loop
    }
  }
} in !
```

Here’s what’s happening:

- `Nat.Repeat(30)` becomes the subject of the `.begin` and `.case` commands.
- In each `.step`, we receive a number from the `fib` sequence using the `fib.next[n]` command.
- We convert it to a string with a template string and print it to the console.
- Then we `loop`.

The `fib.next[n]` line is where _receive_ command truly shines. It receives the payload
of the `.next` branch — a number — and updates `fib` to the rest of the sequence. Note, that
it's a combination of two commands: a _selection_ and a _receive_.

Once done, both `fib` and `console` are still linear, but their `.close*` branches make them
droppable. Reaching the final `!` cleans them up automatically. We could write `fib.close` and
`console.close` explicitly, but it's not necessary here.