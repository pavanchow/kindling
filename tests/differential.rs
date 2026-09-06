//! Correctness gate 1: differential testing.
//!
//! For many randomly generated programs, the bytecode VM's result must equal the
//! independent tree-walking reference interpreter's result. Two independent
//! evaluators agreeing is the machine-checkable oracle.
//!
//! Program count is controlled by `KINDLING_FUZZ_OPS` (default 500).

use kindling::gen::random_program;
use kindling::{compile_source, eval_reference, run_source};

fn program_count() -> u64 {
    std::env::var("KINDLING_FUZZ_OPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(500)
}

/// Assert the bytecode VM and the reference interpreter agree on the whole
/// outcome of `src`, whether that is a value or a trap.
fn assert_agree(src: &str) {
    match (run_source(src), eval_reference(src)) {
        (Ok(v), Ok(r)) => {
            assert_eq!(v.value, r.value, "value mismatch for: {src}");
            assert_eq!(v.output, r.output, "output mismatch for: {src}");
        }
        (Err(ve), Err(re)) => assert_eq!(ve, re, "error mismatch for: {src}"),
        (vm, reference) => panic!("one side trapped and the other did not for: {src}\nVM={vm:?} REF={reference:?}"),
    }
}

#[test]
fn vm_matches_reference_interpreter() {
    let count = program_count();
    let mut checked = 0u64;
    for seed in 0..count {
        let ops = (seed as usize % 9) + 3;
        let src = random_program(seed, ops);

        let vm = run_source(&src);
        let reference = eval_reference(&src);

        // The two evaluators must agree on the whole outcome, including whether
        // the program traps. A widened program may divide by zero, and then both
        // must return the same runtime error; either one succeeding while the
        // other traps is a divergence.
        match (vm, reference) {
            (Ok(v), Ok(r)) => {
                assert_eq!(
                    v.value, r.value,
                    "seed {seed}: value mismatch\nVM={:?} REF={:?}\n--- program ---\n{src}",
                    v.value, r.value
                );
                assert_eq!(
                    v.output, r.output,
                    "seed {seed}: output mismatch\n--- program ---\n{src}"
                );
            }
            (Err(ve), Err(re)) => {
                assert_eq!(
                    ve, re,
                    "seed {seed}: error mismatch\nVM={ve:?} REF={re:?}\n--- program ---\n{src}"
                );
            }
            (Ok(v), Err(re)) => panic!(
                "seed {seed}: VM produced {:?} but reference trapped with {re:?}\n--- program ---\n{src}",
                v.value
            ),
            (Err(ve), Ok(r)) => panic!(
                "seed {seed}: VM trapped with {ve:?} but reference produced {:?}\n--- program ---\n{src}",
                r.value
            ),
        }
        checked += 1;
    }
    assert_eq!(checked, count);
    eprintln!("differential: {checked} programs, VM == reference on all");
}

#[test]
fn hand_written_programs_agree() {
    let cases = [
        "return 2 + 3 * 4;",
        "let x = 10; let y = 20; return x * y - 5;",
        "let s = 0; let i = 1; while (i <= 100) { s = s + i; i = i + 1; } return s;",
        "fn fib(n) { if (n < 2) { return n; } return fib(n-1) + fib(n-2); } return fib(15);",
        "fn fact(n) { if (n <= 1) { return 1; } return n * fact(n-1); } return fact(10);",
        "let a = 7; if (a % 2 == 0) { a = a * 2; } else { a = a * 3; } return a;",
        "fn add(a, b) { return a + b; } fn mul(a, b) { return a * b; } return add(mul(2,3), 4);",
        "let x = -5; return -x + 10;",
        "return (1 < 2) == (3 > 2);",
        "let acc = 0; let i = 0; while (i < 10) { if (i % 3 == 0) { acc = acc + i; } i = i + 1; } return acc;",
    ];
    for (n, src) in cases.iter().enumerate() {
        let vm = run_source(src).unwrap_or_else(|e| panic!("case {n}: VM error {e}"));
        let reference = eval_reference(src).unwrap_or_else(|e| panic!("case {n}: ref error {e}"));
        assert_eq!(vm.value, reference.value, "case {n}: {src}");
    }
}

/// Regression cases for closure capture semantics. Each of these once diverged
/// (or panicked) because the VM captured upvalues by value; both evaluators must
/// now agree, closing over live, mutable, shared, and self-referential state.
#[test]
fn closure_semantics_agree() {
    // A self-referential local function once panicked in the VM.
    assert_agree("fn outer() { fn inner(n) { if (n <= 0) { return 0; } return inner(n-1) + n; } return inner(5); } return outer();");
    // A closure that reassigns a captured local must affect the enclosing scope.
    assert_agree("fn outer() { let c = 0; fn inc() { c = c + 1; return c; } inc(); inc(); return c; } return outer();");
    // A mutation to a captured variable made after capture must be observed.
    assert_agree("fn outer() { let x = 1; fn get() { return x; } x = 99; return get(); } return outer();");
    // Two closures over one variable must share a single cell.
    assert_agree("fn outer() { let x = 10; fn a() { x = x + 5; return x; } fn b() { return x; } a(); return b(); } return outer();");
    // A closure over a block-scoped local that escapes the block.
    assert_agree("fn outer(){ let g = 0; { let x = 7; fn get(){ return x; } g = get(); } return g; } return outer();");
    // A counter factory: the returned closure keeps mutating its own captured n.
    assert_agree("fn mk(){ let n = 0; fn step(){ n = n + 1; return n; } return step; } let s = mk(); s(); s(); return s();");
    // Mutual recursion through global names.
    assert_agree("fn e(n){ if(n==0){return true;} return o(n-1);} fn o(n){ if(n==0){return false;} return e(n-1);} return e(10);");
}

/// Regression cases for the standard-library builtins. The two evaluators
/// implement them independently and must produce identical results and traps.
#[test]
fn builtins_agree() {
    assert_agree("return abs(-5);");
    assert_agree("return abs(5);");
    assert_agree("return abs(-3.5);");
    assert_agree("return min(3, 7) + max(3, 7);");
    assert_agree("return min(2, 9.0);");
    assert_agree("return max(2.5, 2);");
    assert_agree("return len(\"kindling\");");
    assert_agree("return len(\"\");");
    assert_agree("return len(\"ab\" + \"cde\");");
    // Trap agreement: wrong arity and wrong argument types.
    assert_agree("return abs(1, 2);");
    assert_agree("return len(5);");
    assert_agree("return min(1);");
    assert_agree("return abs(\"x\");");
    // A builtin bound as a value, then called.
    assert_agree("let f = abs; return f(-8);");
    // A user global shadows the builtin in both evaluators.
    assert_agree("let abs = 3; return abs;");
}

/// Adversarial input must trap cleanly (a recoverable error), never overflow the
/// process stack, and the two evaluators must agree on the trap.
#[test]
fn adversarial_input_traps_cleanly() {
    // Deep nesting is rejected by the parser before it can overflow.
    let deep_parens = format!("return {}1{};", "(".repeat(5000), ")".repeat(5000));
    assert!(compile_source(&deep_parens).is_err(), "deep parens should error");
    assert_agree(&deep_parens);

    let deep_blocks = format!("{}{}", "{".repeat(5000), "}".repeat(5000));
    assert!(compile_source(&deep_blocks).is_err(), "deep blocks should error");

    let deep_unary = format!("return {}1;", "-".repeat(5000));
    assert!(compile_source(&deep_unary).is_err(), "deep unary should error");

    // A long left-associative chain is accepted by the parser but bounded by the
    // compiler and the interpreter, which must agree.
    let deep_chain = format!("return 1{};", "+1".repeat(5000));
    assert_agree(&deep_chain);

    // Runaway recursion traps on the call depth limit in both evaluators.
    assert_agree("fn f(n){ if(n<=0){return 0;} return f(n-1)+1; } return f(100000);");
}
