# Floating point: the tutorial's Mandelbrot iteration over a 1000 x 1000 grid, summed (chapter 7).
extern printd(x);

def binary : 1 (x y) y;
def binary > 10 (LHS RHS) RHS < LHS;
def binary | 5 (LHS RHS)
  if LHS then 1 else if RHS then 1 else 0;

def mandelconverger(real imag iters creal cimag)
  if iters > 255 | (real*real + imag*imag > 4) then
    iters
  else
    mandelconverger(real*real - imag*imag + creal, 2*real*imag + cimag, iters+1, creal, cimag);

def mandelconverge(real imag)
  mandelconverger(real, imag, 0, real, imag);

# Kaleidoscope has no division: the steps come in.
def mandelsum(xmin ymin steps xstep ystep)
  var total = 0 in
    (for y = 0, y < steps in
      (for x = 0, x < steps in
        total = total + mandelconverge(xmin + x * xstep, ymin + y * ystep))) :
    total;

printd(mandelsum(0-2.3, 0-1.3, 1000, 0.003, 0.0025));
