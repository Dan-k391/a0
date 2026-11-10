# Concurrency: threads and channels

Programs on A0 can run code on several threads at once, and use every processor. Threads share
the program's memory, and *channels* pass values from one thread to another in order. This chapter
shows a program in [Minnow](../../examples/minnow/README.md), a small typed language, that uses
both, then what a language's compiler does to offer them.

## A program with threads

```
import std.thread;

fn count(from: int, to: int) -> int {
  let n = 0;
  let i = from;
  while i < to {
    if i % 7 == 0 { n = n + 1; }
    i = i + 1;
  }
  return n;
}

fn main() {
  let workers: [thread.Thread<int>] = [];
  let k = 0;
  while k < 4 {
    let start = k * 1000;
    push(workers, thread.spawn(fn() -> int { return count(start, start + 1000); }));
    k = k + 1;
  }
  let total = 0;
  for w in workers {
    total = total + thread.join(w);
  }
  print(total);

  let words: thread.Channel<string> = thread.channel(2);
  let sender = thread.spawn(fn() {
    for w in ["one", "two", "three"] {
      thread.send(words, w);
    }
    thread.close(words);
  });
  let more = true;
  while more {
    match thread.receive(words) {
      Ok(w) => print(w),
      Err(e) => { print(e); more = false; }
    }
  }
  thread.join(sender);
}
```

```
$ minnow run threads.mn
572
one
two
three
the channel is closed
```

The first half counts the multiples of 7 below 4000 on four threads, a quarter each:

- `thread.spawn(f)` runs the closure `f` on a thread of its own, and returns a handle to the
  thread.
- `thread.join(t)` waits for the thread to end, and returns what `f` returned.
- Each closure sees its own `start`, because each pass through the loop makes a new `start`
  variable.

The second half sends three words from one thread to another:

- `thread.channel(2)` makes a channel that holds up to 2 values sent and not yet received. `send`
  waits while the channel is full.
- `thread.receive(c)` waits for the next value. It returns `Ok(value)`, or `Err` once the channel
  is closed and empty.
- `thread.close(c)` says that no more values will be sent.

[The standard library](../std.md#modules) lists these functions with the rest of `std.thread`.

## What a compiler does to offer threads

Threads come from `std.thread`, a module of A0's standard library. Every language can offer it.

**A language with modules does nothing.** When a program says `import std.thread;`, the driver
(the part of A0 that turns a compiler into a command) adds the library's module to the program
instead of looking for a file. That is how Minnow and B0
get threads.

**A language without modules imports the module itself**, once, outside any function body, and
calls its functions by member:

```rust
b.import_module("std.thread", "thread");
// later, for a call of spawn with a closure `f`:
b.call_member("thread", "spawn", &[f]);
```

[The standard library](../std.md#using-it-from-a-language) shows both ways in full.

`std.thread` is written in A0's [high-level IR](../hir.md) (HIR), over a few C functions of A0's
runtime. Values travel between
threads as `any`, so a channel can carry any value except an inline struct
([memory](memory.md#values-in-place-inline-structs)).

A0 compiles a program that uses `std.thread` a little differently from one that does not:

- the exception being thrown is kept per thread, not in a global;
- field lookups on dynamic objects do not use per-place caches, which two threads could tear
  ([dynamic languages](dynamic.md#dynamic-objects));
- with the collector, generated code checks at each loop head and function entry whether a
  collection is waiting. The check is a load and a branch.

A program without threads pays for none of this.

## Sharing data between threads

A closure captures variables by reference, so threads that run closures share the variables they
capture. Races on shared data are the program's to avoid. The runtime keeps its own state
consistent, but not the program's arrays, maps and objects that two threads change at once. Pass
such values through a channel instead.

## The collector and threads

Allocation is safe on every thread. A collection stops every thread first, at a *safe point*: a
place where all of the thread's references are recorded for the collector. A thread reaches one
at the checks above, at an allocation, and while it waits for a channel, another thread, time or
input. Then the collection runs, and every thread goes on.

Once a second thread exists, allocation takes a lock on the heap. Threads that allocate a lot wait
for each other at that lock.

## When a thread fails

A panic or an uncaught exception on any thread ends the program. Its stack trace shows the
thread's own calls, then where the thread was spawned:

```
import std.thread;

fn check(xs: [int], i: int) -> int {
  return xs[i];
}

fn main() {
  let xs = [1, 2, 3];
  let t = thread.spawn(fn() -> int {
    return check(xs, 7);
  });
  print(thread.join(t));
}
```

```
$ minnow run thread_panic.mn
panic: index out of bounds: the length is 3 but the index is 7
    at check (thread_panic.mn:5:10)
    at main::{closure} (thread_panic.mn:11:12)
    at std.thread.spawn<int>::{closure} (<std>/thread.hir:23:54)
    in thread 2, spawned
    at std.thread.spawn<int> (<std>/thread.hir:23:24)
    at main (thread_panic.mn:10:11)
```

The program exits with status 101. The debugger shows every thread, with its stack and variables
([debugging](../debugging.md)).

## How fast

Threads run at the same time on all processors. `bench/minnow/parallel.mn` counts primes 3.8 times
faster on four threads than on one, on a machine with four processors
([performance](../benchmarks.md)).

The browser playground has no threads: WebAssembly in a browser would need shared memory and
workers. A program there that spawns a thread fails with "threads are not supported on this
platform".
