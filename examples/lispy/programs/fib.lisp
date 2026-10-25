; Recursion, loops and printing in parentheses.
(defn fib (n)
  (if (< n 2)
    (return n))
  (return (+ (fib (- n 1)) (fib (- n 2)))))

(defn countdown (n)
  (while (> n 0)
    (print "T minus" n)
    (set! n (- n 1)))
  (print "liftoff"))

(print (fib 20))
(countdown 3)
(let total 0)
(repeat 4 (set! total (+ total (sqrt 16))))
(print "total:" total)
(print (and (< 1 2) (== total 16)))
; expect-stdout:
; 6765
; T minus 3
; T minus 2
; T minus 1
; liftoff
; total: 16
; true
