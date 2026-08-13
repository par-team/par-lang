# Auto-Cleanup

The previous chapter introduced the [`drop` constraint](./constraints.md#the-drop-constraint). A
value whose type satisfies `drop` may be left unused.

For an `Int`, this is not surprising. But what if the unused value is an open file, a stream, or a
transaction? These are linear values. We cannot simply forget their endpoints — the process at the
other end may still be waiting for a close, cancel, or rollback signal.

But having to always clean everything up by hand would be cumbersome, especially on error paths.
In fact, [error handling](../quality_of_life/error_handling.md) is the main reason why Par supports
auto-cleanup. Without it, each error path would have to explicitly close all the pending resources,
and those resources could be different for each path! That's a lot of boilerplate when trying to focus
on the happy path.

## The Star Is a Contract

To make a linear resource droppable, its protocol has to tell Par how to finish it. We do it
by adding cleanup markers (denoted `*`) to [`choice`](./choice.md) types. Without a cleanup marker, even a
one-branch resource remains strictly linear:

```par
type StrictResource = choice {
  .release => !,
}

dec IgnoreStrict : [StrictResource] !
def IgnoreStrict = [resource] !  // Error! `resource` was not consumed.
```

Why doesn't Par just choose `.release`? Because branch names have no built-in meaning. Another
protocol might offer both `.commit` and `.rollback`, and the compiler cannot know which one the
program intends.

We can declare that `.release` is safe for cleanup by marking it with a star in the type:

```par
type Resource = choice {
  .release* => !,
}

dec Ignore : [Resource] !
def Ignore = [resource] !  // Okay.
```

Once `resource` is no longer referenced, Par selects `.release*` automatically.
A choice may have at most one such branch.

The star also appears when we construct the choice value:

```par
def NewResource: Resource = case {
  .release* => !,
}
```

The two markers play different roles. In the `Resource` type, the star promises that cleanup is available.
In the actual `Resource` value construction (the `case` expression), it registers the cleanup branch,
so that Par's runtime can identify and call it while cleaning up. In that sense, the second star is
fulfilling the protocol demanded by the type.

The marker is also allowed in [`either`](./either.md) types because `choice` and `either` are two sides
of the same protocol. We will return to that when discussing [duality](../processes/duality.md).

## Cleanup Follows the Type

Cleanup continues through the value returned by the marked branch.

```par
type Finalizer = choice {
  .finish* => !,
}

type TwoStageResource = choice {
  .release* => Finalizer,
}

dec IgnoreTwoStage : [TwoStageResource] !
def IgnoreTwoStage = [resource] !
```

Cleaning up `resource` first selects `.release*`, which produces a `Finalizer`. Par then selects
`.finish*` on that finalizer. In this example, cleanup does the same work as:

```par
resource.release.finish
```

Cleanup is **structural**: Par follows the shape of the value being discarded.

- A shareable value needs no action.
- A pair cleans up both of its parts.
- An either cleans up the payload that is actually present.
- A recursive value is cleaned up according to its finite structure.
- A choice selects its marked branch, then continues with the result.

Because this rule applies recursively, a whole list of resources may be left unused:

```par
dec IgnoreAll : [List<Resource>] !
def IgnoreAll = [resources] !
```

Par walks through the list and selects `.release*` on every resource inside it.

So, when does a choice satisfy `drop`? Two things have to be true:

1. it has a branch marked with `*`; and
2. the result of that branch satisfies `drop` too.

A star alone is not enough. `IncompleteCleanup` still does not satisfy `drop`:

```par
type IncompleteCleanup = choice {
  .release* => [String] !,
}
```

Selecting `.release*` produces a linear function, which still has to be called exactly once. The
star gave Par one cleanup step, but not a complete route to the end.

When the cleanup result contains a type parameter, that parameter may determine whether cleanup can
finish. Consider a writer whose `.close*` operation may fail:

```par
type Writer<e> = iterative choice {
  .close* => Try<e, !>,
  .write(Bytes) => Try<e, self>,
}
```

`Writer<e>` satisfies `drop` only when `e` does. Otherwise, an `.err e` returned by `.close*` could
leave us with a value that cannot itself be cleaned up. So a generic function that leaves the writer
unused has to put `drop` on `e`:

```par
dec AbandonWriter : [type e: drop, Writer<e>] !
def AbandonWriter = [type e: drop, writer] !
```

The same constraint lets us discard a value without knowing anything else about its type:

```par
dec Discard : [<a: drop> a] !
def Discard = [<a: drop> value] !
```

No other information about `a` is needed: `drop` tells `Discard` that any value of type `a` can be
cleaned up.

Par starts cleanup as soon as a value is no longer referenced on the current process path. It does
not need to wait for the surrounding function to finish. If the last reference comes before a
blocking operation such as `.case`, cleanup begins as soon as the process starts waiting there.

Cleanup also happens when a droppable linear value is shadowed by another value with the same
name. Strictly linear values still have to be consumed explicitly before they can be shadowed.

## Automatic or Explicit?

A marked cleanup branch remains an ordinary operation. We can always select it ourselves:

```par
let result = writer.close
```

Selecting `.close` ourselves leaves its `result` available to inspect. On the other hand, if `writer`
is left unused, Par selects `.close*` and then cleans up the returned `Try`. If closing returns `.err`,
that error is cleaned up too, so we never observe it.

If we are already leaving because of another error, ignoring a second error from `.close*` is often
what we want. But on a successful path, closing a writer may flush buffered output and fail. There
we call `.close` ourselves and propagate its error:

```par
writer.close.try
```

The `.try` keeps the closing error by sending it to the nearest `catch`, rather than letting cleanup
discard it. We will cover both constructs in [Error Handling](../quality_of_life/error_handling.md).

The standard library uses this pattern in several places. `Console`, `Bytes.Reader`, and
`Bytes.Writer` provide `.close*`. `Stream` provides `.cancel*`, while `Sql.Transaction` provides
`.rollback*`.

Not every choice has a cleanup branch. The marker says that a branch is always safe to select when
the value is left unused. If no branch has that property, none is marked. The choice then remains
strictly linear and must be consumed explicitly.
