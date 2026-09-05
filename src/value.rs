//! Runtime values for the bytecode VM.
//!
//! `Value` is `Copy`: reference types (strings, closures) live on the GC heap
//! and are held indirectly through a `GcRef`.

use crate::gc::GcRef;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Obj(GcRef),
}

impl Value {
    /// Kindling truthiness: only `nil` and `false` are falsey.
    pub fn is_falsey(self) -> bool {
        matches!(self, Value::Nil | Value::Bool(false))
    }
}

/// A backend-independent, comparable view of a result value. Both the VM and
/// the tree-walking reference interpreter reduce their final answer to this so
/// the differential test can compare them directly.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Func,
}
