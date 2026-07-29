# Nondeterminism, Servers & Clients

In an [episode of the "Type Theory Forall" podcast](https://www.typetheoryforall.com/episodes/goal-of-science), Phil Wadler said:

> **❝Linear logic is good for the bits of concurrency where you don't need concurrency.❞**
>
> — Phil Wadler, 2025

Thanks Phil, that's very encouraging.

What Phil — the wonderful researcher and author of the [process language Par is based on](https://homepages.inf.ed.ac.uk/wadler/papers/propositions-as-sessions/propositions-as-sessions.pdf) — is talking about here is nondeterminism and races. Indeed, linear logic and races are like water and oil. The research has been largely unsatisfying, yielding steps in certain directions, but never quite far enough to solve real-world problems in an elegant and obviously correct manner.

**Par offers an innovative solution!**

## What is nondeterminism?

All features of Par that we've covered so far are deterministic. Yes, Par is an automatically concurrent language. Independent bits of your program proceed concurrently, without you needing to lift a finger to make it happen. But when it comes to each single piece, the next instruction clearly says who to talk to, and what to say. Decisions are being made, but they only depend on the inputs. If the inputs have the same values, the outcomes will be the same.

But sometimes, quite often in fact, decisions need to be made based on _who speaks first._ That's the race, to speak first. A program gathering information from multiple slow sources can't say _"I'll first listen to A, then to B."_ If _A_ takes 30 minutes to produce its first message, while _B_ has already produced 50 of them, the program just won't work well.

**That's nondeterminism.** It's not determined if I'll speak to _A_ or _B_ first, it depends! In this case, it depends on **timing.**

> We're not talking about randomness, or "nondeterministic choice" here. While those fall under the general umbrella of nondeterminism, the randomness can be understood as a part of the program input. The nondeterminism we are interested in here is concerned with timing.

**The problem-space of nondeterminism** is quite large, but it mainly comes in **two kinds:**
- **Bidirectional communication** — That's when _A_ and _B_ have a channel _between_ them, but either of them can speak first. If _A_ speaks first, _B_ must react, and vice versa. This is useful for preemptive cancellation, implementing chat servers or real-time dashboards without periodic pings, and so on.
- **Server and clients** — I don't mean web servers. What I mean is structuring your program as multiple independent agents — clients — that communicate with a central, stateful agent: the server. The server pays attention to one client at a time, but always the one who speaks first.

> Par does not solve the first, yet. You can have cancellation, but it has to be cooperative, you can write a chat server, but there's got to be periodic checks.

**Par solves the second: server and clients,** while:
- Keeping all the guarantees: **no deadlocks, no runtime crashes, no infinite loops.**
- Leveraging Par's types as **session types to allow arbitrary communication protocols** between the server and the clients.
- **Being very simple to understand and reason about!**

> 👉 Just to make sure it's clear... we're **not** talking about web servers here. We're talking about a concurrent structure, where any number of _client_ agents are communicating with a central _server_ agent, all inside a single program.

## The state of the research

When it comes to linear logic — the theory underlying Par — the problem of races is, to my best knowledge, unsolved in research. Here are some papers that shaped my understanding of the problem, and also why they fall short:

- [**_Client-server sessions in linear logic_**](https://dl.acm.org/doi/abs/10.1145/3473567) — This paper is spectacularly elegant. It invents coexponentials, which partially solve the client-server communication topology. Unfortunately, the clients are not resumable.
- [**_Concurrency and races in classical linear logic_**](https://cs.au.dk/~birke/phd-students/QianZ-thesis.pdf) — This is a follow-up paper, which tries to solve the non-resumability of the clients by introducing single input, single output interfaces for clients to communicate with a central server. Unfortunately, it's not as elegant, and still not expressive enough.
- [**_Safe session-based concurrency with shared linear state_**](https://library.oapen.org/bitstream/handle/20.500.12657/63011/1/978-3-031-30044-8.pdf#page=434) — This one is fairly simple, yet surprisingly potent. It introduces a deadlock-safe mutex type, that can be used to safely share a linear resource among independent processes. Unfortunately, it does not extend well to more involved communication protocols.
- [**_Towards races in linear logic_**](https://lmcs.episciences.org/6979/pdf) — An extremely elegant approach presented with colorful formulas and icons. It fits very well into linear logic, but unfortunately is restricted to servers with a constant number of clients, which finds few use-cases.

**Par's solution here,** its `poll`/`submit` control structure, can be used to implement everything from the first 3 papers above, and a lot more, with very few ingredients.

The last paper is able to express some invariants that `poll`/`submit` cannot (the constant number of clients), but it's questionable how useful those could be in real-world programs.
