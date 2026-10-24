# Mutable variables, scoping, loops and operator precedence.
# expect: 6.000000
# expect: 1.000000
# expect: 10.000000
# expect: 3.000000
# expect: 15.000000
# expect: 0.000000
# expect: 7.000000
# expect: 2.000000
# expect: -5.000000
# expect: 1.000000
# expect: 0.000000

extern printd(x);
extern sqrt(x);

def binary : 1 (x y) y;
def unary-(v) 0-v;

# Parameters are mutable.
def double(x) x = x * 2 : x;

# 'var' initializers see the outer binding, not their own.
def shadow(a)
  var a = a + 1 in
    var a = a * 2, b = a in b;

# The loop variable shadows an outer one only inside the loop.
def loopscope(i)
  (for i = 0, i < 5 in 0) : i;

# The end condition is checked after the body with the old value, so 'i < n' still runs the body
# for i = n: this sums 0 + 1 + ... + n.
def sum(n)
  var total = 0 in
  (for i = 0, i < n, 1 in total = total + i) : total;

printd(double(3));
printd(1 < 2);
printd(shadow(4));
printd(loopscope(3));
printd(sum(5));
printd(2 < 1);
# Precedence: * binds tighter than +, + tighter than <.
printd(1 + 2 * 3);
printd(sqrt(4));
printd(-5);
# A for loop body runs at least once, and the loop evaluates to 0.
printd(var n = 0 in (for i = 0, 0 in n = n + 1) : n);
printd(for i = 0, i < 3 in 0);
