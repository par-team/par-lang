# Switching Modes With `repoll`

Sometimes a server needs to "switch gears".

For example, imagine merging a bunch of cancelable streams into a single cancelable stream:
- While the merged stream is being consumed, the server should keep pulling items from the sources.
- But if the consumer decides to cancel the merged stream, **all underlying sources must be canceled too.**

This is exactly the kind of situation `repoll` is for: it lets a server keep the same pool, but start polling it with a *different handler*.

## A cancelable stream: `Source<a>`

To talk about cancellation, it helps to contrast with `List<a>`.

Once a `List<a>` is created, it will keep producing items until it's done — even if the consumer would prefer to stop early. To make cancellation possible, we can switch to a protocol where the producer and consumer cooperate: the producer can _offer_ work, but the consumer can choose to stop.

Let's call such a stream-like protocol `Source<a>`. A first idea might be:

```par
type Source<a> = recursive choice {
  .close => !,
  .next => either {
    .end!,
    .item(a) self,
  }
}
```

This is a perfectly valid protocol, but it works poorly with `poll`: each source is stuck waiting for the server to choose `.close` or `.next`, so the sources are not "ready" on their own and we lose most of the concurrency.

A second idea is to let the source produce items first, and only then ask the consumer how to proceed:

```par
type Source<a> = recursive either {
  .end!,
  .item(a) choice {
    .close => !,
    .next => self,
  }
}
```

This is closer, but it has an awkward corner when merging: when the consumer of the merged stream decides to `.close`, the underlying sources may have already produced more `a`s — but we have nowhere to send them.

The solution is to separate "an item is available" from "the item is transferred":

```par
type Source<a> = recursive either {
  .end!,
  .item choice {
    .discard => !,
    .get => (a) self,
  }
}
```

Here, the source can make progress on its own (it can produce `.item` or `.end!`), which works great with `poll`. But when it produces `.item`, it doesn't immediately hand out the value — it waits for the consumer to decide:
- `.get` to receive the value and continue.
- `.discard` to cancel (and the source ends with `!`).

> 💡 This is **cooperative cancellation**: the consumer can only cancel at points where the producer is willing to accept cancellation (here: right after offering `.item`).

## Switching from "produce" to "cancel"

Now suppose we have many sources, and we want to merge them into one:

```par
dec MergeSources : [<a> List<Source<a>>] Source<a>
```

When the merged source is canceled, all underlying sources must be canceled too. That means our server needs two distinct modes:

- **Normal mode:** pull items from ready sources and emit them downstream.
- **Cancel mode:** stop pulling items and instead discard every remaining source.

This mode switch is what `repoll` expresses.

## `repoll`

`repoll` looks like `poll`, but it does not create a new pool. Instead, it reuses the pool of the nearest enclosing `poll` (if any), optionally adds some clients into it, and starts polling it with a new handler.

```par
poll(...) {
  client => ... repoll(...) {
    client => ...
    else => ...
  }

  else => ...
}
```

It can **only** be used _inside_ an active branch of a `poll`, or another `repoll`.

## Example: Merging cancelable sources

We'll reuse the same fan idea from the [fan pattern](./fan_pattern.md), but now for `Source<a>`:

```par
type Source<a> = recursive either {
  .end!,
  .item choice {
    .discard => !,
    .get => (a) self,
  }
}

type SourceFan<a> = recursive either {
  .end!,
  .spawn(self) self,
  .item choice {
    .discard => !,
    .get => (a) self,
  }
}

dec SourceFan : [<a> List<Source<a>>] SourceFan<a>
def SourceFan = [<a> sources] sources.begin.case {
  .end! => .end!,
  .item(source) sources => .spawn(source) sources.loop,
}
```

Now we can implement:

```par
dec MergeSources : [<a> List<Source<a>>] Source<a>
```

The key idea is that `MergeSources` has two modes:
- **Normal mode:** keep producing items by polling sources.
- **Cancel mode:** once the consumer cancels, discard everything left in the pool.

Here's the overall structure:

```par
def MergeSources = [<a> sources] poll(SourceFan(sources)) {
  fan => fan.case {
    .end! => submit(),
    .spawn(l) r => submit(l, r),

    .item s => .item case {
      .discard => ...   // switch into cancel mode
      .get => ...       // produce one item and keep going
    }
  }

  else => .end!,
}
```

### Normal mode: produce items

In the `.item` branch, the current polled source has offered an item and is now waiting for the consumer's decision.

If the consumer chooses `.get`, we request the value from the source, emit it, and submit the source back into the pool:

```par
.get => do { s.get[x] } in (x) submit(s),
```

Read it left to right:
- `s.get[x]` asks the current source for the `a`.
- `(x)` produces the item from the merged `Source<a>`.
- `submit(s)` puts the source back, so it can produce again later.

### Cancel mode: discard everything left

If the consumer chooses `.discard`, we must cancel:
1. The current source `s` (which is in our hands right now).
2. Every other source still sitting in the pool.

We discard the current source immediately:

```par
let ! = s.discard in ...
```

Then we **switch gears** using `repoll()` (with no extra clients), reusing the same pool but changing the handler to "discard everything":

```par
.discard => let ! = s.discard in repoll() {
  fan => fan.case {
    .end! => submit(),
    .spawn(l) r => submit(l, r),
    .item s => let ! = s.discard in submit(),
  }
  else => !,
}
```

This `repoll()` does not produce any `a` values. It just drains the pool by discarding sources until it's empty, and then returns `!`.

## Full code

Putting it all together:

```par
module Main

import @core/List

dec MergeSources : [<a> List<Source<a>>] Source<a>
def MergeSources = [<a> sources] poll(SourceFan(sources)) {
  fan => fan.case {
    .end! => submit(),
    .spawn(l) r => submit(l, r),

    .item s => .item case {
      .discard => let ! = s.discard in repoll() {
        fan => fan.case {
          .end! => submit(),
          .spawn(l) r => submit(l, r),
          .item s => let ! = s.discard in submit(),
        }
        else => !,
      }

      .get => do { s.get[x] } in (x) submit(s),
    }
  }

  else => .end!,
}
```

> **Labels on `poll` / `repoll` / `submit`:** You can optionally write `poll@label(...)`, `repoll@label(...)`, and `submit@label(...)`.
>
> Labels select *which poll-point* a `submit` continues at when there are multiple poll-points in scope (created by `repoll`).
>
> A missing label is itself a label: `submit(...)` targets the nearest unlabeled poll-point, while `submit@x(...)` targets a poll-point labeled `@x`.
