# Polling & Submitting

How does Par solve this server-client nondeterminism, i.e. a concurrency structure with independent client agents all communicating with a central server agent?

Par does it by introducing a single feature: **the `poll`/`submit` control structure.**

There are no new types; the existing ones are wholly sufficient for specifying communication between the server and client counterparts.

## The idea: Pools

Before we dive into the syntax, it's good to understand the overall idea. Under the surface, the `poll`/`submit` control structure works around _pools of clients._ You can imagine a pool as just a space for objects to hang out. We call them _agents_ or _clients_, but they're really just any values.

A single pool has all its clients of the same type.

They hang out there until they're **ready to take an action,** that is:
- Sending a value, for a [pair type](../types/pair.md).
- Receiving a value, for a [function type](../types/function.md).
- Making a choice, for an [either type](../types/either.md).
- Responding to a choice, for a [choice type](../types/choice.md).

A client ready to take an action is then **polled out of the pool** and made available to the poller: the server process. Then once the client is served, zero or more clients can be **submitted back to the pool,** and the polling continues.

## `poll` / `submit`

Let's first take a look at the expression version because it's more natural to work with most of the time.

Here's the general syntax for a `poll` expression:

```par
poll(<client-value>, ...) {
  <client-variable> => <result-for-non-empty-pool>

  else => <result-after-empty-pool>
}
```

The `poll` keyword is followed by a **list of initial clients** enclosed in parentheses. It must contain at least one initial client.

Then, inside curly braces, we have exactly two branches:
- **The active branch:** Once a client becomes ready, this branch starts and binds the client to the `<client-variable>`. This branch **must call `submit`.**
- **The `else` branch:** Once the entire pool is depleted, the `else` branch is entered to produce the final result.

**Inside the active branch, we must call `submit` exactly once.**

```par
submit(<client-value>, ...)
```

Unlike `poll`, it can take zero clients:

```par
submit()
```

It puts the clients back in the pool, and goes back to polling again.

The `submit` expression then returns the overall result of its corresponding `poll` after this point. It's all very similar to [`.begin`/`.loop`](../types/recursive.md#destruction). Even the treatment of local variables is the same: local variables used by the `poll` get automatically moved to the next `poll` iteration at `submit`.

**There are two important restrictions:**
- The client type must be [`recursive`](../types/recursive.md).
- `submit` is only allowed on **descendants** of the client in the active branch. This ensures progress: you can't loop forever by putting the same client back into the pool unchanged.

Let's make it all make sense with some examples.

## Simple examples

The simplest way to start is to recreate some functions you'd normally implement with `.begin`/`.loop`, but use `poll`/`submit` instead.

> `poll`/`submit` is not a replacement for `.begin`/`.loop`. One can't achieve the job of the other and vice versa, only in some situations.

**Let's compute the sum of a list:**

```par
module Main

import {
  @core/Int
  @core/List
}

dec PollSum : [List<Int>] Int
def PollSum = [nums] poll(nums) {
  list => list.case {
    .end! => submit(),
    .item(x) xs => x + {submit(xs)},
  }
  else => 0,
}
```

Breaking it down:
- `poll(nums)`
  
  Creates a pool containing a single client: `nums`.
- `list =>`
  
  The client is activated and assigned to a new variable: `list`.
- `list.case`
  
  Just a normal [`.case`](../types/either.md) expression.
- `.end! => submit()`
  
  If the list is empty, we do an empty `submit()`. That returns the result of the rest of the next `poll`, which in this case will go to the `else` branch.
- `.item(x) xs => x + submit(xs),`
  
  If the list is not empty, we `submit(xs)` the remainder of the list back into the pool, and return the sum of the first item with the result of the next `poll`.

That's very similar to what we'd do with `.begin`/`.loop`:

```par
dec Sum : [List<Int>] Int
def Sum = [nums] nums.begin.case {
  .end! => 0,
  .item(x) xs => x + xs.loop,
}
```

Instead of `xs.loop`, we have `submit(xs)`, but there's another important difference:

In `.begin`/`.loop`, we know that we're done in the `.end!` branch. For a pool, the end of a client does not mean the end of the pool. That's why the `.end!` branch still needs to `submit`!

Indeed, the `PollSum` function can be trivially extended to add up all the numbers from two lists!

```par
dec PollSumTwo : [List<Int>, List<Int>] Int
def PollSumTwo = [nums1, nums2] poll(nums1, nums2) {
  list => list.case {
    .end! => submit(),
    .item(x) xs => x + {submit(xs)},
  }
  else => 0,
}
```

All we did was change this:

```par
def PollSum = [nums] poll(nums) {
```

To this:

```par
def PollSumTwo = [nums1, nums2] poll(nums1, nums2) {
```

Everything else remains the same.

The empty `submit()` in the `.end!` branch now makes sense! Just because one of the lists has ended doesn't mean there's no more clients: there's the other list!

**Let's now merge two lists into one:**

```par
dec MergeTwoLists : [<a> List<a>, List<a>] List<a>
def MergeTwoLists = [<a> left, right] poll(left, right) {
  list => list.case {
    .end! => submit(),
    .item(x) xs => .item(x) submit(xs),
  }
  else => .end!,
}
```

It's the same principle, but instead of adding numbers, we're building a resulting list.

> Note that the order of items in the resulting list will not be fully arbitrary! The remainder of each list enters the pool only after the item before it gets produced, so for example:
>
> ```par
> MergeTwoLists(*(1, 2, 3), *(4, 5, 6))
> ```
>
> The result can be any of these:
> - `*(1, 4, 2, 5, 3, 6)`
> - `*(1, 2, 3, 4, 5, 6)`
> - `*(4, 5, 6, 1, 2, 3)`
>
> But not `*(6, 5, 4, 3, 2, 1)`!

**What about nondeterministically turning a tree into a list?**

```par
type Tree<a> = recursive either {
  .leaf a,
  .node(self) self,
}
```

Works like a charm:

```par
dec TreeToList : [<a> Tree<a>] List<a>
def TreeToList = [<a> tree] poll(tree) {
  tree => tree.case {
    .leaf x => .item(x) submit(),
    .node(l) r => submit(l, r),
  }
  else => .end!,
}
```

The order of the resulting list only depends on the time when the leaf nodes come into existence.

This is the first time we see `submit` with more than one client:

```par
    .node(l) r => submit(l, r),
```

A `.loop` can be used any number of times inside a single `.begin`, but is always applied to exactly one value. On the other hand, `submit` must be used exactly once inside a `poll`, but it can be applied to any number of values.

## Process syntax

While all of the examples in this chapter use expression syntax, `poll`/`submit` is perfectly available in process syntax as well.

```par
// inside a process
poll(client1, client2) {
  client => {
    // ... handle the client
    submit(client)
  }

  else => {
    // ... handle the rest of the process
  }
}
```

Here, `poll` and `submit` are control-flow statements, so they don't "return" anything. The `submit` statement simply jumps back to its `poll`, in addition to putting clients back into the pool.
