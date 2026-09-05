//! Correctness gate 1: differential testing.
//!
//! For many randomly generated programs, the bytecode VM's result must equal the
//! independent tree-walking reference interpreter's result. Two independent
//! evaluators agreeing is the machine-checkable oracle.
//!
//! Program count is controlled by `KINDLING_FUZZ_OPS` (default 500).

use kindling::gen::random_program;
use kindling::{eval_reference, run_source};

fn program_count() -> u64 {
    std::env::var("KINDLING_FUZZ_OPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(500)
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
