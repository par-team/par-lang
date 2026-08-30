use super::lexer::TokenKind;

pub(crate) struct KeywordDocumentation {
    pub name: &'static str,
    pub markdown: &'static str,
}

pub(crate) fn documentation(kind: TokenKind) -> Option<KeywordDocumentation> {
    match kind {
        TokenKind::Let => Some(KeywordDocumentation {
            name: "let",
            markdown: r#"Bind a value to local variables.

`let` introduces bindings from a pattern. A pattern can be a lower-case variable, destructure a pair into several variables, annotate individual variables, or include `try` and `default(...)` steps. Each binding is available only in the part of the program that follows it.

In an expression, `let` evaluates its value and then evaluates the expression after `in` with the new bindings in scope:

```par
let three = 3 in three + three
```

In a process, a `let` statement has no `in`; its bindings remain available to the following process steps. On a path that falls through a `do { ... } in expression`, those bindings are also available to `expression` after `in`. This is the idiomatic way to introduce several intermediate values:

```par
do {
  let sum = left + right
  let message = `Sum: #{sum}`
} in message
```

A binding may have a type annotation after its variable name. Otherwise, Par infers the type when possible:

```par
def Six = let three: Nat = 3 in three + three
def Twelve = let (a, b)! = (3, 4)! in a * b
```

Annotations belong on the variables inside a pattern, not on the pattern as a whole. For example, write `(a: Nat, b: Nat)!`, not `(a, b)! : (Nat, Nat)!`.

Within a `let` pattern, `default(fallback)` unwraps an `Option`, binding its `.some` payload or the fallback for `.none!`:

```par
def ReadOrZero: [Option<Nat>] Nat = [option]
  let default(0) value = option in value
```

Within a matching `catch`, a `try` pattern unwraps a `Try` value and transfers its error to that catch:

```par
catch error => fallback in
let try value = result in value
```

[Let expressions](https://par.run/book/structure/let_expressions) | [Error handling](https://par.run/book/quality_of_life/error_handling)"#,
        }),
        TokenKind::In => Some(KeywordDocumentation {
            name: "in",
            markdown: r#"Continue an expression after establishing a local scope.

`in` separates the setup part of an expression from the expression that follows it. The expression after `in` is evaluated with the bindings, process result, handler, or type context established before it.

`in` appears in four expression forms:

```par
let pattern = value in expression
do { process } in expression
catch pattern => handler in expression
type T in expression
```

With `let`, names bound by the pattern are available after `in`:

```par
let three = 3 in three + three
```

With `do`, Par first runs the process block, then evaluates and returns the expression after `in`. Process `let` statements deliberately have no `in`; their bindings are available to later process steps and, on a fallthrough path, to the expression after `in`:

```par
do {
  let sum = left + right
  let message = `Sum: #{sum}`
} in message
```

With `catch`, the handler before `in` handles a matching `throw` in the expression after `in`. A label may make the matching catch explicit. With `type`, the type before `in` ascribes a type to the following expression, which can guide type inference:

```par
catch error => .err error in
throw "failed"
```

For example, `*()` is an empty list, so it contains no value from which Par can infer its item type. `List.Concat` expects a list of lists; ascribing `*()` as `List<List<Nat>>` tells Par that concatenating it produces an empty `List<Nat>`:

```par
def EmptyNaturals: List<Nat> =
  List.Concat(type List<List<Nat>> in *())
```
[Let expressions](https://par.run/book/structure/let_expressions) | [Do expressions](https://par.run/book/processes/do_expression) | [Error handling](https://par.run/book/quality_of_life/error_handling) | [Implicit generics](https://par.run/book/types/implicit_generics)"#,
        }),
        TokenKind::Begin => Some(KeywordDocumentation {
            name: "begin",
            markdown: r#"Establish a recursion or corecursion point.

`begin` pairs with `loop`. A `loop` returns to its matching loop point and starts the next iteration. At a normal `begin`, captured local variables must still be defined and assignable to their begin-time types when looping; a recursive `.loop` must also use a descendant of the subject opened by `.begin`.

Use `.begin` after a value of a `recursive` type to unfold that value for a recursive reduction. The descendant value in a `self` position can then use `.loop` to repeat the same body:

```par
dec SumList : [List<Nat>] Nat
def SumList = [list] list.begin.case {
  .end!       => 0,
  .item(x) xs => x + xs.loop,
}
```

In process syntax, `.begin` is the same operation in a command chain. It can be followed by commands such as `.case`:

```par
strings.begin.case { /* branches are processes */ }
```

Use standalone `begin` to construct a value of an `iterative` type. Its body describes one observable step; standalone `loop` supplies the next version of that value:

```par
type Sequence<a> = iterative choice {
  .close => !,
  .next  => (a) self,
}

def SevenForever: Sequence<Int> = begin case {
  .close => !,
  .next  => (7) loop,
}
```

When `begin` points are nested, label the matching pair explicitly with `begin@label` and `loop@label` (or `.begin@label` and `.loop@label`). `unfounded` uses the same forms but skips the descendant proof used by normal `begin`; it still requires a matching loop point and compatible captured variables. Prefer `begin` whenever the checker can prove the required descent or productivity.

[Recursive types](https://par.run/book/types/recursive) | [Iterative types](https://par.run/book/types/iterative) | [Looping and branching](https://par.run/book/processes/commands/looping_and_branching)"#,
        }),
        TokenKind::Loop => Some(KeywordDocumentation {
            name: "loop",
            markdown: r#"Continue from a matching recursion or corecursion point.

`loop` pairs with `begin`. It returns to the matching loop point and starts the next iteration. Use it only after a corresponding `begin` has established that point. For recursive reduction, `.loop` must use a descendant of the subject opened by the matching `.begin`; captured variables must remain defined with types assignable to their begin-time types.

For a recursive reduction, use `.loop` on a descendant of the recursive value opened with `.begin`. In this list sum, `xs` occupies the recursive `self` position, so `xs.loop` continues the reduction with the remainder of the list:

```par
dec SumList : [List<Nat>] Nat
def SumList = [list] list.begin.case {
  .end!       => 0,
  .item(x) xs => x + xs.loop,
}
```

In process syntax, `.loop` terminates the current command chain and returns the command subject to its matching `.begin`:

```par
strings.begin.case {
  .end! => {}
  .item(str) => {
    builder.add(str)
    strings.loop
  }
}
```

For iterative construction, use standalone `loop` to supply the next version of the value being built. Here every `.next` request produces `7` and loops back to construct the next `Sequence<Int>`:

```par
type Sequence<a> = iterative choice {
  .close => !,
  .next  => (a) self,
}

def SevenForever: Sequence<Int> = begin case {
  .close => !,
  .next  => (7) loop,
}
```

Nested loop points need labels: `loop@label` targets `begin@label`, and `.loop@label` targets `.begin@label`. `loop` also pairs with `unfounded`; it keeps the matching-point and variable-preservation rules but skips the normal descendant proof.

[Recursive types](https://par.run/book/types/recursive) | [Iterative types](https://par.run/book/types/iterative) | [Looping and branching](https://par.run/book/processes/commands/looping_and_branching)"#,
        }),
        TokenKind::As => Some(KeywordDocumentation {
            name: "as",
            markdown: r#"Give an imported module a file-local alias.

Use `as` in an `import` to choose the module name used in this file. Without `as`, the imported module's declared name is its alias. An explicit alias is useful when two imported modules have the same name or when a shorter name makes the use site clearer.

```par
import @dep1/blah/Data as Data1
import @dep2/bleh/Data as Data2
```

An alias must be an uppercase, module-style identifier. Use it to qualify exported members:

```par
import @core/List as L

def Length = L.Length(*(1, 2, 3))
```

`as` also works for entries in grouped imports:

```par
import {
  @basic/Console
  @core/List as L
}
```

Aliases are local to the importing file. They must be unique, and top-level `type`, `dec`, and `def` names cannot reuse an imported alias. Aliasing does not change module visibility or dependency requirements.

[Packages and modules](https://par.run/book/structure/packages_and_modules)"#,
        }),
        TokenKind::Box => Some(KeywordDocumentation {
            name: "box",
            markdown: r#"Make a type or expression non-linear.

Par tracks linear values: a linear value must be used exactly once. `box` makes a type or expression non-linear, so the resulting value may be copied, reused, or dropped.

The clearest use is a function that must be called more than once. `Twice` applies its `f` parameter twice: first to `value`, then to the first result. The type `box [Nat] Nat` says that `f` is a reusable function from `Nat` to `Nat`; without `box`, `f` would be linear and the second use would not be allowed.

```par
import @core/Nat

def Twice: [box [Nat] Nat, Nat] Nat = [f, value]
  f(f(value))

def Eight = Twice(box [n] n * 2, 2)
```

`box [n] n * 2` constructs the reusable function passed as `f`; it does not double anything at that point. `Twice` later calls that function with `2`, gets `4`, calls it again with `4`, and produces `8`.

More generally, `box T` is the non-linear form of `T`, and `box expression` constructs a value of that type. A boxed value may be copied, reused, or dropped.

A `box` expression may capture only local variables whose types satisfy the `box` constraint. This includes data types, existing boxed values, and generic values known to satisfy that constraint.

In a generic binder, `a: box` is a constraint rather than a boxed type. It requires `a` to be non-linear, allowing generic code to copy, reuse, or discard values of that type. Both explicit `type a: box` binders and implicit `<a: box>` binders can carry the constraint:

```par
dec DuplicateExplicit : [type a: box, a] (a, a)!
def DuplicateExplicit = [type a: box, value] (value, value)!

dec Duplicate : [<a: box> a] (a, a)!
def Duplicate = [<a: box> value] (value, value)!
```

A `box T` can be used where `T` is expected. If `T` is already non-linear, it can also be used where `box T` is expected.

[Box types](https://par.run/book/types/box) | [Type constraints](https://par.run/book/types/constraints)"#,
        }),
        TokenKind::Module => Some(KeywordDocumentation {
            name: "module",
            markdown: r#"Declare the module defined by this source file.

Every package source file declares an uppercase module name:

  ```par
  module Main
  ```

  Use `export module` when dependent packages may import it:

  ```par
  export module Greeter
  ```

  The file name must match that name case-insensitively: `src/Main.par` declares `module Main`. Directories contribute to an import path, not to the declared name; for example, `src/data/Post.par` declares `module Post` and is imported as `data/Post`.

  A module is always visible to other modules in its own package. Files split into one multi-file module must all declare the same name and agree on `export module`.

  [Packages and modules](https://par.run/book/structure/packages_and_modules)"#,
        }),
        TokenKind::Include => Some(KeywordDocumentation {
            name: "include",
            markdown: r#"Embed a file's contents as a primitive value at compile time.

`include("path")` reads a regular file during compilation and places its contents in the program:

```par
def HomePage = include("html/index.html")
def Logo = include("assets/logo.png")
```

The path must be a double-quoted string literal relative to the package root, the directory containing `Par.toml`; it is not relative to the source file. Therefore, source files at different paths can include the same package file:

```text
my_package/
  Par.toml
  src/
    Main.par
    web/Server.par
  html/
    index.html
```

Both `src/Main.par` and `src/web/Server.par` can use `include("html/index.html")`. Absolute paths and paths that resolve outside the package are rejected. The target must be a regular file.

The compiler preserves the file bytes. Valid UTF-8 becomes a `String`; invalid UTF-8 becomes `Bytes`. An empty file is valid UTF-8 and therefore becomes `String`. Because `String` is a subtype of `Bytes`, included text can also be used where bytes are expected.

[Primitive types](https://par.run/book/structure/primitive_types)"#,
        }),
        TokenKind::Import => Some(KeywordDocumentation {
            name: "import",
            markdown: r#"Make another module available in this file.

An import uses an absolute path from `src/`, or an `@dependency` alias followed by a path. The three forms are a local import, a dependency import with an optional uppercase alias, and a grouped import:

```par
import data/Post
import @core/List
import @dep1/blah/Data as Data1

import {
  @basic/Console
  @core/List
  data/Post
}
```

Imported items stay qualified by the module name or alias:

```par
import @core/List

def Count = List.Length(*(1, 2, 3))
```

Relative paths are not supported. Use `as` to give an import a different uppercase module alias; aliases must be unique in the file and cannot conflict with top-level names.

[Packages and modules](https://par.run/book/structure/packages_and_modules)"#,
        }),
        TokenKind::Export => Some(KeywordDocumentation {
            name: "export",
            markdown: r#"Make a module, type, or declaration visible outside its default scope.

`export module` makes a module visible to dependent packages. `export type` and `export dec` make individual API items visible. Braces group several type and declaration exports:

```par
export module Greeter

export type Greeting = !
export dec Greet : Greeting
```

Grouped exports are an alternative form:

```par
export {
  type Greeting = !
  dec Greet : Greeting
}
```

Types and declarations are module-private by default. There is no `export def`: a `def` is the implementation of a value, while an exported `dec` publishes its signature. An exported item in a non-exported module is visible within its package but not to dependent packages.

[Packages and modules](https://par.run/book/structure/packages_and_modules)"#,
        }),
        TokenKind::Type => Some(KeywordDocumentation {
            name: "type",
            markdown: r#"Introduce or pass a type in one of Par's type-level forms.

At the top level, `type` creates a named structural type alias. Its optional parameters are lower-case and unconstrained type variables:

```par
type Pair<a, b> = (a, b)!
```

Inside a function or pair type, `type a` is an explicit generic binder. It may carry a constraint; the same parameter can be bound in the implementation:

```par
dec Duplicate : [type a: box, a] (a, a)!
def Duplicate = [type a: box, value] (value, value)!
```

At a call or send, `type T` supplies an explicit generic argument:

```par
Swap(type String, type Int, pair)
```

In a pair type, `type a` binds an existential type whose value follows it:

```par
type SomeData = (type a: data) a
```

Use `type T in expression` to ascribe a type locally, often to guide inference for an otherwise ambiguous expression:

```par
def EmptyNaturals: List<Nat> =
  List.Concat(type List<List<Nat>> in *())
```

Implicit generic binders use `<a: constraint>` instead, as in `[<a: data> a] a`. Type aliases use unconstrained parameters; place constraints such as `type a: data` or `<a: box>` on the functions that need them.

[Definitions and declarations](https://par.run/book/structure/definitions_and_declarations) | [Forall](https://par.run/book/types/forall) | [Implicit generics](https://par.run/book/types/implicit_generics) | [Type constraints](https://par.run/book/types/constraints)"#,
        }),
        TokenKind::Dec => Some(KeywordDocumentation {
            name: "dec",
            markdown: r#"Declare the type of a top-level value.

A declaration gives an uppercase global name a type, usually before or near its implementation:

```par
dec IdentityUnit : [!] !
def IdentityUnit = [unit] unit
```

Use `dec` when a signature is clearer on its own or is part of a module's public API. A declaration can be exported with `export dec`; its matching `def` supplies the implementation. Writing `def Name: T = expression` is the compact form: the annotation also provides the declaration.

[Definitions and declarations](https://par.run/book/structure/definitions_and_declarations) | [Packages and modules](https://par.run/book/structure/packages_and_modules)"#,
        }),
        TokenKind::Def => Some(KeywordDocumentation {
            name: "def",
            markdown: r#"Define a top-level value or function.

A definition has an uppercase name, an optional type annotation, `=`, and either a Par expression or `external`:

```par
def Answer = 42
def IdentityUnit: [!] ! = [unit] unit
```

The annotation in `def Name: T = expression` also acts as the value's declaration. Use a separate `dec Name : T` when the signature should stand apart from the implementation.

Definitions are the implementation side of a module API. There is no `export def`; export the corresponding `dec` to make a value's signature visible from another module.

[Definitions and declarations](https://par.run/book/structure/definitions_and_declarations) | [Packages and modules](https://par.run/book/structure/packages_and_modules)"#,
        }),
        TokenKind::External => Some(KeywordDocumentation {
            name: "external",
            markdown: r#"Mark a definition as implemented by the host rather than by Par source.

`external` is permitted only as the body of a `def`:

```par
dec Open : !
def Open = external
```

The definition still needs a type, supplied by a `dec` or a `def` annotation. The embedding runtime or built-in package must attach an implementation for it; `external` is not a general-purpose placeholder for unfinished Par code. Use `todo` while writing an incomplete program.

Par's built-in packages use this form for host services such as operating-system handles."#,
        }),
        TokenKind::Either => Some(KeywordDocumentation {
            name: "either",
            markdown: r#"Define a tagged union: one value is one named variant.

An `either` type lists lower-case variants, each with one required payload type:

```par
type Result = either {
  .ok!,
  .err String,
}
```

Construct a value with `.variant payload`, then inspect it with `.case`:

```par
def Describe = [result] result.case {
  .ok! => "ok",
  .err message => message,
}
```

Every variant has exactly one payload. Write `!` for a unit payload: `.ok!` carries no additional data, but still distinguishes the `.ok` variant from every other variant. When a variant needs several values, make its single payload a pair. For example, `.both(Int, String)!` carries an `Int` and a `String`, is constructed as `.both(42, "answer")!`, and can be matched as `.both(number, text)!`.

Use `either` when a value can be one of a finite set of named alternatives. The code that constructs the value chooses its variant; the code that receives it learns that choice by matching with `.case`. This differs from `choice`, where a value offers operations and its consumer selects one with postfix `.branch`.

An `either` is also the required guard around `self` in a `recursive` type: a recursive reference must occur inside a variant payload, so each recursive layer is revealed only by choosing a variant, such as `.end!` or `.item self` in a list.

[Either](https://par.run/book/types/either) | [Recursive types](https://par.run/book/types/recursive)"#,
        }),
        TokenKind::Choice => Some(KeywordDocumentation {
            name: "choice",
            markdown: r#"Define a finite set of operations a linear value offers.

A `choice` type lists lower-case branches and the result type of selecting each branch:

```par
type Command = choice {
  .close => !,
  .reset => !,
}
```

Construct a choice value with standalone `case`, then select an operation with postfix `.branch`:

```par
def MakeCommand: Command = case {
  .close => !,
  .reset => !,
}

def Close = MakeCommand.close
```

Branches that accept arguments use `.branch(ArgumentType) => ResultType`. A `choice` offers operations: its implementation provides all branches, and the code holding the value selects exactly one branch. This differs from `either`, where the constructor chooses one variant and a `.case` expression discovers which variant it received.

Choice values are linear: select exactly one available branch, or use an explicit branch such as `.close` to consume them. A `choice` guards `self` in an `iterative` type.

[Choice](https://par.run/book/types/choice) | [Iterative types](https://par.run/book/types/iterative)"#,
        }),
        TokenKind::Recursive => Some(KeywordDocumentation {
            name: "recursive",
            markdown: r#"Create a finite self-referential type.

`recursive` binds `self` inside its body, allowing a type to contain smaller values of the same type without a cyclic global definition:

```par
type List = recursive either {
  .end!,
  .item self,
}
```

Values are constructed as the body type. To recursively consume one, use `.begin` on the recursive value and `.loop` on a descendant occupying a `self` position.

Every `self` belonging to a `recursive` type must be guarded by an `either` somewhere between `recursive` and `self`. Nested recursive or iterative types can use labels to select the intended binder:

```par
type Nested = recursive@outer either {
  .next self@outer,
}
```

[Recursive types](https://par.run/book/types/recursive)"#,
        }),
        TokenKind::Iterative => Some(KeywordDocumentation {
            name: "iterative",
            markdown: r#"Create a potentially unbounded self-referential type.

`iterative` binds `self` inside its body. It commonly describes an object or protocol that can produce another version of itself after each operation:

```par
type Sequence = iterative choice {
  .close => !,
  .next => self,
}
```

Construct an iterative value with standalone `begin` and use standalone `loop` for its next `self` value. Consume an iterative value by operating on its expanded body, such as `sequence.next`.

Every `self` belonging to an `iterative` type must be guarded by a `choice`. Iterative values are always linear, so a practical type usually offers a branch such as `.close` that consumes it. Labels disambiguate nested binders:

```par
type Nested = iterative@outer choice {
  .close => !,
  .next => self@outer,
}
```

[Iterative types](https://par.run/book/types/iterative)"#,
        }),
        TokenKind::Self_ => Some(KeywordDocumentation {
            name: "self",
            markdown: r#"Refer to the type currently bound by `recursive` or `iterative`.

Use `self` only inside the body of a recursive or iterative type:

```par
type List = recursive either {
  .end!,
  .item self,
}
```

Here the payload of `.item` is another `List`. A `self` reference must match an enclosing binder; labels select one when binders are nested:

```par
recursive@outer either {
  .next self@outer,
}
```

For totality, `self` in a `recursive` type must be guarded by `either`; `self` in an `iterative` type must be guarded by `choice`. It is a type-level reference, not a runtime object variable.

[Recursive types](https://par.run/book/types/recursive) | [Iterative types](https://par.run/book/types/iterative)"#,
        }),
        TokenKind::Dual => Some(KeywordDocumentation {
            name: "dual",
            markdown: r#"Transform a type into its communication opposite.

`dual T` describes the peer that communicates with a value of type `T`: what one side provides, the other side consumes. It flips every connective in `T`: a function becomes a pair, an `either` becomes a `choice`, and a `recursive` type becomes an `iterative` type; the reverse transformations apply as well.

```par
type Request = [String] Nat
type Response = dual Request
```

`Request` describes a side that receives a `String` and returns a `Nat`. Its dual, `Response`, is `(String, Nat)!`: the peer provides the `String` and receives the `Nat`. The same reversal applies throughout nested types. For example, a producer-facing `either` becomes a consumer-selectable `choice` for its peer.

`dual` is primarily useful for channels. A `chan` expression exposes one endpoint as its result and binds the opposite endpoint inside its process:

```par
def FortyTwo: Nat = chan respond {
  respond <> 42
}
```

Because the whole expression has type `Nat`, `respond` has type `dual Nat`. More generally, if a `chan` expression has type `T`, its bound endpoint has type `dual T`, so linking the two endpoints connects compatible opposite roles. Applying `dual` twice returns the original type: `dual dual T` is equivalent to `T`.

[Construction by destruction](https://par.run/book/processes/duality) | [Channels and linking](https://par.run/book/processes/chan_expression)"#,
        }),
        TokenKind::Unfounded => Some(KeywordDocumentation {
            name: "unfounded",
            markdown: r#"Establish a loop point without Par's usual totality check.

Like `begin`, `unfounded` marks the point to which a later `loop` returns. Use standalone `unfounded` when constructing an `iterative` value, as in `unfounded case { ... }`. Use postfix `.unfounded` after a `recursive` value when consuming it, as in `value.unfounded.case { ... }`. In a process command chain, write `.unfounded` on the command subject. In every form, a later matching `loop` starts the body again from that marked point.

```par
type Sequence = iterative choice {
  .close => !,
  .next => self,
}

def Endless: Sequence = unfounded case {
  .close => !,
  .next => loop,
}
```

Postfix `.unfounded` can also end a chain after exposing a recursive value's body; append `.case`, `.loop`, or another supported postfix operation when the body must be used immediately. Labels work in all positions: `unfounded@label`, `.unfounded@label`, and `loop@label`.

Use it only when a program is total or productive but the checker cannot prove the required descent or productivity. Unlike `begin`, `unfounded` disables that proof obligation, so it can admit non-terminating or unproductive behavior.

[Recursive types](https://par.run/book/types/recursive#the-escape-hatch-from-totality-unfounded) | [Iterative types](https://par.run/book/types/iterative#the-escape-hatch-from-totality-unfounded)"#,
        }),
        TokenKind::Neg => Some(KeywordDocumentation {
            name: "neg",
            markdown: r#"Negate a signed numeric expression.

`neg expression` is prefix arithmetic negation:

```par
def Negative = neg 5
def Positive = neg neg 5
```

It works on signed numeric types, currently `Int` and `Float`; it does not apply to `Nat`. `neg` binds more tightly than `*`, `/`, `+`, and `-`, so `neg x * y` means `(neg x) * y`.

Although it is lexed specially for prefix negation, `neg` is also accepted as a lower-case local name where a name is expected:

```par
def UseNeg = let neg = 2 in neg
```

[Primitive types](https://par.run/book/structure/primitive_types) | [Type constraints](https://par.run/book/types/constraints)"#,
        }),
        TokenKind::Case => Some(KeywordDocumentation {
            name: "case",
            markdown: r#"Construct a `choice` value or branch on an `either` value.

Standalone `case` constructs a value of a `choice` type by implementing each operation:

```par
def MakeCommand: Command = case {
  .close => !,
  .reset => !,
}
```

Postfix `.case` destructs an `either`. Each named branch matches a variant and binds its payload with a pattern:

```par
result.case {
  .ok value => value,
  .err error => error,
}
```

Postfix branches accept a unit pattern, a short payload binding, or a parenthesized pattern. These forms are equivalent when the payload has the matching shape:

```par
result.case {
  .none! => fallback,
  .some value => value,
  .pair(left, right)! => Combine(left, right),
}
```

In a process command chain, `.case` branch bodies are `{ process }` blocks rather than result expressions. A branch can omit its payload pattern, bind it directly, or destructure it:

```par
stream.begin.case {
  .end! => { exit! }
  .item value => { Handle(value); stream.loop }
}
```

Case forms may also have one final `else` fallback for unmatched branches:

```par
result.case {
  .ok value => value,
  else => fallback,
}
```

Within a branch, `try` follows the variant name and precedes its payload pattern to unwrap a `Try` payload. An `.err` payload transfers to the matching `catch`:

```par
catch error => fallback in
result.case {
  .loaded try value => value,
  else => fallback,
}
```

Likewise, `default(fallback)` follows the variant name and precedes its payload pattern to unwrap an `Option` payload. The binding receives the `.some` payload or the fallback for `.none!`:

```par
result.case {
  .count default(0) count => count + 1,
  else => 0,
}
```

[Either](https://par.run/book/types/either) | [Choice](https://par.run/book/types/choice) | [Looping and branching](https://par.run/book/processes/commands/looping_and_branching)"#,
        }),
        TokenKind::Chan => Some(KeywordDocumentation {
            name: "chan",
            markdown: r#"Create two opposite channel endpoints and run a process with one of them.

`chan pattern { process }` creates a pair of endpoints. It binds one endpoint to `pattern` inside `process` and evaluates to its opposite outside the block. If the whole expression has type `T`, the bound endpoint has type `dual T`.

The shortest form uses `<>` to connect the bound endpoint directly to a value. This ends the process and makes the `chan` expression evaluate to that value:

```par
def FortyTwo: Nat = chan out {
  out <> 42
}
```

`<>` is not required. The bound endpoint can instead be used through its protocol commands over several process steps. For example, this process supplies the two steps of a one-item list to the endpoint it calls `yield`:

```par
def Singleton: List<Nat> = chan yield {
  yield.item(1)
  yield.end!
}
```

Here the whole expression evaluates to the list, while `yield` has the dual endpoint type that offers `.item` followed by `.end!`. A `chan` process must use or complete its bound endpoint according to that endpoint's type; it does not need to link it with `<>`.

The bound endpoint is a pattern, so it can carry a type annotation or destructure a value. The process must satisfy Par's process-termination rules. `do { process } in value` is syntax sugar for a `chan` whose process links its result endpoint to `value`.

An empty process is also valid syntax, although it must still meet the surrounding type and termination requirements:

```par
chan out { }
```

[Channels and linking](https://par.run/book/processes/chan_expression) | [Construction by destruction](https://par.run/book/processes/duality)"#,
        }),
        TokenKind::Do => Some(KeywordDocumentation {
            name: "do",
            markdown: r#"Run a sequence of process commands before producing an expression value.

`do { process } in expression` runs the process block, then evaluates the expression after `in`. Process bindings that fall through are available to that expression:

```par
def Four: Nat = do {
  let two = 2
  let sum = two + two
} in sum
```

Process `let` statements deliberately have no `in`; they bind names for subsequent process steps. A `do` block can also be empty: `do { } in expression`.

`do` is syntax sugar for a `chan` expression whose process links its result endpoint to the expression after `in`.

[Do expressions](https://par.run/book/processes/do_expression) | [Channels and linking](https://par.run/book/processes/chan_expression)"#,
        }),
        TokenKind::If => Some(KeywordDocumentation {
            name: "if",
            markdown: r#"Choose the first branch whose condition succeeds.

In an expression, each branch returns a value and all selected results must have compatible types:

```par
def Describe: [Flag] String = [flag] if {
  flag is .yes! => "yes",
  else => "no",
}
```

Expression branches are tried top to bottom. `else` is optional only when the conditions are exhaustive. In process syntax, branches contain `{ process }` blocks and can fall through to later process code. The short process form `if condition => { process }` uses the following process code as its fallback path.

The two process forms are:

```text
if {
  <condition> => { <process> }
  else => { <process> }
}

if <condition> => { <process> }
<following process>
```

Conditions support `is`, `not`, `and`, `or`, and braces for explicit grouping. Bindings introduced by a successful condition are available in its branch.

[Conditions and if](https://par.run/book/quality_of_life/if)"#,
        }),
        TokenKind::Else => Some(KeywordDocumentation {
            name: "else",
            markdown: r#"Provide the final fallback branch of a branching construct.

`else` appears once, after the named or conditional branches:

```par
if {
  value is .some item => item,
  else => fallback,
}
```

In `if`, it runs when no earlier condition succeeds. In `case`, it handles variants not covered by named branches:

```par
result.case {
  .some value => value,
  else => fallback,
}
```

In `poll` and `repoll`, it runs after the client pool is empty. Expression forms use `else => expression`; process forms use `else => { process }`:

```text
poll(<client>) {
  <current> => { submit() }
  else => { <process> }
}
```

An expression `if` may omit `else` only when its conditions are exhaustive. `poll` and `repoll` always require their empty-pool `else` branch.

[Conditions and if](https://par.run/book/quality_of_life/if) | [Polling and submitting](https://par.run/book/nondeterminism/poll_submit)"#,
        }),
        TokenKind::Is => Some(KeywordDocumentation {
            name: "is",
            markdown: r#"Match an `either` variant inside a condition.

The form is `value is .variant payload-pattern`:

```par
if {
  result is .ok value => value,
  result is .err error => error,
}
```

The payload pattern is required. Use `!` for a unit payload, as in `result is .none!`; use a name or pair pattern to bind a non-unit payload. Names bound by `is` are available on the condition's success path, including a later `and` operand and the selected `if` branch.

Payload variables can be annotated, and a parenthesized pattern can destructure a pair:

```par
if {
  result is .ok value: String => value,
  result is .pair(left, right)! => Combine(left, right),
  else => fallback,
}
```

`is` works only with `either` values. Use ordinary boolean expressions for non-variant tests.

[Conditions and if](https://par.run/book/quality_of_life/if) | [Either](https://par.run/book/types/either)"#,
        }),
        TokenKind::And => Some(KeywordDocumentation {
            name: "and",
            markdown: r#"Require two conditions to succeed, evaluating the right side only after the left succeeds.

`and` short-circuits on failure, so bindings created by the left condition are available to the right condition and to the successful branch:

```par
if {
  result is .ok value and value == 0 => "zero",
  else => "not zero or error",
}
```

Condition precedence is `not` before `and` before `or`; use `{ ... }` to group a condition explicitly. Although lexed specially, `and` is also accepted as a lower-case identifier where the parser expects a local name.

[Conditions and if](https://par.run/book/quality_of_life/if)"#,
        }),
        TokenKind::Or => Some(KeywordDocumentation {
            name: "or",
            markdown: r#"Try a fallback condition only when the first condition fails.

`or` short-circuits on success:

```par
if {
  primary is .ok value or fallback is .ok value => value,
  else => "missing",
}
```

If a binding is needed after `or`, every success path must bind the same name, as `value` does above. Condition precedence is `not` before `and` before `or`; use `{ ... }` to group explicitly. Although lexed specially, `or` is also accepted as a lower-case identifier where the parser expects a local name.

[Conditions and if](https://par.run/book/quality_of_life/if)"#,
        }),
        TokenKind::Not => Some(KeywordDocumentation {
            name: "not",
            markdown: r#"Invert whether a condition succeeds or fails.

`not` changes both the result and the path on which condition bindings exist:

```par
if {
  not result is .ok value => "error",
  else => value,
}
```

Here `value` is bound when `result is .ok value` succeeds. Because `not` reverses that condition, `value` is available in `else`, not in the first branch. `not` has higher precedence than `and` and `or`; use `{ ... }` for explicit grouping.

Although lexed specially, `not` is also accepted as a lower-case identifier where the parser expects a local name.

[Conditions and if](https://par.run/book/quality_of_life/if)"#,
        }),
        TokenKind::Catch => Some(KeywordDocumentation {
            name: "catch",
            markdown: r#"Handle `Try` errors or explicit `throw`s in the same local control-flow scope.

Expression form gives an optional label, an error pattern, and a value to produce when an error occurs:

```par
catch error => .err error in
let try value = result in .ok value
```

Process form gives the handler a process block:

```par
catch error => {
  console.print(error)
  exit!
}
```

Labels select a particular handler in either form:

```text
catch@io <pattern> => <handler-expression> in <expression>
catch@io <pattern> => { <process> }
```

`try` and `throw` target the nearest matching catch. Grouping with `{ ... }` does not leave that catch scope. Propagation is rejected when it would cross into another process, such as the value side of a `let` or a `chan` body, or after an enclosing construction has begun producing its result. `catch` is local syntax sugar; it does not unwind a call stack.

[Error handling](https://par.run/book/quality_of_life/error_handling)"#,
        }),
        TokenKind::Throw => Some(KeywordDocumentation {
            name: "throw",
            markdown: r#"Transfer an error value to a matching local `catch` handler.

Use `throw expression` in an expression or as a terminating process step:

```par
catch error => .err error in
throw "failed"
```

In a process, `throw` ends the current path and transfers directly to the handler:

```par
catch error => {
  console.print(error)
  exit!
}
throw "failed"
```

The error value is matched against the catch pattern. `throw@label value` targets the nearest catch with the same label; unlabeled `throw` targets the nearest unlabeled catch.

A matching catch must remain active at the `throw` site. Grouping with `{ ... }` keeps it active, but `throw` cannot cross into another process or escape after an enclosing construction has begun producing its result. It does not unwind a call stack.

[Error handling](https://par.run/book/quality_of_life/error_handling)"#,
        }),
        TokenKind::Try => Some(KeywordDocumentation {
            name: "try",
            markdown: r#"Unwrap a successful `Try` value and send its error to a matching `catch`.

`try` has four related placements. In an expression, a `let` pattern or postfix `.try` unwraps a `Try` value:

```par
catch error => 0 in
let try value = result in value

catch error => 0 in
{result.try}
```

In a process command, postfix `.try` unwraps a `Try` result:

```par
writer.close.try
```

When a command's next receive is a `Try` payload, put `try` inside that receive pattern:

```par
console.prompt("Name: ")[try name]
```

These forms can also be combined with an ordinary receive:

```par
source.next.try[value]
```

Here `.try` unwraps the `Try` result of `source.next`; `[value]` is an ordinary receive pattern.

Within a `.case` branch, `try` follows the variant name and precedes its payload pattern:

```text
subject.case {
  .variant try <payload-pattern> => <expression>
}
```

On `.err error`, all forms transfer `error` to the matching local catch. Labels pair with labeled catches: `try@io` and `try@io value`. Grouping with `{ ... }` preserves the active catch, so `{result.try}` can propagate. When a `Try` is evaluated as the value side of a `let`, put `try` in the `let` pattern (`let try value = result`); that value is evaluated in another process. A `try` must also run before an enclosing construction has begun producing its result.

[Error handling](https://par.run/book/quality_of_life/error_handling)"#,
        }),
        TokenKind::Default => Some(KeywordDocumentation {
            name: "default",
            markdown: r#"Unwrap an `Option`, substituting a fallback for `.none!`.

Postfix `.default(fallback)` keeps a `.some` value or evaluates the fallback for `.none!`:

```par
let value = option.default(0) in value
```

The pattern form works in an expression `let`:

```par
let default(0) value = option in value
```

In a process receive command, it appears inside the brackets after the command:

```par
counts.entry(word)[default(0) count]
```

Inside a `.case` branch, `default(fallback)` follows the variant name and precedes its payload pattern. Both forms bind or produce the contained value for `.some`, and the fallback value for `.none!`. `default` is for `Option`; `try` is the corresponding error-propagating sugar for `Try`.

```text
subject.case {
  .variant default(<fallback>) <branch-binding> => <expression>
}
```

[Error handling](https://par.run/book/quality_of_life/error_handling#providing-defaults-with-default)"#,
        }),
        TokenKind::Todo => Some(KeywordDocumentation {
            name: "todo",
            markdown: r#"Mark an expression or process path as intentionally unimplemented.

`todo` can appear where an expression is required or as a process terminator:

```par
def Pending = todo
def StopHere: ! = chan exit { todo }
```

It is not a value-producing default and it is not accepted as completed program behavior: the type checker reports a `todo` error, and the backend also treats it as an unimplemented safeguard. Replace it with a real expression or process before the program can type-check."#,
        }),
        TokenKind::Poll => Some(KeywordDocumentation {
            name: "poll",
            markdown: r#"Repeatedly serve whichever recursive client in a pool becomes ready.

`poll` starts a pool of recursive clients. Its expression form has one active-client branch and one required empty-pool `else` branch:

```text
poll@label(<client>, ...) {
  <current> => <expression containing submit>,
  else => <empty-pool result>,
}
```

The label is optional. The parser accepts an empty `poll()`, but a checked `poll` must have at least one initial client. Every initial client must have a recursive type. Its active branch receives a ready client and must end with exactly one `submit` or nested `repoll`:

```par
def Drain: [List] ! = [list] poll(list) {
  current => current.case {
    .end! => submit(),
    .item rest => submit(rest),
  }
  else => !,
}
```

The expression form returns branch values. Process form uses `{ process }` bodies and has the same pool rules:

```text
poll(<client>) {
  <current> => { submit() }
  else => { <process> }
}
```

Submitted clients must have the pool type and descend from the current polled client, ensuring progress. Variables carried across iterations must remain available with compatible types.

[Polling and submitting](https://par.run/book/nondeterminism/poll_submit)"#,
        }),
        TokenKind::Repoll => Some(KeywordDocumentation {
            name: "repoll",
            markdown: r#"Reuse the nearest active poll pool with a new handler.

`repoll` looks like `poll`, but it does not create a pool. Inside an active `poll` or `repoll` branch, it can add clients and switch the handler for the same pool:

```par
poll(client) {
  current => repoll() {
    next => submit(),
    else => !,
  },
  else => !,
}
```

The optional label identifies the new poll point; its client list may be empty when no clients need to be added:

```text
repoll@next() {
  <current> => <expression containing submit>,
  else => <empty-pool result>,
}
```

Like `poll`, it has an active client branch and an empty-pool `else` branch. Process form uses process bodies:

```text
repoll() {
  <current> => { submit() }
  else => { <process> }
}
```

It must be inside an active polling branch, and each active branch must finish with exactly one `submit` or nested `repoll`.

Use it for mode changes, such as switching from producing work to cancelling every client that remains in the pool.

[Switching modes with repoll](https://par.run/book/nondeterminism/repoll)"#,
        }),
        TokenKind::Submit => Some(KeywordDocumentation {
            name: "submit",
            markdown: r#"Return zero or more descendant clients to the nearest active poll pool.

Inside a `poll` or `repoll` active branch, `submit(...)` ends that branch and starts the next polling iteration. Its optional label selects an enclosing labeled poll point:

```par
submit()
submit(child)
submit@next(left, right)
```

In an expression, `submit` produces the eventual result of the selected poll. In a process, it is a terminating statement. A complete expression example is:

```par
poll(list) {
  current => current.case {
    .end! => submit(),
    .item rest => submit(rest),
  }
  else => !,
}
```

`submit()` is valid and removes the current client without adding another. Every submitted client must match the pool type and descend from the current client; this prevents reinserting an unchanged client forever. `submit` is valid only inside an active `poll` or `repoll` branch.

[Polling and submitting](https://par.run/book/nondeterminism/poll_submit)"#,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::lexer::RESERVED_KEYWORDS;
    use super::*;

    #[test]
    fn documents_every_reserved_keyword() {
        for &(name, keyword) in RESERVED_KEYWORDS {
            let documentation = documentation(keyword)
                .unwrap_or_else(|| panic!("missing documentation for {name}"));
            assert_eq!(documentation.name, name, "documentation name for {name}");
        }
    }

    #[test]
    fn converted_documentation_does_not_indent_prose_as_code() {
        for &(name, keyword) in RESERVED_KEYWORDS {
            let documentation = documentation(keyword).unwrap();

            assert!(
                !documentation.markdown.contains("\n\n    "),
                "{} documentation contains an indented Markdown code block",
                name,
            );
        }
    }
}
