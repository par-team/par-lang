# Definitions & Declarations

Let's keep working with the package created by `par new`.

Its `src/Main.par` file starts like this:

```par
module Main
```

There may also be imports above the rest of the code, but we're going to ignore those for a moment
and focus only on what comes after them.

At the top level of a Par module, you write:

- **definitions**
- **declarations**
- **type definitions**

These define **global names** that can be used throughout the module, an unlimited number of times.

In a fresh package, `par run` looks for the `Main` definition in the `Main` module, that is
`Main.Main`.

So:

- `par run` means `Main.Main`
- `par run Main.Other` means the `Other` definition in the `Main` module

Once there are more modules, the same idea extends to paths such as `par run util/Parse.Program`.
We'll come back to that in the next section.

> Because of Par's linear type system, local variables may be required to be used _exactly once._ That is, if they have a strictly linear type.
> Droppable linear variables may instead be cleaned up when left unused, while shareable variables
> may be used freely. Global definitions can be used any number of times regardless of their type.

Par has a simple naming rule:
- **Global names start with an upper-case letter.** That is global types, functions, and so on.
- **Local names start with a lower-case letter, or `_`.** That includes local variables, function
  parameters, and type variables in generic functions.

While global names can be used throughout their module, there is an _important restriction!_

> ❗ **Cyclic usages are forbidden!** Both in types, and in definitions.
>
> That means that if a type `Alice` uses a type `Bob`, then `Bob` can't use `Alice`. Same for functions,
> and other definitions. In fact, `Alice` can't use `Alice` either!

This apparently mad restriction has important motivations, and innovative remedies.

**The motivation** is Par's ambitious stride towards **totality** — which means preventing
infinite loops.

Unrestricted recursion is a source of infinite loops, and while that can be partially
remedied by totality checkers, such as in [Agda](https://en.wikipedia.org/wiki/Agda_(programming_language)),
Par chooses a different approach. That is outlawing unrestricted recursion, and instead relying on more
_principled_ ways to achieve cyclic behavior, including what's usually achieved by mutual recursion.

**The remedies** come in the form of these more principled ways. Fortunately, they don't just replace the
familiar recursion by clunkier mechanisms, they bring their own perks.

Naive recursion on the term level (like in functions) is replaced by a powerful,
**universal looping mechanism, called `begin`/`loop`.** It's a single tool usable for:
- _[Recursive](../types/recursive.md) reduction._ Analyzing lists, trees, or even files.
- _[Iterative](../types/iterative.md) construction_. Those are objects that can be interacted
  with repeatedly.
- _Imperative-looking loops_ in [process syntax](../process_syntax.md).

Naive recursion in types is replaced by
**anonymous [recursive](../types/recursive.md) and [iterative](../types/iterative.md) (corecursive) types.**

## Definitions

Global values (including functions) are defined at the top level starting with the keyword `def`,
followed by an upper-case name, an `=` sign, and an expression computing the value.

```par
module Main

def MyNumber = 7
```

In this case, Par is able to infer the type of `MyNumber` as [`Nat`](./primitive_types.md)
(a natural number), so no type annotation is needed. Often, a type annotation is needed, or wanted.
In those cases, we can add it using a colon after the name:

```par
def MyName: String = "Michal"
```

## Declarations

Sometimes a type is longer and a definition becomes busy and hard to read with it.

For example, here's a simple function adding up all the numbers in a list:

```par
def SumList: [List<Int>] Int = [list] list.begin.case {
  .end!       => 0,
  .item(x) xs => x + xs.loop,
}
```

The code uses many concepts that will be covered later, so only focus on the parts you know: `def`,
`:`, and `=`. _(You can run it in the playground, though!)_

In such a case, the type annotation can be extracted into a separate _declaration_. A declaration
starts with the keyword `dec`, followed by the name we want to annotate, a colon, and a type.

```par
dec SumList : [List<Int>] Int

def SumList = [list] list.begin.case {
  .end!       => 0,
  .item(x) xs => x + xs.loop,
}
```

That's much better!

Declarations may be placed anywhere in a file, so feel free to put them all on top, or keep them close
to their corresponding definitions.

## Type Definitions

Par has a [structural](../introduction.md#orthogonality-goes-wide-not-deep) type system. While many languages offer multiple forms of type definitions
— for example, Rust has `struct`, `enum`, and more — Par only has one: **type aliases.**

> With [recursive](../types/recursive.md) and [iterative](../types/iterative.md) types being anonymous,
> Par has no issue treating types as their shapes, instead of their names. In fact, type definitions are
> completely redundant in Par. Every usage of a global type (with the exception of the
> [primitives](./primitive_types.md)) can be replaced by its definition, until no definitions are
> used.
>
> _Actually, all definitions are redundant in Par. However, programming without them would be quite tedious._

To give a name to a type, use the `type` keyword at the top level, followed by an upper-case name,
an `=` sign, and a type to assign to it.

```par
type MyString = String
```

`MyString` is now equal to `String` and can be used wherever `String` can.

A more useful example:

```par
type StringBuilder = iterative choice {
  .build => String,
  .add(String) => self,
}
```

> This particular type is a part of the built-in functionality, under the name `String.Builder`.

That's an [iterative](../types/iterative.md) [choice](../types/choice.md) type, something we will learn
later. It's an object that can be interacted with repeatedly, choosing a branch (a method) every time.

While we could paste the entire definition every time we would use this `StringBuilder`, it's quite
clear why we wouldn't want to do that.

### Generic types

A type definition may include generic type parameters, turning it into a formula that can be instantiated
with any types substituted for the parameters.

The type parameters are specified in a comma-separated list inside angle brackets right after the
type name. The parameters are local names, so they must be lower-case.

For example, the built-in `List` type has one type parameter:

```par
type List<a> = recursive either {
  .end!,
  .item(a) self,
}
```

That's a [recursive](../types/recursive.md) [either](../types/either.md) type, also something we will
learn later. In this case, it defines a finite, singly-linked list of items of type `a`.

To use a generic type, we append a comma-separated list of specific type arguments enclosed in
angle brackets after the type's name.

```par
type IntList = List<Int>
```

The resulting type is obtained by replacing each occurrence of each type variable by its corresponding
type argument. So, the above is equal to:

```par
type IntListExpanded = recursive either {
  .end!,
  .item(Int) self,
}
```

Now that we know what goes inside a module, let's zoom out and look at **packages, modules, imports,
and exports.**
