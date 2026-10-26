//! Operator precedence resolution for operator tables that are only known after parsing.
//!
//! Grammars with a fixed operator table use `@prec`. Languages whose programs define operators (like
//! Kaleidoscope's `def binary|`) parse a flat chain `x0 op0 x1 op1 x2 ...` instead and resolve it with
//! [`resolve_precedence`] once the table is known.

/// A reference to an operand of a [`Combine`] step: an original operand (`is_step == false`) or the
/// result of an earlier step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Operand {
    pub index: u32,
    pub is_step: bool,
}

impl Operand {
    pub fn leaf(index: u32) -> Operand {
        Operand {
            index,
            is_step: false,
        }
    }

    pub fn step(index: u32) -> Operand {
        Operand {
            index,
            is_step: true,
        }
    }
}

/// One binary operation: `left op right`, where `op` indexes the chain's operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Combine {
    pub left: Operand,
    pub op: u32,
    pub right: Operand,
}

/// Resolves the chain `x0 op0 x1 op1 ... xn` into a tree of binary operations.
///
/// `precedences[i]` is operator `i`'s binding strength (higher binds tighter) and `right_assoc[i]`
/// whether it groups to the right. The result lists the operations so that every step only refers to
/// original operands and earlier steps; the last step is the root (there are no steps for a single
/// operand). To evaluate operands in source order, walk the tree recursively from the root.
pub fn resolve_precedence(precedences: &[i32], right_assoc: &[bool]) -> Vec<Combine> {
    assert_eq!(precedences.len(), right_assoc.len());
    let mut steps = Vec::with_capacity(precedences.len());
    let mut operands = vec![Operand::leaf(0)];
    let mut ops: Vec<u32> = Vec::new();
    let reduce = |steps: &mut Vec<Combine>, operands: &mut Vec<Operand>, op: u32| {
        let right = operands.pop().unwrap();
        let left = operands.pop().unwrap();
        steps.push(Combine { left, op, right });
        operands.push(Operand::step(steps.len() as u32 - 1));
    };
    for i in 0..precedences.len() {
        while let Some(&top) = ops.last() {
            let (pt, pi) = (precedences[top as usize], precedences[i]);
            if pt > pi || (pt == pi && !right_assoc[i]) {
                ops.pop();
                reduce(&mut steps, &mut operands, top);
            } else {
                break;
            }
        }
        ops.push(i as u32);
        operands.push(Operand::leaf(i as u32 + 1));
    }
    while let Some(op) = ops.pop() {
        reduce(&mut steps, &mut operands, op);
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders the tree rooted at the last step, with operands named a, b, c, ...
    fn render(steps: &[Combine], ops: &[&str]) -> String {
        fn go(steps: &[Combine], ops: &[&str], o: Operand) -> String {
            if o.is_step {
                let s = steps[o.index as usize];
                format!(
                    "({} {} {})",
                    go(steps, ops, s.left),
                    ops[s.op as usize],
                    go(steps, ops, s.right)
                )
            } else {
                ((b'a' + o.index as u8) as char).to_string()
            }
        }
        match steps.last() {
            None => "a".to_string(),
            Some(_) => go(steps, ops, Operand::step(steps.len() as u32 - 1)),
        }
    }

    #[test]
    fn precedence_and_associativity() {
        // a + b * c - d
        let steps = resolve_precedence(&[20, 40, 20], &[false, false, false]);
        assert_eq!(render(&steps, &["+", "*", "-"]), "((a + (b * c)) - d)");
        // a = b = c (right associative)
        let steps = resolve_precedence(&[2, 2], &[true, true]);
        assert_eq!(render(&steps, &["=", "="]), "(a = (b = c))");
        // a < b | c with | weaker than <
        let steps = resolve_precedence(&[10, 5], &[false, false]);
        assert_eq!(render(&steps, &["<", "|"]), "((a < b) | c)");
        assert!(resolve_precedence(&[], &[]).is_empty());
    }
}
