# Recursive

_Par_ has, among others, these two ambitious design choices:
- **Totality,** meaning preventing infinite loops by type-checking.
- **A structural type system,** where global type definitions are merely aliases.

When it comes to self-referential types, totality necessitates distinguishing between:
- _Recursive types_, those are finite.
- _Corecursive types_, potentially infinite. In Par, we call them [_iterative types_](./iterative.md).

The choice of a structural type system has led to avoiding defining self-referential types naively,
and instead adding a first-class syntax for anonymous self-referential types.

Par is very radical here. If you try the usual way of defining a singly-linked list, it fails:

```par
type IllegalList = either {
  .end!,
  .item(String) IllegalList,  // Error! Cyclic dependency.
}
```

In general, **cyclic dependencies between global definitions are disallowed.** Instead, we have:
- Anonymous self-referential types: `recursive` and [`iterative`](./iterative.md).
- A single, universal recursion construct: `begin`/`loop`. It's suitable for recursive destruction,
  iterative construction, and imperative-style loops in [process syntax](../process_syntax.md).

**Let's take a look at `recursive`!**

> **Totality _does not_ mean you can't have a web server, or a game.** While these are often
> implemented using infinite event loops, it doesn't have to be done that way. Instead, we can employ
> corecursion, which Par supports with its [iterative](./iterative.md) types.
>
> To make it clearer, consider this Python program:
>
> ```python
> def __main__():
>     while True:
>         req = next_request()
>         if req is None:
>             break
>         handle_request(req)
> ```
>
> That's a simplified web server, handling requests one by one, using an infinite loop.
>
> Could we switch it around and not have an infinite loop? Absolutely!
>
> ```python
> class WebServer:
>     def close(self):
>         pass
>
>     def handle(req):
>         handle_request(req)
>
> def __main__():
>     start_server(WebServer())
> ```
>
> A small restructuring goes a long way here. [Iterative](./iterative.md) types in Par enable
> precisely this pattern, but with the ergonomics of the infinite loop version.

A recursive type starts with the keyword `recursive` followed by a body that may contain any number
of occurrences of `self`: the self-reference.

```par
type LegalList = recursive either {
  .end!,
  .item(String) self,  // Okay.
}
```

> **If there are nested `recursive` (or [`iterative`](./iterative.md)) types, it may be necessary to
> distinguish between them.** For that, we can attach **labels** to `recursive` and `self`. That's
> done with an `@`: `recursive@label`, `self@label`. Any lower-case identifier can be used for the
> label.

The recursive type can be thought of as being equivalent to its _expansion_. That is, replacing each
`self` inside the body with the `recursive` type itself:

1. The original definition:
   ```par
   recursive either {
     .end!,
     .item(String) self
   }
   ```
2. The first expansion:
   ```par
   either {
     .end!,
     .item(String) recursive either {
       .end!,
       .item(String) self
     }
   }
   ```
3. The second expansion:
   ```par
   either {
     .end!,
     .item(String) either {
       .end!,
       .item(String) recursive either {
         .end!,
         .item(String) self
       }
     }
   }
   ```
4. And so on...

> The body of a `recursive` often starts with an `either`, but doesn't have to. Here's an example of that:
> a non-empty list, which starts with a pair.
>
> ```par
> type NonEmptyList<a> = recursive (a) either {
>   .end!,
>   .item self,
> }
> ```
>
> Another example of a `recursive` type, which doesn't start with an `either` would be a finite stream.
>
> ```par
> type FiniteStream<a> = recursive choice {
>   .close => !,
>   .next => either {
>     .end!,
>     .item(a) self,
>   }
> }
> ```
>
> This one starts with a [choice](./choice.md), which enables polling the elements on demand, or
> cancelling the rest of the stream. However, being recursive, a `FiniteStream<a>` is guaranteed
> to reach the `.end!` eventually, if not cancelled.
>
> There is nonetheless an important restriction: in order for `self` references to remain useful,
> **every `self` reference for a `recursive` must be guarded by an `either`.** The `either` doesn't
> have to be right next to the `recursive`, but it *has* to be somewhere in-between `recursive` and
> `self`:
>
> ```par
> type ValidList<a> = recursive (a) either {
>   .end!,
>   .item self,  // Okay. This `self` is guarded by an `either`.
> }
> 
> type InvalidList<a> = recursive (a) self  // Error! Unguarded `self` reference
> ```
> 
> [Iterative](./iterative.md) types have a similar restriction: **their `self` reference must be
> guarded by a [`choice`](./choice.md).**

The key features of _recursive types_ are that **their values are finite,** and that
**we can perform recursion on them.**

## Construction

Recursive types don't have any special construction syntax. Instead, we directly construct their
bodies, as if they were expanded.

```par
type Tree = recursive either {
  .leaf Int,
  .node(self, self)!,
}

def SmallTree: Tree = .node(
  .node(
    .leaf 1,
    .leaf 2,
  )!,
  .node(
    .leaf 3,
    .leaf 4,
  )!,
)!
```

Already constructed recursive values can be used in the `self`-places of new ones:

```par
def BiggerTree: Tree = .node(SmallTree, SmallTree)!
```

**Lists** are a frequently used recursive type, and so are predefined as:

```par
type List<a> = recursive either {
  .end!,
  .item(a) self,
}
```

Constructing them goes like:

```par
dec OneThroughFive  : List<Int>
dec ZeroThroughFive : List<Int>

def OneThroughFive  = .item(1).item(2).item(3).item(4).item(5).end!
def ZeroThroughFive = .item(0) OneThroughFive
```

Because lists are so ubiquitous, there is additionally a **syntax sugar** for constructing them more
concisely:

```par
def OneThroughFive = *(1, 2, 3, 4, 5)
```

However, prepending onto an existing list has no syntax sugar, so `ZeroThroughFive` still has to be
done the same way.

## Destruction

If we don't need to perform recursion, it's possible to treat recursive types as their expansions
when destructing them, too. For example, here we treat a `List<String>` as its underlying `either`:

```par
type Option<a> = either {
  .none!,
  .some a,
}

dec Head : [List<String>] Option<String>
def Head = [list] list.case {
  .end!      => .none!,
  .item(x) _ => .some x,
}
```

**For a recursive reduction, we have `.begin`/`.loop`.** Here's how it works:
1. Apply `.begin` to a value of a `recursive` type.
2. Apply more operations to the resulting expanded value.
3. Use `.loop` on a _descendent_ recursive value, descendent meaning it was a `self` in
   the original value we applied `.begin` to.

Let's see it in practice. Suppose we want to add up a list of integers.

0. We obtain a value (`list`) of a recursive type (`List<Int>`):
   ```par
   dec SumList : [List<Int>] Int

   def SumList = [list]
   ```
1. We apply `.begin` to it:
   ```par
   list.begin
   ```
2. We match on the possible variants:
   ```par
   .case {
     .end!       => 0,
     .item(x) xs =>
   ```
   If the list is empty, the result is `0`. Otherwise, we need to add the number `x`
   ```par
   x +
   ```
   to the sum of the rest of the list: `xs`.
3. Since `xs` is a _descendant_ of the original `list` that we applied the `.begin` to, and is again a
   `List<Int>`, we can recursively obtain its sum using `.loop`:
   ```par
   xs.loop
   ```
   And close the braces.
   ```par
   }
   ```

All put together, it looks like this:

```par
def SumList = [list] list.begin.case {
  .end!       => 0,
  .item(x) xs => x + xs.loop,
}
```

You can think of `.loop` as going back to the corresponding `.begin`, but with the new value.

The semantics of `.begin`/`.loop` are best explained by _expansion_, just like the recursive types
themselves. In all cases, **the meaning of `.begin`/`.loop` is unchanged, if we replace each `.loop` with the entire body starting at `.begin`.**

Observe:
1. The original code:
   ```par
   def SumList = [list] list.begin.case {
     .end!       => 0,
     .item(x) xs => x + xs.loop,
   }
   ```
2. The first expansion:
   ```par
   def SumList = [list] list.case {
     .end!       => 0,
     .item(x) xs => x + xs.begin.case {
       .end!       => 0,
       .item(x) xs => x + xs.loop,
     },
   }
   ```
3. The second expansion:
   ```par
   def SumList = [list] list.case {
     .end!       => 0,
     .item(x) xs => x + xs.case {
       .end!       => 0,
       .item(x) xs => x + xs.begin.case {
         .end!       => 0,
         .item(x) xs => x + xs.loop,
       },
     },
   }
   ```
4. And so on...

`.loop` may be applied to any number of descendants. Here's a function adding up the leafs in the `Tree`
type defined previously:

```par
dec SumTree : [Tree] Int
def SumTree = [tree] tree.begin.case {
  .leaf number        => number,
  .node(left, right)! => {left.loop} + {right.loop},
}

def BiggerSum = SumTree(BiggerTree)  // = 20
```

> **If there are multiple nested `.begin`/`.loop`, it may be necessary to
> distinguish between them.** Labels can be used here too, just like with the types:
> `.begin@label` and `.loop@label` does the job.
>
> TODO:
> ```par
> type Tree<a> = recursive List<(a) self>
> ```

### Retention of local variables

Let's consider Haskell for a moment. Say we write a simple function that increments each item in
a list by a specified amount:

```haskell
incBy n []     = []
incBy n (x:xs) = (x + n) : incBy n xs
```

This recursive function has a parameter that has to be remembered across the iterations: `n`, the
increment. In Haskell, that's achieved by explicitly passing it to the recursive call.

Now, let's look at _Par_. In Par, `.loop` has a neat feature:
**local variables are automatically passed to the next iteration.**

```par
dec IncBy : [List<Int>, Int] List<Int>
def IncBy = [list, n] list.begin.case {
  .end!       => .end!,
  .item(x) xs => .item(x + n) xs.loop,
}
```

Notice, that `xs.loop` makes no mention of `n`, the increment. Yet, `n` is available throughout the
recursion, because it is automatically passed around.

This feature is what makes `begin`/`loop` not just a universal recursion construct, but a sweet spot
between usual recursion and imperative loops.

> If you're confused about how or why it should work this way, try expanding the `.begin`/`.loop`
> in the above function. Notice that when expanded, `n` is in fact visible in the next iteration.
> It's truly the case that expanding a `.begin`/`.loop` never changes its meaning.

Together with `.begin`/`.loop` being usable deep in expressions, local variable retention is also
very useful in avoiding the need for helper functions.

Let's again switch to Haskell, and take a look at this list reversing function:

```haskell
reverse list = reverseHelper [] list

reverseHelper acc []     = acc
reverseHelper acc (x:xs) = reverseHelper (x:acc) xs
```

This function uses a state: `acc`, the accumulator. It prepends a new item to it in every iteration,
eventually reversing the whole list. In Haskell, this requires a helper recursive function.

In Par, it doesn't!

```par
dec Reverse : [type a, List<a>] List<a>
def Reverse = [type a, list]
  let acc: List<a> = .end!
  in list.begin.case {
    .end!       => acc,
    .item(x) xs => let acc = .item(x) acc in xs.loop,
  }

def TestReverse = Reverse(type Int, *(1, 2, 3, 4, 5))  // = *(5, 4, 3, 2, 1)
```

And there we go! All we had to do was to re-assign `acc` with the new value, and continue with `xs.loop`.

### The escape-hatch from totality: `.unfounded`

If the Par's type checker refuses to accept your recursive algorithm despite you being certain it's
total — meaning it resolves on all inputs — it's possible to disable the totality checking by
replacing `.begin` with `.unfounded`.

Par's totality checking is currently not powerful enough for some algorithms, especially divide
and conquer, and it's also lacking when decomposing recursive algorithms into multiple functions.
In such cases, using `.unfounded` is okay. We do, however, aim to make the type system stronger,
and eventually remove `.unfounded`.
