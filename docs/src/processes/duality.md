# Construction by Destruction

We've now seen the `chan` expression in action, using the obtained channel to return early
in its process. But that only scratches the surface!

The channel you obtain isn't just a return handle — it's a **direct connection to the consumer** of the result.
It can be interacted with sequentially, constructing a value step-by-step.

The title of this section, _"construction by destruction",_ is very apt! What we're about to
learn here is exactly analogous to the famous trick in mathematics: _a proof by contradiction._
And, it's just as powerful.

## Duality in theory

Every type in Par has a **dual** — the opposite role in communication.

There is a type operator, `dual <type>`, which transforms a type to its dual. Here’s how
it's defined structurally:

<table>

<tr/>
<tr>
<td><code class="language-par">dual !</code></td>
<td><strong>＝</strong></td>
<td><code class="language-par">&#63;</code></td>
</tr>

<tr/>
<tr>
<td><code class="language-par">dual &#63;</code></td>
<td><strong>＝</strong></td>
<td><code class="language-par">!</code></td>
</tr>

<tr/>
<tr>
<td><code class="language-par">dual (A) B</code></td>
<td><strong>＝</strong></td>
<td><code class="language-par">[A] dual B</code></td>
</tr>

<tr/>
<tr>
<td><code class="language-par">dual [A] B</code></td>
<td><strong>＝</strong></td>
<td><code class="language-par">(A) dual B</code></td>
</tr>

<tr/>
<tr>
<td><pre><code class="language-par">dual either {
  .left A,
  .right B,
}</code></pre></td>
<td><strong>＝</strong></td>
<td><pre><code class="language-par">choice {
  .left => dual A,
  .right => dual B,
}</code></pre></td>
</tr>

<tr/>
<tr>
<td><pre><code class="language-par">dual choice {
  .left => A,
  .right => B,
}</code></pre></td>
<td><strong>＝</strong></td>
<td><pre><code class="language-par">either {
  .left dual A,
  .right dual B,
}</code></pre></td>
</tr>

<tr/>
<tr>
<td><code class="language-par">dual recursive F&lt;self&gt;</code></td>
<td><strong>＝</strong></td>
<td><code class="language-par">iterative dual F&lt;dual self&gt;</code></td>
</tr>

<tr/>
<tr>
<td><code class="language-par">dual iterative F&lt;self&gt;</code></td>
<td><strong>＝</strong></td>
<td><code class="language-par">recursive dual F&lt;dual self&gt;</code></td>
</tr>

<tr/>
<tr>
<td><code class="language-par">dual [type a] F&lt;a&gt;</code></td>
<td><strong>＝</strong></td>
<td><code class="language-par">(type a) dual F&lt;a&gt;</code></td>
</tr>

<tr/>
<tr>
<td><code class="language-par">dual (type a) F&lt;a&gt;</code></td>
<td><strong>＝</strong></td>
<td><code class="language-par">[type a] dual F&lt;a&gt;</code></td>
</tr>

</table>

> Don’t worry if the `F<...>` in recursive and generic types looks intimidating. It's just a way to
> formalize that after flipping from `recursive` to `iterative` (and similarly in the other cases),
> we continue dualizing the body.”.

Looking at the table, here's what we can see:
- [Units](../types/unit.md) are dual to [continuations](../types/continuation.md).
- [Functions](../types/function.md) are dual to [pairs](../types/pair.md).
- [Eithers](../types/either.md) are dual to [choices](../types/choice.md).
- [Recursives](../types/recursive.md) are dual to [iteratives](../types/iterative.md).
- [Foralls](../types/forall.md) are dual to [existentials](../types/exists.md).
- **The duality always goes both ways!**

The last point is important. It's a fact, in general, that `dual dual A` is equal to `A`.

[Cleanup markers](../types/auto_cleanup.md) are preserved by duality too:

```par
dual choice {
  .cancel* => !,
}
```

becomes

```par
either {
  .cancel* ?,
}
```

The choice side promises that `.cancel` is safe to select automatically. The either side carries
the same marker so that its consumer is forced to register the cleanup branch at runtime (by marking it with a star inside a `.case`), to be potentially selected by auto-cleanup.

## Duality in action

Here’s a familiar definition:

```par
type List<a> = recursive either {
  .end!,
  .item(a) self,
}
```

So, what's its dual? Applying the above rules, we get:

```par
iterative choice {
  .end => ?,
  .item(a) => self,
}
```

While a `List<a>` provides items, until it reaches the end, its dual requires _you_ to
give items (via the `.item` branch), or signal the end. Whoever is consuming a list,
this dual type provides a good way to communicate with them.

Notice the `?` in the `.end` branch. That's a **continuation.** There is no expression syntax for
this type, but finally, we're going to learn how to handle it in processes!

Remember: in a `chan`, the channel you obtain inside the block always has the **dual type** of the expression’s
final result. Let's use this to construct a list step-by-step.

```par
module Main

import {
  @core/Int
  @core/List
}

def SmallList: List<Int> = chan yield {
  yield.item(1)
  yield.item(2)
  yield.item(3)
  yield.end
  yield!
}

// def SmallList = *(1, 2, 3)
```

This is just a usual handling of an [iterative](../types/iterative.md)
[choice](../types/choice.md), except for the last line.

Selecting the `.end` branch transforms the `yield` channel into a `?` — the continuation type. At that point,
the protocol is over, and only one command is valid: **a break.** It's spelled `!`, that's the last line:

```par
  yield!  // break here
```

Like [linking](./chan_expression.md#linking--the--command), it has to be the last command in
a process. In fact, **link and break are the only ways to end a process.**

## A real example: flattening a list of lists

Let’s now use this style to implement something meaningful. We’ll write a function that flattens
a nested list:

```par
dec Flatten : [type a, List<List<a>>] List<a>
```

It's a generic function, using the [forall](../types/forall.md) for polymorphism.

Here's the full implementation, imperative-style:

```par
def Flatten = [type a, lists] chan yield {
  lists.begin@outer.case {
    .end! => {
      yield.end!
    }

    .item(list) => {
      list.begin@inner.case {
        .end! => {}
        .item(value) => {
          yield.item(value)
          list.loop@inner
        }
      }
      lists.loop@outer
    }
  }
}
```

It makes a bunch of concepts we've already covered come together.

- We loop through the outer list of lists using `.begin@outer` and `.loop@outer`.
- If it’s `.end!`, we finish: `yield.end!`.
- If it’s `.item(list)`, we then begin looping through the inner list.
  - We don't manually bind the remainder of the list — the communication simply continues on
    the original variable: `lists`.
  - The nested `.begin`/`.loop` shines here; no helper functions needed.
- In the inner loop:
  - If it’s `.end!`, we’re done with that list — we continue the outer loop.
  - If it’s `.item(value)`, we yield it to the consumer with `yield.item(value)`, then loop again.
    Re-binding of the rest of the list is not needed here, either.

Duality combined with the `chan` expressions gives us a lot of expressivity.\
Constructing lists generator-style is just one of the use-cases.\
Whenever it feels more appropriate to _become a value_ instead of constructing it from parts, `chan` and
duality come to the task!
