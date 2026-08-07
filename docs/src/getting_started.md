# Getting Started

Let's install the **Par** programming language.

At the moment, there are no pre-built binaries, or releases, so we'll have to build it from
source.

### 1. Install Rust and Cargo

Par is written in [Rust](https://www.rust-lang.org). To be able to build it from source,
we'll need to install Rust and its build tool, called Cargo.

The easiest way to do that is via [rustup](https://rustup.rs). The website instructs:

> Run the following in your terminal, then follow the onscreen instructions.
>
> ```sh
> $ curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
> ```

### 2. Clone Par's repository

The next step is to obtain Par's source code. That is located on
[GitHub](https://github.com/par-team/par-lang). Clone it locally by running the following
in your terminal:

```sh
$ git clone https://github.com/par-team/par-lang
```

### 3. Build and install Par's CLI tool

Navigate to the newly created directory:

```sh
$ cd par-lang
```

Then install the executable using Cargo:

```sh
$ cargo install --path .
```

This may take a while as Rust downloads and builds all the dependencies.

This installs the `par` command.

### 4. Install the Visual Studio Code extension

If you use Visual Studio Code, install the
[Par extension](https://marketplace.visualstudio.com/items?itemName=par-lang.par-vscode).
It gives you syntax highlighting and editor support while you follow along.

This step is optional; the command-line tools work without it.

### 5. Create a package

A new `par` command should now be available in your terminal. It may be necessary
to restart the terminal for it to appear.

Let's create a fresh package:

```sh
$ par new hello_par
$ cd hello_par
```

This creates:

```text
hello_par/
  Par.toml
  src/
    Main.par
```

The generated `src/Main.par` is a tiny runnable Par program. You can run it with:

```sh
$ par run
```

And you can type-check the package without running it:

```sh
$ par check
```

### 6. Browse the docs

Par comes with a built-in docs browser:

```sh
$ par doc
```

This command is useful in three different situations:

- **Outside any package,** `par doc` shows the documentation for the built-in packages.
- **Inside a package,** `par doc` shows the current package together with its dependencies.
- **For a remote package,** `par doc --remote github.com/faiface/par-cancellable` lets you inspect
  a package without manually adding it as a dependency.

### 7. Open the playground

The playground is a great way to experiment with code and interact with values through the
playground's automatic UI:

```sh
$ par playground
```

And the playground should appear:

![Playground window](./images/getting_started_1.png)

If all is good, turn the page and **let's get into the language itself!**

In case of problems, head over to our [Discord](https://discord.gg/8KsypefW99), we'll try and
help.
