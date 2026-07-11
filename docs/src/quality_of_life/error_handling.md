# Error Handling

Programs that interact with the real world must handle errors gracefully. Files don't exist, networks disconnect, users type unexpected input. Most errors occur at I/O boundaries where your program meets external systems beyond its control.

Par takes a structured approach to error handling that builds on its linear type system. At its core, Par uses explicit Try types — but adds lightweight syntax sugar that makes working with errors feel natural while keeping the underlying semantics transparent.

## Why Par Needs Unique Error Handling

Par's linear type system together with its concurrent evaluation creates a unique situation for error handling. Traditional approaches don't work for Par:

**Exceptions** propagate across call stacks, unwinding through multiple function calls automatically. But Par's concurrent execution model has no call stacks! Instead, it has processes that communicate via channels. Any error must be explicitly passed via a channel, making something like a `Try` type necessary for error handling.

**Rust's `?` operator** works by dropping remaining owned values when propagating errors. This implicit cleanup doesn't translate to Par's linear types, where each value must be consumed according to its specific type and context.

Par needs error handling that makes cleanup fully explicit while remaining convenient to use. The `try`/`catch`/`throw` syntax sugar introduced here achieves this balance — borrowing familiar keywords from exception handling while operating very differently. Unlike traditional exceptions, Par's error handling is purely local syntax sugar over `Try` types, with no hidden control flow or stack unwinding.

## Working with Files: Error Handling Without Sugar

Let's start with a concrete example using Par's file system operations through the built-in `@basic/Os` module. The `Os.Path` type provides methods for working with the filesystem — creating files, reading directories, and so on. Most of these operations can fail, so they return `Try` values.

Here's what error handling looks like without any syntax sugar. We'll write a program that creates a log file and writes some entries to it:

```par
module Main

import {
  @basic/Console
  @basic/Os
}

def Main: ! = chan exit {
  let console = Console.Open

  let path = Os.Path("logs.txt")
  Os.CreateOrAppendToFile(path).case {
    .err e => {
      console.print(e)
      console.close
      exit!
    }
    .ok writer => {}
  }
  // ...
```

A few things to note about this pattern:

The `chan exit` creates a channel called `exit` of type `?` — [the continuation type](./processes/duality.md), which is dual to our `Main` function's return type `!`. The `exit!` syntax is the _break_ command applied to this continuation, which ends the process.

After the `.case` block, the `writer` variable is available in the surrounding scope. This is how process-scoped variables work in Par — variables bound in `.case` branches continue to exist after the case analysis.

```par
  writer.write("[INFO] First new log\n").case {
    .err e => {
      console.print(e)
      console.close
      exit!
    }
    .ok => {}
  }
```

In [process syntax](./process_syntax.md), when we use `.ok =>`, the subject of the command (`writer`) gets updated to the payload of the .ok branch. Since `.write` returns the same `Os.Writer` type on success, `writer` remains usable.

```par
  writer.write("[INFO] Second new log\n").case {
    .err e => {
      console.print(e)
      console.close
      exit!
    }
    .ok => {}
  }
```

And finish by closing the file:

```par
  writer.close.case {
    .err e => {
      console.print(e)
      console.close
      exit!
    }
    .ok! => {}
  }
  exit!
}
```

Note the `.ok!` pattern here — after closing, the writer becomes a unit value `!`.

Here's the complete program:

```par
module Main

import {
  @basic/Console
  @basic/Os
}

def Main: ! = chan exit {
  let console = Console.Open

  let path = Os.Path("logs.txt")
  Os.CreateOrAppendToFile(path).case {
    .err e => {
      console.print(e)
      console.close
      exit!
    }
    .ok writer => {}
  }
  
  writer.write("[INFO] First new log\n").case {
    .err e => {
      console.print(e)
      console.close
      exit!
    }
    .ok => {}
  }
  writer.write("[INFO] Second new log\n").case {
    .err e => {
      console.print(e)
      console.close
      exit!
    }
    .ok => {}
  }

  writer.close.case {
    .err e => {
      console.print(e)
      console.close
      exit!
    }
    .ok! => {}
  }

  console.close
  exit!
}
```

This is extremely verbose! The same error handling code is repeated for every operation that might fail. Let's see how Par's error handling sugar can clean this up.

## The Same Program with `try`/`catch`

Here's the exact same functionality using Par's error handling syntax:

```par
module Main

import {
  @basic/Console
  @basic/Os
}

def Main: ! = chan exit {
  let console = Console.Open

  catch e => {
    console.print(e)
    console.close
    exit!
  }

  let path = Os.Path("logs.txt")
  let try writer = Os.CreateOrAppendToFile(path)
  
  writer.write("[INFO] First new log\n").try
  writer.write("[INFO] Second new log\n").try

  writer.close.try
  console.close
  exit!
}
```

Significantly shorter and more readable! The error handling is declared once and applies to all subsequent operations.

## How `try`/`catch`/`throw` Work in Process Syntax

Par's error handling sugar is built around small, local keywords that desugar to explicit `Try` handling. Let's understand how they work.

### The `catch` Statement

Before you can use `try` or `throw`, you must define a `catch` block in the same process. This restriction is crucial — the corresponding `try` and `throw` commands must be in the same sequential process as their `catch`, not in nested processes or expressions.

```par
catch <pattern> => {
  <process>
}
```

The `<pattern>` can be any pattern like those used in `let` statements or function parameters. Usually this is a simple variable name, but you can use more complex patterns when needed.

For example, if the error type is unit:

```par
catch ! => { ... }
```

You can also include type annotations:

```par
catch e: Os.Error => { ... }
```

The `<process>` inside a `catch` block must end with a process-ending command:

- _break:_ `continuation!`
- _linking:_ `left <> right`
- `.loop` to return to a .begin that's outside the catch block, useful for retrying operations
- `throw` to jump to another `catch` block

### The `throw` Command

`throw` jumps directly to a `catch` block with an error value:

```par
catch e => {
  console.print(e)
  console.close
  exit!
}

throw "Total meltdown"
```

This is equivalent to executing the catch block directly:

```par
console.print("Total meltdown")
console.close
exit!
```

`throw` is useful for creating custom error conditions in your logic.

## The `try` Patterns and Commands

The real power comes from `try`, which provides conditional error handling based on `Try` values:

```par
type Try<e, a> = either {
  .err e,
  .ok a,
}
```

`try` appears in two contexts: _patterns_ and _commands._

### `.try` in Commands

The `.try` postfix transforms verbose `Try` case analysis into clean linear code. Remember our original verbose version:

```par
writer.write("[INFO] First new log\n").case {
  .err e => {
    console.print(e)
    console.close
    exit!
  }
  .ok => {}
}
```

With `.try`, this becomes:

```par
writer.write("[INFO] First new log\n").try
```

The `.try` postfix desugars any command or expression returning a `Try`:

```par
variable.try
```

becomes:

```par
variable.case {
  .err e => {
    throw e
  }
  .ok => {}
}
```

This works for more complex command chains too. Consider this type for polling data with possible errors:

```par
type Poll<e, a> = iterative choice {
  .close => Try<e, !>,
  .next => Try<e, (a) self>,
}
```

You can poll an element and handle errors seamlessly:

```par
// source : Poll<Os.Error, String>
source.next.try[value]
```

After this command, `source` maintains its `Poll<Os.Error, String>` type and value contains the successfully polled `String`.

<!-- moved `default` to the end of this chapter -->

### The Concurrent Evaluation Restriction

You might think this would work:

```par
let writer = Os.CreateOrAppendToFile(path).try
```

However, this causes a type error. The reason reveals something fundamental about Par's evaluation model.

Par evaluates expressions concurrently with the processes that use them. When you write:

```par
let writer = Os.CreateOrAppendToFile(path).try
```

The expression `Os.CreateOrAppendToFile(path)` runs concurrently with the process doing the `let`. If the expression were to fail on `.try`, the main process might already be executing other commands — there's no sound way to "rewind" that execution.

This is why `try` and `throw` can only be used in the same process as their corresponding `catch`, not in nested expressions or processes.

### `try` in Patterns

The solution is to use `try` in the pattern itself:

```par
let try writer = Os.CreateOrAppendToFile(path)
```

This moves the error handling into the correct process. The desugaring is:

```par
let writer = Os.CreateOrAppendToFile(path)
writer.case {
  .err e => {
    throw e
  }
  .ok => {}
}
```

Since `try` is part of the pattern, it works in nested patterns too:

```par
let (try leftReader, try rightReader)! = (
  Os.OpenFile(leftPath),
  Os.OpenFile(rightPath),
)!
```

And it works in receive commands, too. The `Console` type demonstrates this well:

```par
type Console = iterative choice {
  .close => !,
  .print(String) => self,
  .prompt(String) => (Try<!, String>) self,
}
```

The `.prompt` method returns a `Try` while keeping the console alive for more operations:

```par
let console = Console.Open

catch ! => {
  console.print("Failed to read input.")
  console.close
  exit!
}

console.prompt("What's your name?")[try name]
console.prompt("What's your address?")[try address]
```

## Error Handling in Expression Syntax

Par also supports `try`/`catch` directly in expressions, with syntax adapted for expression contexts:

```par
catch <pattern> => <err result> in <expression using try/throw>
```

The same concurrent evaluation restrictions apply, with an additional constraint: `try`/`throw` can only be used before any part of the result is constructed.

This is invalid because `result.try` appears in a nested expression, which runs as a separate concurrent process:

```par
// result : Try<String, Int>
catch e => .err e in
.ok {result.try + 1}
```

This fix attempts to work around the nested expression issue but still fails — the outer `.ok` constructs part of the result before `try` executes:

```par
catch e => .err e in
.ok let try value = result in
value + 1
```

Here's the correct version:

```par
catch e => .err e in
let try value = result in
.ok {value + 1}
```

This ensures all error handling completes before constructing the result.

### Useful Expression Patterns

Expression-form `catch` enables several common patterns:

#### Mapping the error (adding context):

```par
catch e => .err String.Builder.add("Failed to process file: ").add(e).build in
let try content = file.readAll in 
.ok ProcessContent(content)
```

#### Mapping the success value:

```par
catch e => .err e in
let try rawData = source.fetch in 
.ok Encode(rawData)
```

#### Unwrapping with a default value:

```par
catch ! => "Unknown" in 
config.getUserName.try
```

## Labels and Layered Error Handling

Like `begin`/`loop`, `catch` blocks can be labeled for precise control:

```par
catch@label e => { ... }
```

The corresponding `try` and `throw` commands reference the same label:

```par
let try@label value = result
throw@label "Custom error"
```

Labels are selected by proximity and name, not by error type. The nearest `catch` with the matching label (or no label) is chosen. This allows different error types to be routed to different handlers:

```par
catch@fs e => { /* handle file system errors */ }
catch@net e => { /* handle network errors */ }

let try@fs writer = path.createFile
let try@net conn = url.connect
```

### Throwing to Previous `catch` Blocks

A powerful pattern is using nested `catch` blocks for resource cleanup while delegating to outer blocks for shared error handling.

Here's a simple example showing the basic pattern:

```par
catch e => {
  Debug.Log("Main error handler")
  Debug.Log(e)
  exit!
}

let try resource = AcquireResource
catch e => {
  resource.cleanup
  throw e  // delegate to the main handler above
}

// use resource, but error might occur elsewhere
let try otherData = SomeOtherOperation  // this might fail
ProcessTogether(resource, otherData)
```

The inner `catch` handles cleanup of the specific resource, then `throw`s to the outer `catch` for shared error reporting logic. The key point is that the error occurs in `SomeOtherOperation`, not in the resource itself, so the resource is still valid and needs proper cleanup.

Here's this pattern in a more complex, real-world example — copying a file with proper resource management:

```par
def Main: ! = chan exit {
  let console = Console.Open

  catch ! => { console.print("Failed to read input.").close; exit! }
  console.prompt("Src path: ")[try src]
  console.prompt("Dst path: ")[try dst]

  catch e: Os.Error => {
    console.print("An error occurred:")
    console.print(e)
    console.close
    exit!
  }

  let try reader = Os.OpenFile(Os.Path(src))
  catch@w e => { reader.close; throw e }

  let try@w writer = Os.CreateOrReplaceFile(Os.Path(dst))
  catch@r e => { writer.close; throw e }

  reader.begin.read.try@r.case {
    .end! => {
      writer.close.try
      console.close
      exit!
    }
    .chunk(bytes) => {
      writer.write(bytes).try@w
      reader.loop
    }
  }
}
```

Here, the `catch@r` and `catch@w` blocks provide resource-specific cleanup (closing file handles) but then throw to the main error handler for shared logic like printing the error and exiting.

This layered approach allows you to build sophisticated error handling hierarchies while keeping each level focused and clear.

## Propagating Errors in Functions

The examples so far have shown terminal error handling — printing errors and exiting. But often you want to propagate errors up to the caller. Here's a utility function that reads an entire file's contents:

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

This function opens a file with `Os.OpenFile(path)`, then uses `Bytes.ReadAll` to collect the chunked `Bytes.Reader` into a single `Bytes` value. The `catch` block propagates any errors as an `.err` result, while success returns the contents as `.ok`.

## Providing defaults with `default`

Sometimes you don’t want to branch on a missing optional value — you want to replace it with a fallback and keep going. The `default` sugar does exactly that for `Option` values.

This is separate from `try`/`catch`: `try` unwraps `Try` values and propagates `.err`, while `default` unwraps `Option` values and replaces `.none!`. If you have a `Try` and want to ignore the error, convert it first with `Try.ToOption`.

- Postfix form (expressions/commands):

  ```par
  let r1: Option<Int> = .some 7
  let r2: Option<Int> = .none!

  let x = r1.default(0)   // x = 7
  let y = r2.default(0)   // y = 0

  let result: Try<String, Int> = .err "not a number"
  let option = Try.ToOption(result)
  let z = option.default(0)  // z = 0
  ```

  This desugars to a `.case` on the subject: on `.some` it continues with the unwrapped value, on `.none` it evaluates the fallback expression and uses that value instead. Because it is a local rewrite, it can be used directly in `let` bindings and other expression contexts.

- Pattern form (including in receives):

  ```par
  let default(0) n = Nat.FromString("oops")
  ```

  The pattern binds on `.some`, and binds the fallback expression on `.none`.

  Here’s a practical example that shows why the pattern form is particularly useful with receive commands. It counts word occurrences using a map; when a key is missing, it starts from `0`:

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

  In the `.item` branch, `counts.entry(word)` returns an `Option<Nat>` via a receive; `default(0)` seamlessly handles the missing case and binds `count` to `0`.
