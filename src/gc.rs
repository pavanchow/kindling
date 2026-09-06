//! A precise mark-and-sweep garbage collector.
//!
//! Every heap object lives in a slot addressed by a `GcRef`. Collection resets
//! all marks, marks everything transitively reachable from a set of root
//! values, then frees any slot that was not marked. Freed slots are recycled by
//! later allocations. Because marking traverses the child references contained
//! in each object (a closure's captured upvalues, and the value inside a closed
//! upvalue), no reachable object is ever freed, and every unreachable object is.

use crate::value::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GcRef(pub usize);

/// A captured variable, shared between the enclosing frame and every closure
/// that captures it. While the variable is still live on the value stack the
/// upvalue is `Open` and holds the stack index it points at, so reads and writes
/// go straight through to the live slot. When the slot is about to disappear
/// (its function returns or its block ends) the upvalue is `Closed`: the current
/// value is lifted onto the heap and the closure keeps working afterwards. This
/// is what makes captured mutable state, self-referential local functions, and
/// two closures sharing one variable behave the same as the reference
/// interpreter's shared environments.
#[derive(Debug)]
pub enum Upvalue {
    Open(usize),
    Closed(Value),
}

/// A runtime closure: an index into the program's function table plus the
/// upvalue cells captured from enclosing scopes. Each entry points at an
/// `Obj::Upvalue`, so capture is by reference, not by value.
#[derive(Clone, Debug)]
pub struct Closure {
    pub func: usize,
    pub upvalues: Vec<GcRef>,
}

/// A built-in function implemented in Rust rather than in Kindling bytecode.
/// Both evaluators expose the same set under the same global names and compute
/// identical results, so the differential gate covers them like any other call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Native {
    Abs,
    Min,
    Max,
    Len,
}

impl Native {
    /// The global name this builtin is bound to.
    pub fn name(self) -> &'static str {
        match self {
            Native::Abs => "abs",
            Native::Min => "min",
            Native::Max => "max",
            Native::Len => "len",
        }
    }

    /// The number of arguments this builtin takes.
    pub fn arity(self) -> usize {
        match self {
            Native::Abs | Native::Len => 1,
            Native::Min | Native::Max => 2,
        }
    }

    /// Every builtin, for registration into the global namespace.
    pub const ALL: [Native; 4] = [Native::Abs, Native::Min, Native::Max, Native::Len];
}

#[derive(Debug)]
pub enum Obj {
    Str(String),
    Closure(Closure),
    Upvalue(Upvalue),
    Native(Native),
}

impl Obj {
    /// The GC references directly contained by this object.
    fn children(&self, out: &mut Vec<GcRef>) {
        match self {
            Obj::Closure(c) => {
                for uv in &c.upvalues {
                    out.push(*uv);
                }
            }
            // An open upvalue's value lives on the value stack, which the VM
            // roots directly, so only a closed upvalue owns a value to trace.
            Obj::Upvalue(Upvalue::Closed(Value::Obj(r))) => out.push(*r),
            // Strings, open or nil-closed upvalues, and natives own no traceable
            // references.
            Obj::Str(_) | Obj::Upvalue(_) | Obj::Native(_) => {}
        }
    }
}

#[derive(Default)]
pub struct Heap {
    slots: Vec<Option<Obj>>,
    marks: Vec<bool>,
    free: Vec<usize>,
    /// Objects allocated since the last collection (drives auto-GC).
    since_gc: usize,
    /// Threshold of live objects that triggers the next auto-GC.
    pub next_gc: usize,
    /// When set, collect after almost every allocation. Used to shake out
    /// use-after-free bugs in tests.
    pub stress: bool,
}

impl Heap {
    pub fn new() -> Self {
        Heap {
            slots: Vec::new(),
            marks: Vec::new(),
            free: Vec::new(),
            since_gc: 0,
            next_gc: 128,
            stress: false,
        }
    }

    pub fn alloc(&mut self, obj: Obj) -> GcRef {
        self.since_gc += 1;
        if let Some(i) = self.free.pop() {
            self.slots[i] = Some(obj);
            self.marks[i] = false;
            GcRef(i)
        } else {
            self.slots.push(Some(obj));
            self.marks.push(false);
            GcRef(self.slots.len() - 1)
        }
    }

    pub fn alloc_str(&mut self, s: String) -> GcRef {
        self.alloc(Obj::Str(s))
    }

    pub fn get(&self, r: GcRef) -> &Obj {
        self.slots[r.0]
            .as_ref()
            .expect("dereferenced a freed GcRef")
    }

    pub fn get_mut(&mut self, r: GcRef) -> &mut Obj {
        self.slots[r.0]
            .as_mut()
            .expect("dereferenced a freed GcRef")
    }

    pub fn is_live(&self, r: GcRef) -> bool {
        self.slots.get(r.0).is_some_and(Option::is_some)
    }

    /// Number of currently allocated (unfreed) objects.
    pub fn live_count(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    /// Whether an auto-GC should run given the objects allocated since the last
    /// collection.
    pub fn should_collect(&self) -> bool {
        self.since_gc >= self.next_gc
    }

    /// Mark from `roots`, sweep everything unreachable. Returns the number of
    /// objects freed.
    pub fn collect(&mut self, roots: &[Value]) -> usize {
        for m in &mut self.marks {
            *m = false;
        }

        let mut worklist: Vec<GcRef> = Vec::new();
        for v in roots {
            if let Value::Obj(r) = v {
                worklist.push(*r);
            }
        }

        while let Some(r) = worklist.pop() {
            if self.marks[r.0] {
                continue;
            }
            self.marks[r.0] = true;
            if let Some(obj) = &self.slots[r.0] {
                obj.children(&mut worklist);
            }
        }

        let mut freed = 0;
        for i in 0..self.slots.len() {
            if self.slots[i].is_some() && !self.marks[i] {
                self.slots[i] = None;
                self.free.push(i);
                freed += 1;
            }
        }

        self.since_gc = 0;
        let live = self.live_count();
        self.next_gc = if self.stress { 1 } else { (live * 2).max(128) };
        freed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frees_unreachable_keeps_reachable() {
        let mut heap = Heap::new();
        let keep = heap.alloc_str("keep".into());
        let _drop1 = heap.alloc_str("drop1".into());
        let _drop2 = heap.alloc_str("drop2".into());
        assert_eq!(heap.live_count(), 3);

        let freed = heap.collect(&[Value::Obj(keep)]);
        assert_eq!(freed, 2);
        assert_eq!(heap.live_count(), 1);
        assert!(heap.is_live(keep));
    }

    #[test]
    fn marks_through_closure_upvalues() {
        let mut heap = Heap::new();
        let captured = heap.alloc_str("captured".into());
        let cell = heap.alloc(Obj::Upvalue(Upvalue::Closed(Value::Obj(captured))));
        let closure = heap.alloc(Obj::Closure(Closure {
            func: 0,
            upvalues: vec![cell],
        }));
        let _garbage = heap.alloc_str("garbage".into());

        let freed = heap.collect(&[Value::Obj(closure)]);
        assert_eq!(freed, 1);
        assert!(heap.is_live(cell), "the upvalue cell is reachable via the closure");
        assert!(
            heap.is_live(captured),
            "reachable via closure -> upvalue cell -> string"
        );
        assert!(heap.is_live(closure));
    }

    #[test]
    fn recycles_freed_slots() {
        let mut heap = Heap::new();
        let a = heap.alloc_str("a".into());
        heap.collect(&[]);
        assert!(!heap.is_live(a));
        let b = heap.alloc_str("b".into());
        // the freed slot index is reused
        assert_eq!(a.0, b.0);
    }
}
