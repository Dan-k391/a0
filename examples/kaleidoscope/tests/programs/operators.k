# User-defined operators with precedence, including recursion through an operator.
# expect: 1.000000
# expect: 0.000000
# expect: 13.000000
# expect: 8.000000
# expect: 120.000000

extern printd(x);

def unary!(v) if v then 0 else 1;
def binary> 10 (l r) r < l;
def binary| 5 (l r) if l then 1 else if r then 1 else 0;
def binary& 6 (l r) if !l then 0 else !!r;

# Exponentiation, binding tighter than '*'.
def binary^ 50 (base exp)
  if exp < 1 then 1 else base * (base ^ (exp - 1));

# Factorial as a prefix operator that calls itself.
def unary~(n) if n < 2 then 1 else n * ~(n - 1);

printd(3 > 2 & 1 > 0 | 0);
printd(!1);
printd(1 + 3 * 2 ^ 2);
printd(2 ^ 3);
printd(~5);
