# Pipes

Pipes are a piece of syntax sugar that keep a chain of operations readable.
They do not add new behaviour to Par. They just let you write the
“do this, then that” story from left to right without inventing throwaway
names.

## A first taste

Here are some helper functions that operate on numbers:

```par
module Main

import @core/Nat

def Add    = [m: Nat, n: Nat] m + n
def Double = [n: Nat] n + n
def Square = [n: Nat] n * n
```

Without pipes you call them inside one another:

```par
let result = Square(Add(3, Double(4)))  // = 121
```

With pipes you can say the same thing in the order you want to read it:

```par
let result = 3
  -> Add(Double(4))
  -> Square
```

The pipe feeds the value on the left into the **first argument** of the
function on the right. Nothing else changes — the two version above do
the exact same thing.

## Expression semantics

Whenever you see

```par
value -> Func(args)
```

read it as

```par
Func(value, args)
```

It helps to think of `->Func` as one of the operations in the sequence, just
like `(args)` or `.case { ... }`. You can even rewrite the earlier example as
`value ->Func (args)` to make this explicit.

```par
Account.Lookup(name)
  ->Auth.Check
  .case {
    .ok user   => user.balance
    .err _     => 0
  }
```

If you need to push the value into a later argument, wrap the call in braces so
the whole expression becomes the operation:

```par
value -> {Func(arg1)}(arg3)  // == Func(arg1, value, arg3)
```

Names and values such as `3`, `"text"`, `*(1, 2, 3)`, and `<<1 2 3>>` can
start a pipe or another postfix operation directly. Compound expressions and
constructions need braces in that position:

```par
{if { ready => value, else => fallback }}->Func
{.true!}.case { .true! => yes, .false! => no }
n->{box [x] x + n}
```

The braces turn the entire enclosed expression into one value before the
postfix operation starts. They are also how you pipe a whole infix expression:
`{1 + 2}->Func`. Without them, `1 + 2->Func` applies `Func` only to `2`.

## Pipes as commands

In process syntax every command “uses” the variable on its left and stores the
result back into that same variable. Pipes follow the same rule, which means
you can treat any function as if it were a built-in command.

```par
module Main

import {
  @core/List
  @core/Option
  @core/Nat
}

dec Push : [List<Nat>, Nat] List<Nat>
def Push = [stack, value] .item(value) stack

let stack = *(1, 2, 3)
stack->Push(4)
// exactly the same as: let stack = Push(stack, 4)
```

Pipes make destructuring commands pleasant too. Take a `Pop` function that
returns a head and a tail:

```par
dec Pop : [List<Nat>] (Option<Nat>) List<Nat>
def Pop = [list] list.case {
  .end! => (.none!) .end!,
  .item(x) xs => (.some x) xs,
}

let numbers = *(10, 20, 30)
numbers->Pop[top]
// top     : Option<Nat>
// numbers : List<Nat>  -- now *(20, 30)
```

Again, the command is just a prettier spelling of
`let (top) numbers = Pop(numbers)`.

This is the reason the pipe feeds the **first** argument: it lets you extend
the familiar `value.operation` sequences that already exist in the language
without introducing special cases.

## Wrapping up

- Pipes keep left-to-right reading order while preserving Par’s evaluation
  rules. Drop them and you get the original nested calls back.
- They play nicely with the rest of the syntax: mix them with `case`, `.`
  selections, further commands — anything that already appends to the sequence.
- If the first argument is not the one you need, braces give you full control.

Whenever you notice yourself inventing a `tmp` variable simply to pass it to
another function, try a pipe — it’s often all the ceremony you actually need.
