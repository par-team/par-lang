# Unit

The unit type — spelled `!` — has a single value, also `!`.

```par
def Unit: ! = !
```

Unit is frequently used as an end-marker for other types. All composite types — such as [pairs](./pair.md),
[eithers](./either.md), and [choices](./choice.md) — have an obligatory _"and then"_ part. The unit type
does the job for the case of _"and then nothing"_.

For example, the predefined `List<a>` type has this definition:

```par
type List<a> = recursive either {
  .end!,
  .item(a) self,
}
```

Each variant in an [`either`](./either.md) type has an obligatory payload. For the node marking the
end of the list, the payload is empty, and so it's `!`.

## Construction

The expression `!` has type `!` and is the only possible value for this type.

```par
def Unit = !  // infers `Unit` to be of type `!`
```

## Destruction

Being a [shareable](../types_and_expressions.md#linearity) type, variables of type `!` can be left unused.

If `!` is a part of a larger type, it may be needed to assign it as a part of a pattern. For this
purpose, the pattern `!` will destruct a `!` value without assigning it to a variable.

```par
def TestUnitDestruction = do {
  let unit = !
  let ! = unit
} in !
```

This is useful when matching an end of a list:

```par
module Main

import {
  @core/Int
  @core/List
}

dec GetFirstOrZero : [List<Int>] Int
def GetFirstOrZero = [list] list.case {
  .end!      => 0,  // `!` is a pattern here
  .item(x) _ => x,
}
```

Or when destructing a `!`-ended [tuple](./pair.md):

```par
module Main

import @core/Int

dec SumPair : [(Int, Int)!] Int
def SumPair = [pair]
  let (x, y)! = pair  // `!` is a pattern here
  in x + y

def Five =
  let pair = (2, 3)!
  in SumPair(pair)
```
