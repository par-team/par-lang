# Communicating Both Ways

So far, information only flowed one way: from the clients to the server. **What about the other way?**

For example, you might want a server that:
- Generates a unique ID upon request from a client.
- Manages a linear resource with clients taking alternating ownership of it.
- Mediates messages between clients that they address to one another.

All of the above require information flowing not just from clients to the server, but **from the server to the clients,** too. And **all of it is doable!**

Two things this is **not** about:
- **A server initiating an interaction.** With `poll`/`submit`, the client is always the one who initiates an interaction by becoming ready.
- **Nondeterministic direction of communication.** As we learned in the [introduction to this section](./README.md), Par does **not** support this, at least yet. The types always say which direction is the next one.

## Reminder: [Construction by destruction](../processes/duality.md)

In the previous chapters, we've used expression syntax exclusively. But, it's often useful to switch to [process syntax](../process_syntax.md), especially when combining I/O operations with the asynchronous construction of data structures, like lists and trees.

Let's take the `ListFan<a>` from the previous chapter.

```par
type ListFan<a> = recursive either {
  .end!,
  .spawn(self) self,
  .item(a) self,
}
```

We used it for merging `List<List<a>>`, but nothing prevents us from using it standalone as well:

```par
dec ServeListFan : [<a> ListFan<a>] List<a>
def ServeListFan = ...
```

The code will be the same as for `MergeLists` in the [previous chapter](./fan_pattern.md#the-fan-transformation).

How would we use it now? The expression syntax is obvious:

```par
ServeListFan(
  .spawn(.item(1).item(2).end!)
  .spawn(.item(3).item(4).end!)
  .end!
)
```

That certainly works, but if we wanted to combine that with some I/O operations, the [process syntax](../process_syntax.md) would be better. We'll use the [`chan`](../processes/chan_expression.md) to construct a `ListFan<a>` by destructing its consumer:

```par
ServeListFan(chan server {
  server.spawn(chan server {
    // I/O anywhere here
    server.item(1)
    server.item(2)
    server.end!
  })
  server.spawn(chan server {
    // I/O anywhere here
    server.item(3)
    server.item(4)
    server.end!
  })
  server.end!
})
```

Now it's more imperative! I'm sure you can imagine doing all kinds of I/O operations between those `.item` and `.end` calls.

What's the type of the `server` there? It's the dual of `ListFan<a>`:

```par
iterative choice {
  .end => ?,
  .spawn(dual self) => self,
  .item(a) => self,
}
```

**The above is the type of the _server_ from the point of view of the _client._**

For a small recap from the [_duality_ chapter](../processes/duality.md):
- The `?` — the continuation type — is an obligation to dispose of all your resources and finish.
  
  > **A client in the server-client setup must finish before the server does.** That follows from the basic principle of Par's concurrency model: processes connected by their channels form a single tree. It ensures no deadlocks, and no leaking of processes.
- The signature of `.spawn` with the `(dual self) => self`, is just the dual version of what we saw previously:
  
  ```par
  .spawn(self) self
  ```
  
  There we saw the server's point of view, but now we're looking at the client's side.

Now back to communicating both ways!

## Example: Giving out unique IDs

Suppose we want to spawn a couple of clients and have each of them get a unique ID. Here's a possible interface from the client's point of view:

```par
type IdServer = iterative choice {
  .end => ?,
  .spawn(dual self) => self,
  .getId => (Nat) self,
}
```

> Since the clients have to be [`recursive`](../types/recursive.md), the servers will be [`iterative`](../types/iterative.md) from the clients' point of view.

Here's the dual type from the server's point of view:

```par
//            = dual IdServer
type IdClient = recursive either {
  .end!,
  .spawn(self) self,
  .getId [Nat] self,
}
```

It's very similar to what we saw previously, except the `.getId` variant isn't giving out a value, it's taking one.

Here's how we can implement such a server, with comments:

```par
module Main

import @core/Nat

dec ServeIds : [IdClient] !
def ServeIds = [clients] do {
  // Initialize the internal state of the server.
  // Here it's just an `id` variable that will keep
  // getting incremented.
  let id = 0
} in poll(clients) {
  client => client.case {
    // Standard handling of the fan structure.
    .end! => submit(),
    .spawn(l) r => submit(l, r),

    // A client is requesting an ID.
    .getId client => do {
      // Increment the `id` variable in-place.
      id += 1
      // The `client`'s type is `[Nat] IdClient` here.
      // We send it a fresh ID using the send command.
      client(id)
    } in submit(client),
  }
  else => !
}
```

> Notice how the `id` variable gets implicitly passed to each new iteration at `submit`. The treatment of local variables in `poll`/`submit` is the same as it is with `.begin`/`.loop`. They're kept around, always keeping their latest values.

And here's how we can use it:

```par
module Main

import {
  @core/Debug
  @core/Nat
  @core/String
}

def SimpleClient: [String] IdClient = [name]
  // `IdServer` and `IdClient` are dual, so we can construct
  // an `IdClient` by operating on an `IdServer`.
  chan server: IdServer {
    server.getId[id]
    Debug.Log(`${name} got number #{id}`)
    server.end!
  }

def Main: ! = ServeIds(chan server {
  server.spawn(SimpleClient("A"))
  server.spawn(SimpleClient("B"))
  server.spawn(SimpleClient("C"))
  server.spawn(SimpleClient("D"))
  server.end!
})
```

Running this particular program will almost invariably assign the number 1 to A, 2 to B, 3 to C, and 4 to D. That's just because the clients immediately ask for their ID. If they did any work before asking for it, or were asking multiple times, the results would've been different.

The point is, they can run independently, ask for an ID at any point, and get a fresh one every time.

## Example: Sharing a resource

We can use the synchronous nature of the server to give out unique access to a shared resource.

1. Initially, the server holds the resource.
2. A client can request it, and the server gives it out.
3. The exclusive session of the server with the client lasts until the client gives the resource back.
4. The client gives it back, and then another client can take it.
5. Once all clients finish, the resource is returned to the creator of the server.

Here's a possible interface from the client's point of view:

```par
type MutexServer<a> = iterative choice {
  .end => ?,
  .spawn(dual self) => self,
  .take => (a) choice {
    .put(a) => self,
  }
}
```

Let's break down the `.take` method:
- `.take => (a) choice {`
  
  First, we directly obtain an `a` value, but the server itself turns into a new `choice`, with just one method.
- `.put(a) => self,`
  
  The method is `.put`, which requires an `a` value back. After that, the server returns to the original protocol.

The exclusive session between a client and the server will last until the client puts the value back.

To implement such a server, we need to decide what to do with the value once all clients finish. Here, we just return it to the creator of the server.

```par
type MutexClient<a> = dual MutexServer<a>

dec ShareMutex : [<a> a, MutexClient<a>] a
def ShareMutex = [<a> value, clients] poll(clients) {
  client => client.case {
    .end! => submit(),
    .spawn(l) r => submit(l, r),

    .take session => session(value).case {
      .put(value) client => submit(client),
    }
  }

  else => value,
}
```

A closer look on this part:

```par
    .take session => session(value).case {
      .put(value) client => submit(client),
    }
```

After choosing `.take`, the client (bound as `session`) becomes this function:

```par
[a] either {
  .put(a) MutexClient<a>,
}
```

We send it the current `value`, and then use `.case` to wait until the client gives us the value back. After that, we just submit the client back to the pool.

Using such a server could look like this:

```par
dec IncrementingClient : [Nat] MutexClient<Nat>
def IncrementingClient = [count] chan server {
  Nat.Repeat(count).begin.case {
    .step rest => {
      server.take[n]
      n += 1
      server.put(n)
      rest.loop
    }
    .end! => {
      server.end!
    }
  }
}

def Main: ! = do {
  let final = ShareMutex(0, chan server {
    server.spawn(IncrementingClient(1000))
    server.spawn(IncrementingClient(1000))
    server.spawn(IncrementingClient(1000))
    server.end!
  })
  Debug.Log(`#{final}`)
} in !
```

We spawn 3 clients, each of which will take, increment, and put the value back a 1000 times. In the end, the value 3000 will be printed.

> 💡 **For the theory lovers:** The `MutexServer` above covers the [**_Safe session-based concurrency with shared linear state_**](https://library.oapen.org/bitstream/handle/20.500.12657/63011/1/978-3-031-30044-8.pdf#page=434) paper.
>
> The remaining one of the three papers that `poll`/`submit` subsumes — [**_Concurrency and races in classical linear logic_**](https://cs.au.dk/~birke/phd-students/QianZ-thesis.pdf) — is all about servers like this:
>
> ```par
> type Server = iterative choice {
>   .end => ?,
>   .spawn(dual self) => self,
> 
>   .method1(A1) => (B1) self,
>   .method2(A2) => (B2) self,
>   ...
> }
> ```
>
> Those are clearly covered by `poll`/`submit` as well!

## Example: A chat server in the [Playground](../getting_started.md)

Here's a more involved example of both-ways communication between a server and its clients. It's a little toy chat server that you can play with in the [Par's interactive playground](../getting_started.md).

It's an example of client-to-client mediation via a server.

You can find a thoroughly commented code for this toy chat server in the [`examples`](https://github.com/par-team/par-lang/blob/main/examples/src/PlaygroundChat.par) directory.
