# Pair

A pair is two independent values packed into one. The only thing that differentiates pairs in Par,
compared to other languages, is their sequential syntax. While unusual, it makes pairs applicable
to a much wider set of use-cases.

A pair type consists of two types, the first enclosed in round parentheses.

```par
module Main

import {
  @core/Int
  @core/String
}

type Pair = (String) Int
```

If the second type is another pair, we can use _syntax sugar_ to write it more concisely:

```par
type Triple1 = (String) (Int) String
type Triple2 = (String, Int) String
// these two are exactly the same type
```

For a **symmetric pair** syntax, it's idiomatic to use the [unit](./unit.md) type as the last element.

```par
type SymmetricPair = (String, Int)!
```

Pairs in their sequential style are frequently used in combination with other types to insert values
into bigger structures. The predefined `List<a>` type uses a pair for its `.item` variant:

```par
type List<a> = recursive either {
  .end!,
  .item(a) self,
}
```

An infinite stream type may use a pair to produce an element together with the remainder of the stream:

```par
type InfiniteStream<a> = iterative choice {
  .close* => !,
  .next  => (a) self,
}
```

## Construction

Pair values look the same as their types, with values instead of types in place of elements.

```par
def Example1: Pair    = ("Hello!") 42
def Example2: Triple1 = ("Alice") (42) "Bob"

// `Triple1` and `Triple2` really are the same type
def Example3: Triple1 = ("Alice", 42) "Bob"
def Example3: Triple2 = ("Alice", 42) "Bob"

// notice the `!` at the end
def Example4: SymmetricPair = ("Hello!", 42)!
```

When embedded in other types, sequential pairs blend in seamlessly:

```par
def Names: List<String> = .item("Alice").item("Bob").item("Cyril").end!
//                             |             |           |_____________
//                             |             |_________________________
//                             |_______________________________________
```

## Destruction

Pairs are deconstructed in patterns on assignments. Those can appear in:
- [`let`-expressions](../structure/let_expressions.md)
- [function](./function.md) arguments
- [`case`](./choice.md)/[`.case`](./either.md) branches

Aside from pairs and whole values, [unit](./unit.md) types can be matched in patterns, too.

Here are some examples:

```par
def Five: Int =
  let (x) y = (3) 2
  in x + y

def FiveSymmetrically: Int =
  let (x, y)! = (3, 2)!
  in x + y

dec AddSymmetricPair : [(Int, Int)!] Int
def AddSymmetricPair = [(x, y)!] x + y
//                      \_____/<---- pattern here

dec SumList : [List<Int>] Int
def SumList = [list] list.begin.case {
  .end!       => 0,
  .item(x) xs => x + xs.loop,
//     \____/<---- pattern here
}
```
