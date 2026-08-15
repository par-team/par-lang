# Types & Their Expressions

Types in Par serve two seemingly incompatible purposes at the same time:

- Objects of every-day programming, like functions and pairs.
- [Session-typed](https://en.wikipedia.org/wiki/Session_type) communication channels.

In the world of linear logic, these are the same thing. But to make this
connection harmonious and ergonomic, some unusual choices have to be made in the design of
the basic building blocks.

**Types in Par are sequential.** The basic building blocks — pairs, functions, eithers (sums),
and choices (co-sums) — all read as **first this, then that.**

Let's take pairs. In many programming languages, `(A, B)` is the type of a pair of `A` and `B`.
This approach is not sequential: both types assume equal position.

In Par, the pair type is instead `(A) B`. The second type being outside of the parentheses is
essential. It allows us to sequentially continue the type without the burden of nesting.

Compare `(A, (B, (C, D)))` against `(A) (B) (C) D`.

Of course, most languages that provide `(A, B)` pairs also support triples `(A, B, C)`, and
quadruples `(A, B, C, D)`, so let's mix it up!

The usual syntax for function types is `A -> B`. That is sequential, but in Par we have a syntax
that plays more nicely with the pairs: `[A] B`. Now compare

- `(A, B -> (C, D -> E))`

versus

- `(A) [B] (C) [D] E`

We can read it as: _first give `A`, then take `B`, then give `C`, then take `D`, and finally give `E`._

This is starting to look a lot like session types! An alternative reading of the type could be:
_first send `A`, then receive `B`, then send `C`, then receive `D`, and finally proceed as `E`._

And that, in a nutshell, is how Par unifies every-day types with session types.

**This chapter covers the every-day aspect of types in Par.** For the session-typed, process-oriented
aspect, check out [The Process Syntax](./process_syntax.md).

## Linearity

Par is based on [linear logic](https://en.wikipedia.org/wiki/Linear_logic), and with that comes a
**linear type system.** That means the type of a value controls not only _how_ it can be used,
but also _how many times._

The strictest values must be consumed **exactly once** — in a way their type allows.
You can't copy them, and you can't throw them away.

This might sound limiting, but it opens the door to something powerful.

When a value must be used — and can only be used once — it becomes possible to **model communication.**
Think about a channel that expects you to send a message. If you don’t send one — or send
two — things fall apart.

With linearity, Par gives you channels where that simply can’t happen.

That's the foundation of [session types](https://en.wikipedia.org/wiki/Session_type),
and Par supports them at its core.

But not every type needs that kind of strictness. Some values have a natural way to be [cleaned up](./types/auto_cleanup.md),
and some should be copyable, droppable, and passed around freely.

So Par distinguishes between three usage capabilities:

- **Strictly linear types** must be used exactly once.
- **Droppable linear types** may be used once or left unused. They still cannot be copied.
- **Shareable types** may be used any number of times — including zero.

The precise names of the last two capabilities are the [`drop` and `share`
constraints](./types/constraints.md). Every `share` type also satisfies `drop`, but not the other
way around. A file handle, for example, can be safely closed without being safe to copy.

How can Par drop a linear value without ignoring its protocol? A [choice](./types/choice.md) can mark
one branch as its cleanup branch:

```par
type Resource = choice {
  .release* => !,
}
```

The star declares `.release` to be the canonical way out. If a `Resource` remains unused when its
process finishes, Par selects that branch automatically. This is covered fully in
[Auto-Cleanup](./types/auto_cleanup.md).

### Which types are shareable?

These include:

- All [**primitives**](./structure/primitive_types.md): `Int`, `Nat`, `Float`, `String`, `Char`,
  `Byte`, and `Bytes`
- [**Unit**](./types/unit.md)
- [**Either**](./types/either.md), [**pair**](./types/pair.md), and
  [**Recursive**](./types/recursive.md) and [**Iterative**](./types/iterative.md) types whose body is shareable
- Every [**box**](./types/box.md), regardless of the type of its content
- [**Forall**](./types/forall.md) and [**Exists**](./types/exists.md) whose body is shareable

### Which types are droppable?

These include all the shareable types above, plus:

- [**Choice**](./types/choice.md) types with a usable cleanup branch, that is one whose result is also droppable.

Droppability is structural. Dropping a pair drops both of its parts; dropping an either drops the
payload that is actually present; dropping a recursive structure walks through it. Eventually,
cleanup reaches shareable values — which need no action — and marked choices — which say what action to
perform.

See [Type Constraints](./types/constraints.md) and [Auto-Cleanup](./types/auto_cleanup.md) for more information.

### Which types are strictly linear?

Everything that is neither shareable nor droppable is strictly linear. Common examples are:

- A [**function**](./types/function.md)
- A [**choice**](./types/choice.md) without a cleanup branch
- A [**continuation**](./types/continuation.md)
- Any type that **contains** a strictly linear component, even deeply

If a type has a linear piece **anywhere inside it,** it becomes linear — unless that part is
wrapped in a [box](./types/box.md).
