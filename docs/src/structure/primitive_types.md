# Primitive Types

Before taking a stroll through Par's types, let's stop by the values that are not built out of
the ordinary type connectives: the _primitives_.

Par currently has seven primitive types:

- **`Nat`** -- natural numbers, starting from zero, arbitrary size. They are a subtype of `Int`.
- **`Int`** -- positive and negative whole numbers, arbitrary size.
- **`Float`** -- IEEE-754 double-precision floating-point numbers.
- **`String`** -- UTF-8 encoded text. It is a subtype of `Bytes`.
- **`Char`** -- a single Unicode character. It is a subtype of `String`.
- **`Byte`** -- a single 8-bit value. It is a subtype of `Bytes`.
- **`Bytes`** -- a contiguous sequence of bytes.

Primitives are the one place where Par is not fully structural. User-defined types are aliases:
their shape is their meaning. Primitive types are opaque, because they need efficient runtime
representations and special operations.

## Literals

Primitive literals are always available. You do not need to import anything to write them.

```par
def Natural = 42       // Nat
def Integer = -7       // Int
def Floating = 3.14    // Float
def Text = "Hello"     // String
def Character = "H"    // Char
def OneByte = <<65>>   // Byte
def ManyBytes = <<65 66 67>>  // Bytes
```

Integer and natural number literals may use underscores for readability:

```par
def Million = 1_000_000
```

Float literals have a fractional part, and may use scientific notation:

```par
def Piish = 3.14
def Half = 0.5
def Avogadroish = 6.02e23
```

Strings use double quotes and normal escape sequences:

```par
def Greeting = "Hello\nWorld"
```

A `Char` literal is just a string literal containing exactly one character:

```par
def Letter = "a"
def Newline = "\n"
```

Byte and bytes literals use double angle brackets. A single byte literal is inferred as `Byte`;
multiple bytes, or the empty literal, are inferred as `Bytes`.

```par
def A = <<65>>
def ABC = <<65 66 67>>
def Empty = <<>>
```

Byte values are stored modulo 256, so out-of-range byte literal values wrap around.

## Including Files at Compile Time

The `include` expression embeds a file as a primitive value when the program is compiled:

```par
def HomePage = include("html/index.html")
def Logo = include("assets/logo.png")
```

The file path must be a double-quoted string literal. The path is
relative to the package root — the directory containing `Par.toml` — regardless of which source file
contains the expression. For example:

```text
my_package/
  Par.toml
  src/
    Main.par
    web/Server.par
  html/
    index.html
```

Both `Main.par` and `web/Server.par` can use `include("html/index.html")` and it will resolve to the same file.

The path must stay within the package's directory and absolute paths are not allowed. 

The compiler preserves the file byte-for-byte and checks whether it is valid UTF-8. Valid UTF-8
becomes a `String`, invalid UTF-8 becomes `Bytes`. An empty file is
valid UTF-8 and therefore becomes `String`. Since `String` is a subtype of `Bytes`, text files can
also be used wherever bytes are expected.

## Operators

Numbers are usually manipulated with operators.

```par
def Arithmetic = 1 + 2 * 3       // = 7
def Grouped = {1 + 2} * 3        // = 9
def Ratio = 22.0 / 7.0
def Difference = 10 - 3
def Negative = neg 5
```

The operators `+`, `*`, and `/` work on `Nat`, `Int`, and `Float`.
The operators `-` and `neg` work on signed numbers: `Int` and `Float`.

Comparisons work on primitive values too:

```par
def Smaller = 3 < 10          // = .true!
def SameText = "hi" == "hi"   // = .true!
def Different = "a" != "b"    // = .true!
```

They produce a `Bool`, which is described below. Comparisons also chain:

```par
def InRange = 0 <= 5 < 10
```

Chained comparisons behave as you would expect: the expression above means `0 <= 5 and 5 < 10`,
with the middle expression evaluated only once.

Reassigning versions of the arithmetic operators are additionally available in the [process syntax](../process_syntax.md), specifically `+=`, `-=`, `*=`, and `/=`:

```par
def Five = do {
  let n = 2
  n += 3  // equivalent to `let n = n + 3`
} in n
```

## Booleans

`Bool` is not a primitive type. It is an ordinary [`either`](../types/either.md) type from
`@core/Bool`:

```par
type Bool = either {
  .false!,
  .true!,
}
```

Boolean values are written as `.true!` and `.false!`.

```par
def Yes = .true!
def No = .false!
```

Boolean expressions use `and`, `or`, and `not`:

```par
def Yes : Bool = .true!
def No : Bool = .false!

def Both = Yes and No
def Either = Yes or No
def Neither = not Yes
```

These same words have extra power in conditions: they short-circuit, and can carry bindings from
matches. That is covered in [Conditions & `if`](../quality_of_life/if.md).

## Template Strings

Backtick strings are template strings. They are still `String` values, but they can contain
interpolation.

Use `${...}` to splice in an expression that already has type `String`:

```par
def Name = "Ada"
def Greeting = `Hello, ${Name}!`
```

Use `#{...}` to splice in any value that can be displayed as data:

```par
def Count = 3
def Message = `You have #{Count} messages.`
```

The `#{...}` form uses `@core/Data.ToString` under the hood. It works for primitives and for
ordinary data structures such as pairs, eithers, and lists of data.

Template strings may span multiple lines, and support the usual string escapes. To write syntax
that would otherwise start or end template behavior, escape it:

```par
def LiteralPieces = `Use \` for backticks, \${ for string interpolation, and \#{ for data.`
```

## Naming Primitive Types

Literals do not need imports, but explicit type names do.

```par
module Main

import {
  @core/Int
  @core/String
}

def Age: Int = 42
def Name: String = "Ada"
```

The same imports give access to helper functions from those modules.

```par
module Main

import {
  @core/Int
  @core/Nat
}

def Magnitude = Int.Abs(-1000)
def Remainder = Int.Mod(-13, 5)
def Numbers = Nat.Range(0, 5)  // *(0, 1, 2, 3, 4)
```

Every package automatically depends on `@core`, but its modules are not imported automatically.

## Useful Primitive Modules

The primitive modules contain operations that are not just generic arithmetic or comparison.
The examples below are only a taste; use `par doc` to browse the full built-in API.

### `@core/Nat`

`Nat` has helpers for finite repetition and natural ranges:

```par
module Main

import @core/Nat

def ThreeSteps = Nat.Repeat(3)
def ZeroToFour = Nat.Range(0, 5)
```

`Nat.Repeat(n)` produces a recursive value with exactly `n` steps. It shows up often when you need
to loop a known number of times.

### `@core/Int`

`Int` has helpers where the result type is not just "another number".

```par
module Main

import {
  @core/Nat
  @core/Int
}

def Absolute: Nat = Int.Abs(-12)
def Modulo: Nat = Int.Mod(-13, 5)
def FromTo = Int.Range(-2, 3)
```

### `@core/Float`

`Float` has constants, conversions, predicates, and math functions:

```par
module Main

import @core/Float

def Tau = 2.0 * Float.Pi
def Root = Float.Sqrt(9.0)
def Rounded = Float.Round(3.6)
def CloseEnough = Float.Equals(1.0, 1.05, 0.1)
```

`Float.Equals` remains useful because it compares with a tolerance. The `==` operator compares data
directly.

### `@core/String`

Strings can be built incrementally:

```par
module Main

import @core/String

def Hello = String.Builder
  .add("Hello")
  .add(", ")
  .add("world!")
  .build
```

They can also be parsed through `String.Parse`. The parser is a larger tool, useful when you want
to read characters or match patterns:

```par
module Main

import @core/String

def ParserFromText = String.Parse("abc")
```

### `@core/Char` and `@core/Byte`

`Char.Is` and `Byte.Is` check membership in character or byte classes:

```par
module Main

import {
  @core/Byte
  @core/Char
}

def Space = Char.Is(" ", .whitespace!)
def HighByte = Byte.Is(<<192>>, .range(<<128>>, <<255>>)!)
```

### `@core/Bytes`

`Bytes` has readers, parsers, and builders for byte-oriented protocols:

```par
module Main

import @core/Bytes

def EmptyReader = Bytes.Reader(<<>>)
def Size = Bytes.Length(<<65 66 67>>)
```

That is enough about primitives for now. The rest of the docs will introduce the structural types
that most Par programs are built from.
