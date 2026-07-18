# The Fan Pattern

We've been able to merge two lists based on the timing of their items:

```par
module Main

import @core/List

dec MergeTwoLists : [<a> List<a>, List<a>] List<a>
def MergeTwoLists = [<a> left, right] poll(left, right) {
  list => list.case {
    .end! => submit(),
    .item(x) xs => .item(x) submit(xs),
  }
  else => .end!,
}
```

And also a tree:

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

Merging multiple sources of information this way is great when you need to aggregate from multiple, slow-producing sources into a single stream, where you can react immediately to every bit produced.

**What about merging a list of lists?**

```par
dec MergeLists : [<a> List<List<a>>] List<a>
def MergeLists = ???
```

If we want to merge them in the order their items are produced, **we run into a problem with `poll`/`submit`.**

That's because a pool requires all its clients to be of the same type. But if we put a `List<List<a>>` into a pool, we get back a `List<List<a>>`. We still don't know which of the inner lists to poll from!

> We can actually solve this by combining `poll`/`submit` with `.begin`/`.loop`.
>
> ```par
> dec MergeLists : [<a> List<List<a>>] List<a>
> def MergeLists = [<a> lists] lists.begin.case {
>   .end! => .end!,
>   .item(list) lists => poll(list, lists.loop) {
>     list => list.case {
>       .end! => submit(),
>       .item(x) xs => .item(x) submit(xs),
>     }
>     else => .end!,
>   }
> }
> ```
>
> In this solution, the pool contains lists. However, we end up creating a new pool for every single list, essentially ending up with a linked (by `poll`) list of pools. It works, but is quite inefficient.

**The efficient solution** is to transform the `List<List<a>>` into a structure that is suitable for polling! We call it **the _fan_ pattern.**

## Fans: the homogeneous trees

Why does `poll` work great for `List<a>` and `Tree<a>`, but not for `List<List<a>>`? It's because both `List<a>` and `Tree<a>` are _homogeneous._ Here's what I mean:

```par
type List<a> = recursive either {
  .end!,
  .item(x) self,
}

type Tree<a> = recursive either {
  .leaf a,
  .node(self) self,
}
```

**Every `self` goes back to the same shape.** That's not true for a list of lists! In a `List<List<a>>`, the outer nodes are:

```par
recursive either {
  .end!,
  .item(List<a>) self,
}
```

While the inner nodes (the `List<a>`) are:

```par
recursive either {
  .end!,
  .item(a) self,
}
```

That's the discrepancy that prevents a clean `poll` usage.

## The _fan_ transformation

To remedy this, we need to transform a `List<List<a>>` into a more suitable structure. Here's one that will work:

```par
type ListFan<a> = recursive either {
  .end!,
  .spawn(self) self,
  .item(a) self,
}
```

There's no general prescription for designing these: it all depends on the use-case. However, these branches will appear quite frequently:

```par
  .end!,
  .spawn(self) self,
```

They allow the structure to **dynamically spread into multiple agents,** while the other branches handle the behavior of a single one: an individual list in this case.

The transformation is quite simple:

```par
dec ListFan : [<a> List<List<a>>] ListFan<a>
def ListFan = [<a> lists] lists.begin.case {
  .end! => .end!,
  .item(list) lists => .spawn(
    list.begin.case {
      .end! => .end!,
      .item(x) xs => .item(x) xs.loop,
    }
  ) lists.loop,
}
```

It consists of two nested recursions.
- The outer one produces a `.spawn` node for each list.
- The inner one produces an `.item` node for each item of a list.
- The `.end` node in the resulting fan structure is used in two ways:
  1. To say there will be no more lists.
  2. To mark an end of a single list.
  
  The consumer of the `ListFan<a>` does not need to differentiate between these two.

**Importantly, the transformation is _on-line:_** Each part of the input immediately produces a part of the output. As a result, no concurrency or timing information is lost, with Par's concurrent execution model.

**Now, we're able to implement `MergeLists` easily:**

```par
dec MergeLists : [<a> List<List<a>>] List<a>
def MergeLists = [<a> lists] poll(ListFan(lists)) {
  fan => fan.case {
    .end! => submit(),
    .spawn(l) r => submit(l, r),
    .item(x) fan => .item(x) submit(fan),
  }
  else => .end!,
}
```

> 💡 **For the theory lovers:** The above covers the [**_Client-server sessions in linear logic_**](https://dl.acm.org/doi/abs/10.1145/3473567) paper. The coexponentials introduced there can be defined as:
>
> ```par
> type Cobang = dual ListFan<dual a>
> type Coquest = List<a>
> ```
> 
> The axiom rule for them is then implemented the same way as the `MergeLists` function, except taking a `ListFan<a>` directly, instead of a `List<List<a>>`.
