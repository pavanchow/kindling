//! Correctness gate 2: round-trip integrity.
//!
//! Two independent transforms must be lossless:
//!   text:   assemble(disassemble(program)) == program
//!   binary: deserialize(serialize(program)) == program
//! and running a program through the binary round trip must produce the same
//! result as running it directly.

use kindling::gen::random_program;
use kindling::{assemble, compile_source, deserialize, disassemble, run_program, serialize};

fn program_count() -> u64 {
    std::env::var("KINDLING_FUZZ_OPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(500)
}

#[test]
fn text_and_binary_round_trips_are_lossless() {
    let count = program_count();
    for seed in 0..count {
        let ops = (seed as usize % 9) + 3;
        let src = random_program(seed, ops);
        let program = compile_source(&src)
            .unwrap_or_else(|e| panic!("seed {seed}: compile error {e}\n{src}"));

        // text round trip
        let text = disassemble(&program);
        let from_text = assemble(&text)
            .unwrap_or_else(|e| panic!("seed {seed}: assemble error {e}\n=== listing ===\n{text}"));
        assert_eq!(
            program, from_text,
            "seed {seed}: text round trip changed the program\n=== listing ===\n{text}"
        );

        // binary round trip
        let bytes = serialize(&program);
        let from_bytes = deserialize(&bytes)
            .unwrap_or_else(|e| panic!("seed {seed}: deserialize error {e}"));
        assert_eq!(
            program, from_bytes,
            "seed {seed}: binary round trip changed the program"
        );
    }
}

#[test]
fn run_after_binary_round_trip_matches_direct_run() {
    let count = program_count();
    for seed in 0..count {
        let ops = (seed as usize % 9) + 3;
        let src = random_program(seed, ops);
        let program = compile_source(&src).unwrap();
        let restored = deserialize(&serialize(&program)).unwrap();

        let direct = run_program(&program);
        let after = run_program(&restored);

        // The round trip must preserve the whole outcome. A widened program may
        // trap on division by zero; then both runs must trap identically, and a
        // value on one side with a trap on the other is a round trip defect.
        match (direct, after) {
            (Ok(d), Ok(a)) => {
                assert_eq!(
                    d.value, a.value,
                    "seed {seed}: serialized run differs from direct run"
                );
                assert_eq!(d.output, a.output, "seed {seed}: output differs");
            }
            (Err(d), Err(a)) => {
                assert_eq!(d, a, "seed {seed}: serialized run trap differs from direct run");
            }
            (Ok(d), Err(a)) => panic!(
                "seed {seed}: direct run produced {:?} but restored run trapped with {a:?}",
                d.value
            ),
            (Err(d), Ok(a)) => panic!(
                "seed {seed}: direct run trapped with {d:?} but restored run produced {:?}",
                a.value
            ),
        }
    }
}
