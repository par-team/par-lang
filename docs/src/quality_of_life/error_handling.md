# Error Handling

Programs that interact with the real world must handle errors gracefully. Files don't exist,
networks disconnect, and users type unexpected input. Most errors occur at I/O boundaries, where
our programs meet systems beyond their control.

Par represents errors with explicit `Try` values. On top of them, `try`/`catch`/`throw` provide a
lightweight syntax for propagating errors through a process. And when propagation abandons
resources in scope, [auto-cleanup](../types/auto_cleanup.md) makes sure they are still closed, canceled, or
rolled back.

## Errors Are Values

The standard `Try` type is an [either](../types/either.md):

```par
type Try<e, a> = either {
  .err e,
  .ok a,
}
```

A successful operation returns `.ok` with its result. A failed operation returns `.err` with an
error value. There are no exceptions hidden underneath.

This matters in Par because concurrent processes do not form a call stack that an exception could
unwind. Processes communicate through channels, so an error moving from one process to another
must be sent explicitly as part of its protocol.

Within one sequential process, though, repeatedly matching on `Try` would be tedious. That's the
part handled by `try`/`catch`/`throw`. These constructs are local syntax sugar: they make explicit
`Try` propagation pleasant without introducing exception-style stack unwinding.

## A First Look at `try`/`catch`

Here is a complete program that copies one file to another:

```par
module CopyFile

import {
  @core/Bytes
  @basic/Console
  @basic/Os
}

def Main: ! = chan exit {
  let console = Console.Open

  catch ! => { console.print("Failed to read input."); exit! }
  console.prompt("Src path: ")[try src]
  console.prompt("Dst path: ")[try dst]

  catch e: Os.Error => {
    console.print("An error occurred:")
    console.print(e)
    exit!
  }

  let try reader = src->Os.Path->Os.OpenFile
  let try writer = dst->Os.Path->Os.CreateOrReplaceFile

  reader.begin.read.try.case {
    .end! => {
      writer.close.try
      exit!
    }
    .chunk(bytes) => {
      writer.write(bytes).try
      reader.loop
    }
  }
}
```

The first `catch` handles the unit error returned by `console.prompt`. The second handles file-system
errors. Every matching `try` either unwraps an `.ok` value and continues, or transfers an `.err`
value to the nearest `catch`.

Notice what the error handlers do **not** contain: a growing list of resources to close. If opening
the destination fails, the already-open reader is cleaned up. If copying fails later, every handle
that remains in scope on that error path is cleaned up. When `exit!` terminates either handler, the
console is cleaned up too.

Only one close remains explicit:

```par
writer.close.try
```

Closing a writer flushes its pending output and may itself fail. On the successful path, that error
is part of the operation's result, so the program observes it with `try`. Auto-cleanup is for paths
where we have already decided to abandon the resource and are willing to ignore the cleanup result.

## Auto-Cleanup on Error Paths

The file handles above are linear, but droppable. Their protocols mark `.close` as a cleanup branch:

```par
type Reader<e> = recursive choice {
  .close* => Try<e, !>,
  .read => Try<e, either {
    .end!,
    .chunk(Bytes) self,
  }>,
}

type Writer<e> = iterative choice {
  .close* => Try<e, !>,
  .write(Bytes) => Try<e, self>,
}
```

The `*` says that `.close` is the canonical safe way to dispose of the object. Whenever a `throw`,
link, or break abandons such a value, Par selects that branch automatically and continues cleaning
up its result.

For an `Os.Writer`, that result is `Try<Os.Error, !>`. Both branches are ordinary data, so the result
may be discarded. This also means an error produced by an automatic close is ignored. If the close
error matters, call `.close` explicitly and handle its `Try`, as the copy program does on success.

## What the Sugar Means

It helps to see the explicit code once. Without `try`, opening a file and continuing with its reader
looks like this in process syntax:

```par
let result = Os.OpenFile(path)
result.case {
  .err e => {
    console.print(e)
    exit!
  }
  .ok reader => {}
}

// `reader` is available here
```

The `.ok` branch falls through and makes `reader` available to the rest of the process. The `.err`
branch ends the current path. `try` packages this recurring shape into three small constructs,
which we will first cover in their [process syntax](../process_syntax.md) version.

### The `catch` Statement

A process `catch` defines what to do with a propagated error:

```par
catch <pattern> => {
  <process>
}
```

The `<pattern>` part binds an error value using the same pattern syntax as the left side of `let`, and makes it available in the `catch` body:

```par
catch ! => { ... }
catch e: Os.Error => { ... }
```

The body must end the current process path, i.e., it cannot fall through. It can:

- break with `continuation!`;
- link two channels with `left <> right`;
- `loop` to an enclosing `begin`, which is useful for retrying;
- `throw` to an earlier catch.

Before using `try` or `throw`, a matching `catch` must appear in the same sequential process. A catch
does not reach into nested expressions or processes.

### The `throw` Command

`throw` transfers a value directly to a catch:

```par
catch e => {
  console.print(e)
  exit!
}

throw "Total meltdown"
```

This behaves as if the catch body ran with `e` bound to `"Total meltdown"`. It is useful for errors
created by our own logic, rather than obtained from an existing `Try`.

### `try` in Patterns

Most fallible operations return a `Try` value. Put `try` in the pattern that matches on it:

```par
let try reader = Os.OpenFile(path)
```

This is shorthand for:

```par
let result = Os.OpenFile(path)
result.case {
  .err e => { throw e }
  .ok reader => {}
}
```

Because `try` is part of a pattern, it composes with other patterns:

```par
let (try leftReader, try rightReader)! = (
  Os.OpenFile(leftPath),
  Os.OpenFile(rightPath),
)!
```

It also works in receive commands. For example, `Console.prompt` returns a `Try` before continuing
with the console:

```par
catch ! => {
  console.print("Failed to read input.")
  exit!
}

console.prompt("What's your name?")[try name]
```

### `.try` in Commands

When the subject of a process command becomes a `Try`, postfix `.try` unwraps its successful branch
in place:

```par
writer.write("[INFO] Started\n").try
```

It is shorthand for the familiar case analysis:

```par
writer.write("[INFO] Started\n").case {
  .err e => { throw e }
  .ok => {}
}
```

On success, `writer` is updated to the value inside `.ok`, ready for its next command.

### Why `try` Must Be Local

This does not work in a process:

```par
let writer = Os.CreateOrReplaceFile(path).try  // Error
```

Par evaluates expressions concurrently. The `let` statement does not
wait for the expression on the right of `=` to evaluate before resuming the process. It proceeds to the
next statement immediately. That makes it impossible to throw from the nested expression: the process
may already be doing something else, and interrupting it would be unsafe.

Put `try` in the pattern instead:

```par
let try writer = Os.CreateOrReplaceFile(path)
```

Now the process waits for the `Try` to reveal `.ok` or `.err`, then proceeds or throws accordingly.

## Error Handling in Expressions

Expressions have their own local form of `catch`:

```par
catch <pattern> => <error result> in <expression using try or throw>
```

For example, a function can propagate an error while transforming its successful value:

```par
catch e => .err e in
let try rawData = source.fetch in
.ok Encode(rawData)
```

As in process syntax, `try` cannot jump out of a concurrently evaluated nested expression. It must
also run before any part of the result has been constructed. This is invalid:

```par
catch e => .err e in
.ok {result.try + 1}  // Error: `try` is inside a nested expression
```

Move the `try` to the sequential part first:

```par
catch e => .err e in
let try value = result in
.ok {value + 1}
```

The expression form is also useful for mapping an error:

```par
catch e => .err `Failed to process file: #{e}` in
let try content = file.readAll in
.ok ProcessContent(content)
```

## Labels and Multiple Error Routes

Like `begin`/`loop`, catches can be labeled:

```par
catch@fs e => { /* handle file-system errors */ }
catch@net e => { /* handle network errors */ }

let try@fs writer = path.createFile
let try@net connection = url.connect
```

Labels are selected by name and proximity, not by error type. `try@fs` and `throw@fs` use the nearest
preceding `catch@fs`; an unlabeled `try` or `throw` uses the nearest unlabeled catch.

Usually one catch is enough. Labels become useful when a process has genuinely different error
routes — or when strict linear resources need explicit cleanup.

### When Auto-Cleanup Is Not Available

Not every linear object admits a safe automatic disposal operation. A protocol may deliberately
leave its close branch unmarked because closing requires information only the caller has, or because
its result must never be ignored:

```par
type StrictResource = choice {
  .close => !,  // no `*`: this resource is strictly linear
}
```

If an error path has a `StrictResource` in scope, Par rejects it unless that path consumes the
resource. Labeled catches can form a small cleanup chain for this case:

```par
catch e => {
  console.print(e)
  exit!
}

let try first = OpenFirst
catch@first e => {
  first.close
  throw e
}

let try@first second = OpenSecond
catch@second e => {
  second.close
  throw@first e
}

Prepare.try@second

second.close
first.close
exit!
```

If `OpenSecond` fails, `catch@first` closes `first` and delegates to the main catch. If `Prepare`
fails, `catch@second` closes `second`, then throws through `catch@first`, which closes `first`. The
successful path closes both resources explicitly.

## Propagating Errors from Functions

A catch does not have to print an error or exit. It can construct a new `Try` result, propagating the
error to the caller:

```par
module Main

import {
  @basic/Os
  @core/Bytes
  @core/Try
}

dec ReadAll : [Os.Path] Try<Os.Error, Bytes>
def ReadAll = [path]
  catch e => .err e in
  let try reader = Os.OpenFile(path) in
  Bytes.ReadAll(reader)
```

The catch turns an error from `Os.OpenFile` into this function's `.err` result. On success, the reader
is passed to `Bytes.ReadAll`, which produces the final `Try`.

## Providing Defaults with `default`

Sometimes we don't want to propagate a missing optional value. We want to replace it with a fallback
and continue. The `default` sugar does that for `Option` values.

This is separate from `try`/`catch`: `try` unwraps `Try` and propagates `.err`, while `default` unwraps
`Option` and replaces `.none!`. If we have a `Try` whose error type satisfies `drop`, and intentionally
want to ignore the error, we can first convert it with `Try.ToOption`.

The postfix form works in expressions and commands:

```par
let r1: Option<Int> = .some 7
let r2: Option<Int> = .none!

let x = r1.default(0)  // 7
let y = r2.default(0)  // 0
```

There is also a pattern form, including in receives:

```par
let default(0) n = Nat.FromString("oops")
```

Here is a practical example. It counts words with a map, starting missing entries at `0`:

```par
dec Counts : [List<String>] List<(String) Nat>
def Counts = [words] do {
  let counts = Map.New(type String, type Nat)
  words.begin.case {
    .end! => {}
    .item(word) => {
      counts.entry(word)[default(0) count]
      counts.put(count + 1)
      words.loop
    }
  }
} in counts.list
```

`counts.entry(word)` returns an `Option<Nat>` through a receive. On `.some`, the pattern binds its
value; on `.none!`, it binds the fallback `0` instead.
