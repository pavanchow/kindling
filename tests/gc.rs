//! Correctness gate 3: garbage collector correctness.
//!
//! With a known reachability graph, a collection must free exactly the
//! unreachable objects and keep every reachable one. A churn test proves there
//! is no use-after-free (a known root is always live) and no leak of known-dead
//! objects. A VM stress test proves the collector never frees a live object
//! during real execution.

use kindling::gc::{Closure, Heap, Obj};
use kindling::run_source;
use kindling::value::Value;
use kindling::vm::Vm;
use kindling::compile_source;

#[test]
fn known_reachable_set_survives_and_dead_set_freed() {
    let mut heap = Heap::new();

    // Reachable graph: a closure that captures two strings, plus a standalone
    // reachable string. Everything else is garbage.
    let s1 = heap.alloc_str("captured-one".into());
    let s2 = heap.alloc_str("captured-two".into());
    let closure = heap.alloc(Obj::Closure(Closure {
        func: 0,
        upvalues: vec![Value::Obj(s1), Value::Obj(s2)],
    }));
    let standalone = heap.alloc_str("standalone".into());

    let mut dead = Vec::new();
    for i in 0..200 {
        dead.push(heap.alloc_str(format!("garbage-{i}")));
    }

    assert_eq!(heap.live_count(), 204);

    let roots = [Value::Obj(closure), Value::Obj(standalone)];
    let freed = heap.collect(&roots);

    assert_eq!(freed, 200, "every unreachable object must be freed");
    assert_eq!(heap.live_count(), 4, "only the reachable graph survives");

    // No reachable object was collected (no use-after-free of live data).
    assert!(heap.is_live(closure));
    assert!(heap.is_live(standalone));
    assert!(heap.is_live(s1), "captured object reachable through closure");
    assert!(heap.is_live(s2), "captured object reachable through closure");

    // Every known-dead object is actually gone.
    for d in &dead {
        assert!(!heap.is_live(*d), "known-dead object leaked");
    }
}

#[test]
fn churn_never_frees_the_live_root() {
    let mut heap = Heap::new();
    let root = heap.alloc_str("permanent-root".into());

    for round in 0..50 {
        // allocate a pile of short-lived garbage
        for i in 0..100 {
            heap.alloc_str(format!("r{round}-{i}"));
        }
        let before = heap.live_count();
        let freed = heap.collect(&[Value::Obj(root)]);
        assert!(heap.is_live(root), "root freed during churn round {round}");
        assert_eq!(heap.live_count(), 1, "only the root should survive");
        assert_eq!(freed, before - 1, "all garbage should be freed");
    }
}

#[test]
fn vm_result_is_correct_under_gc_stress() {
    // A loop that allocates a fresh string every iteration. With GC stress on,
    // the collector runs almost every allocation. The captured accumulator and
    // globals must never be collected, so the final result stays correct.
    let src = r#"
        let s = "";
        let i = 0;
        while (i < 30) {
            s = s + "ab";
            i = i + 1;
        }
        return s;
    "#;

    let expected = "ab".repeat(30);

    let program = compile_source(src).unwrap();
    let mut vm = Vm::new();
    vm.set_gc_stress(true);
    let value = vm.interpret(&program).unwrap();
    match vm.to_outcome(value) {
        kindling::Outcome::Str(s) => assert_eq!(s, expected),
        other => panic!("expected string result, got {other:?}"),
    }

    // Sanity: the normal (non-stress) path agrees.
    let normal = run_source(src).unwrap();
    assert_eq!(normal.value, kindling::Outcome::Str(expected));
}
