// Arrays: literals, `for`, indexing, `push` and `len`, and arrays as arguments.
fn sum(xs: [int]) -> int {
  let total = 0;
  for x in xs {
    total = total + x;
  }
  return total;
}

fn squares(n: int) -> [int] {
  let out = [];
  let i = 1;
  while i <= n {
    push(out, i * i);
    i = i + 1;
  }
  return out;
}

fn main() {
  let xs = [3, 1, 4, 1, 5];
  print(sum(xs));
  xs[0] = 10;
  print(xs[0] + xs[4]);
  let sq = squares(5);
  print(len(sq));
  for s in sq {
    for t in [s] {
      print("square " + str(t));
    }
  }
}
// expect-stdout: 14
// expect-stdout: 15
// expect-stdout: 5
// expect-stdout: square 1
// expect-stdout: square 4
// expect-stdout: square 9
// expect-stdout: square 16
// expect-stdout: square 25
