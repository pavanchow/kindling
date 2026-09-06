//! Correctness gate 3: garbage collector correctness.
//!
//! With a known reachability graph, a collection must free exactly the
//! unreachable objects and keep every reachable one. A churn test proves there
//! is no use-after-free (a known root is always live) and no leak of known-dead
//! objects. A VM stress test proves the collector never frees a live object
//! during real execution.

use kindling::gc::{Closure, Heap, Obj, Upvalue};
use kindling::run_source;
use kindling::value::Value;
use kindling::vm::Vm;
use kindling::compile_source;

#[test]
fn known_reachable_set_survives_and_dead_set_freed() {
    let mut heap = Heap::new();

    // Reachable graph: a closure that captures two strings through upvalue cells,
    // plus a standalone reachable string. Reachability is three levels deep
    // (closure -> upvalue cell -> string), so the mark phase must trace through
    // the cells. Everything else is garbage.
    let s1 = heap.alloc_str("captured-one".into());
    let s2 = heap.alloc_str("captured-two".into());
    let u1 = heap.alloc(Obj::Upvalue(Upvalue::Closed(Value::Obj(s1))));
    let u2 = heap.alloc(Obj::Upvalue(Upvalue::Closed(Value::Obj(s2))));
    let closure = heap.alloc(Obj::Closure(Closure {
        func: 0,
        upvalues: vec![u1, u2],
    }));
    let standalone = heap.alloc_str("standalone".into());

    let mut dead = Vec::new();
    for i in 0..200 {
        dead.push(heap.alloc_str(format!("garbage-{i}")));
    }

    assert_eq!(heap.live_count(), 206);

    let roots = [Value::Obj(closure), Value::Obj(standalone)];
    let freed = heap.collect(&roots);

    assert_eq!(freed, 200, "every unreachable object must be freed");
    assert_eq!(heap.live_count(), 6, "only the reachable graph survives");

    // No reachable object was collected (no use-after-free of live data).
    assert!(heap.is_live(closure));
    assert!(heap.is_live(standalone));
    assert!(heap.is_live(u1), "upvalue cell reachable through closure");
    assert!(heap.is_live(u2), "upvalue cell reachable through closure");
    assert!(heap.is_live(s1), "captured object reachable through closure -> cell");
    assert!(heap.is_live(s2), "captured object reachable through closure -> cell");

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

#[test]
fn closures_survive_gc_stress() {
    // Closures capture live, mutable upvalue cells while a churn of short-lived
    // strings forces collection on nearly every allocation. If the collector
    // ever freed a live upvalue cell (a use-after-free) the captured counter
    // would be wrong or the run would panic. The result must match the reference
    // interpreter, which does not share the VM's heap.
    let src = r#"
        fn make() {
            let n = 0;
            fn step() {
                let junk = "tmp" + "junk";
                n = n + len(junk);
                return n;
            }
            return step;
        }
        let s = make();
        let total = 0;
        let i = 0;
        while (i < 40) {
            total = s();
            i = i + 1;
        }
        return total;
    "#;

    let program = compile_source(src).unwrap();
    let mut vm = Vm::new();
    vm.set_gc_stress(true);
    let value = vm.interpret(&program).unwrap();
    let stressed = vm.to_outcome(value);

    let reference = kindling::eval_reference(src).unwrap();
    assert_eq!(
        stressed, reference.value,
        "closure result under GC stress must match the reference interpreter"
    );
    assert_eq!(stressed, kindling::Outcome::Int(40 * 7));
}
