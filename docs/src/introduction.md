# Introduction

**Par** is affectionately named after the most bemusing connective of
[linear logic](https://en.wikipedia.org/wiki/Linear_logic): ⅋, pronounced _"par"_.
That's because Par is based directly on (classical) linear logic, as an experiment to
see where this paradigm can take us.

[Jean-Yves Girard](https://en.wikipedia.org/wiki/Jean-Yves_Girard) — the author of linear logic,
and [System F](https://en.wikipedia.org/wiki/System_F), among other things — wrote on the page 3
of his [first paper on linear logic](https://www.sciencedirect.com/science/article/pii/0304397587900454):

> The new connectives of linear logic have obvious meanings in terms of
parallel computation, especially the multiplicatives.

This was in 1987. In hindsight, it wasn't that obvious.

Par is an attempt to take that idea seriously — to turn linear logic into a practical programming language.

## Why _Par_?

Based on linear logic, Par has a **linear type system.** That's close to what you know from Rust:
linear values have a single owner and are moved instead of copied.
But unlike in Rust, linear values **cannot be discarded.** Instead, they have to be consumed according to their type. _(Also, see [auto-cleanup](./types/auto_cleanup.md).)_

This unlocks something special: channels that may only be consumed by sending.
Now the receiver has a new guarantee — it no longer has to consider the sender forgetting to communicate.

As a consequence, **concurrent communication** is as transparent and composable in Par as calling functions.
Together with Par's imposition of a tree-like communication structure — **ruling out deadlocks** — a new
promising way of building concurrent applications arises.

But, Par isn't just a concurrent language.

Classical linear logic is a beast, and a powerful one
at that. Par absorbs all this power into its own expressivity. With [**duality,**](./processes/duality.md)
**session types,** and a rich set of concepts all mapping to logical connectives,
multiple paradigms emerge naturally:

- **Functional programming** with side-effects via linear handles.
- A **unique object-oriented style,** where interfaces are just types and implementations are just values.
- An **implicit concurrency,** where execution is non-blocking by default.

Multi-paradigm language often burden its users with multiple ways to solve the same problem.

But, somewhat surprisingly, we found that in Par, **any single problem tends to have a single best solution.**
That solution may be functional, object-oriented, a mix of those, or something else entirely.

Whichever one it is, it always puts a new puzzle into something there underneath: **the _Par_ way.**

## Orthogonality goes _wide,_ not deep

Par doesn’t have dependent types, metaprogramming, higher-order kinds, or a macro system.
Instead of going deeper into complexity, **Par goes wider.**

Its design focuses on small, composable ideas. For Par, it's not that important to have a _small number_
of features. What's important is that _each feature is small,_ and covers something no other feature does.

Most of those ideas are taken directly from classical linear logic.
Every type corresponds to a logical connective. Even recursion! This has two consequences:

- **Everything fits together.** Almost any combination of features has a meaningful use.
- **Everything is a little different.** For better or worse, Par is one of its kind.

The former is great. The latter means Par might feel like learning programming all over again.
Will that be worthwhile? We're going to have to find out.

## An ambitious stride towards _totality_

As if session types and concurrency weren't enough, Par also aims to be **total.**

That means:
- **No exceptions or panics.**
- **No deadlocks.** Par imposes a structure where deadlocks are impossible to express.
- **No accidental non-termination.** By default, recursion and corecursion are checked to prevent infinite loops.

> No infinite loops? How do I write a web server or an event loop? Don't worry, that's not infinite loops,
> that's what we call _corecursion._ It's covered thoroughly by [iterative](./types/iterative.md) types
> with totality ensuring they always advance to their next step.

At the moment, the system isn't powerful enough to capture some more complex algorithms, so there's an
escape hatch. But, the eventual goal is to get rid of it.

## Let's dive in!

Par is a language in active development. It’s not production-ready — but it’s expressive, and ready to be
explored.

If you’re curious about what programming can look like when guided by logic — **turn the page.**
