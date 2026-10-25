// Functions, recursion, strings and control flow.
fn fib(n: int) -> int {
  if n < 2 {
    return n;
  }
  return fib(n - 1) + fib(n - 2);
}

fn greet(who: string) -> string {
  return "hello, " + who;
}

fn main() {
  print(fib(20));
  print(greet("pebble"));
  let x = 7;
  if x % 2 == 1 && x > 5 {
    print("odd and big");
  } else {
    print("not");
  }
  let i = 0;
  while i < 3 {
    i = i + 1;
  }
  print(i * 2 > 4 || false);
}
// expect-stdout: 6765
// expect-stdout: hello, pebble
// expect-stdout: odd and big
// expect-stdout: true
