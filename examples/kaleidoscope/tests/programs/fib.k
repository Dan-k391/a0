# Chapter 7 of the tutorial: recursive and iterative fib, ':' as a sequencing operator.
# expect: 6765.000000
# expect: 55.000000
# expect: 832040.000000

extern printd(x);

# Define ':' for sequencing: as a low-precedence operator that ignores operands
# and just returns the RHS.
def binary : 1 (x y) y;

# Recursive fib, we could do this before.
def fib(x)
  if (x < 3) then
    1
  else
    fib(x-1)+fib(x-2);

# Iterative fib.
def fibi(x)
  var a = 1, b = 1, c in
  (for i = 3, i < x in
     c = a + b :
     a = b :
     b = c) :
  b;

printd(fib(20));
printd(fibi(10));
printd(fibi(30));
