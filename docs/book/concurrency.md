# Concurrency: threads and channels

Programs on A0 can use every processor (D75). The model is the operating system's threads,
sharing the heap, with channels to pass values between them; the standard library's `std.thread`
(`docs/std.md`) offers it to every language, and a language without modules can import it itself.

```
import std.thread;

fn main() {
  let results: thread.Channel<int> = thread.channel(4);
  let workers: [thread.Thread<int>] = [];
  for k in 0..4 {
    push(workers, thread.spawn(fn() -> int { return count(k); }));
  }
  let total = 0;
  for w in workers {
    total += thread.join(w);
  }
  print(total);
}
```

- `spawn(f)` runs the closure `f` on a thread of its own and returns a handle; `join(t)` waits for
  it and returns what `f` returned. A closure captures variables by reference, so threads can share
  them: races on shared data are the program's to avoid, and channels are the way to.
- `channel(capacity)`, `send(c, v)`, `receive(c)` (an `std.Result`, `Err` once the channel is
  closed and empty) and `close(c)` pass values between threads in order.
- The collector works with every thread: a collection stops them all at safe points (a poll at
  loop heads and function entries, only in programs that start threads, D76), so allocation is
  safe everywhere.
- A panic or an uncaught exception in a thread ends the program with the thread's stack trace and
  where it was spawned; the debugger shows every thread (`docs/debugging.md`).

Threads run at the same time on all processors: `bench/minnow/parallel.mn` counts primes 3.8 times
faster on four threads than on one (`docs/benchmarks.md`). Allocation-heavy threads contend for
the heap's lock, and the playground has no threads (WebAssembly in the browser would need shared
memory and workers).
