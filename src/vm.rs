//! The stack-based bytecode virtual machine.

use crate::chunk::{Constant, Program};
use crate::compiler::{CALL_DEPTH_ERROR, MAX_CALL_DEPTH};
use crate::gc::{Closure, GcRef, Heap, Native, Obj, Upvalue};
use crate::opcode::*;
use crate::value::{Outcome, Value};

struct Frame {
    closure: GcRef,
    func: usize,
    ip: usize,
    slot_base: usize,
}

pub struct Vm {
    stack: Vec<Value>,
    frames: Vec<Frame>,
    globals: std::collections::HashMap<String, Value>,
    heap: Heap,
    output: String,
    /// Upvalues that still point at a live value-stack slot, newest last. An
    /// upvalue is captured into this list when a closure closes over a local and
    /// removed when its slot is closed over on return or scope exit.
    open_upvalues: Vec<GcRef>,
    /// GC is exercised during execution unless disabled for a test.
    auto_gc: bool,
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

impl Vm {
    pub fn new() -> Self {
        Vm {
            stack: Vec::new(),
            frames: Vec::new(),
            globals: std::collections::HashMap::new(),
            heap: Heap::new(),
            output: String::new(),
            open_upvalues: Vec::new(),
            auto_gc: true,
        }
    }

    pub fn set_auto_gc(&mut self, on: bool) {
        self.auto_gc = on;
    }

    /// Force a collection after almost every allocation. Used by tests to prove
    /// the VM never frees a live object mid-execution.
    pub fn set_gc_stress(&mut self, on: bool) {
        self.heap.stress = on;
        self.heap.next_gc = if on { 1 } else { 128 };
    }

    pub fn take_output(&mut self) -> String {
        std::mem::take(&mut self.output)
    }

    fn push(&mut self, v: Value) {
        self.stack.push(v);
    }

    fn pop(&mut self) -> Value {
        self.stack.pop().expect("value stack underflow")
    }

    fn peek(&self, distance: usize) -> Value {
        self.stack[self.stack.len() - 1 - distance]
    }

    fn read_byte(&mut self, program: &Program, frame: usize) -> u8 {
        let f = &mut self.frames[frame];
        let b = program.funcs[f.func].code[f.ip];
        f.ip += 1;
        b
    }

    fn read_short(&mut self, program: &Program, frame: usize) -> u16 {
        let hi = u16::from(self.read_byte(program, frame));
        let lo = u16::from(self.read_byte(program, frame));
        (hi << 8) | lo
    }

    fn constant(&self, program: &Program, frame: usize, idx: usize) -> Constant {
        let func = self.frames[frame].func;
        program.funcs[func].constants[idx].clone()
    }

    fn maybe_gc(&mut self) {
        if !self.auto_gc || !self.heap.should_collect() {
            return;
        }
        let mut roots: Vec<Value> = self.stack.clone();
        for v in self.globals.values() {
            roots.push(*v);
        }
        for f in &self.frames {
            roots.push(Value::Obj(f.closure));
        }
        for r in &self.open_upvalues {
            roots.push(Value::Obj(*r));
        }
        self.heap.collect(&roots);
    }

    /// Interpret a whole program, returning the value it produces.
    pub fn interpret(&mut self, program: &Program) -> Result<Value, String> {
        for native in Native::ALL {
            let r = self.heap.alloc(Obj::Native(native));
            self.globals.insert(native.name().to_string(), Value::Obj(r));
        }
        let main_closure = self.heap.alloc(Obj::Closure(Closure {
            func: program.main,
            upvalues: Vec::new(),
        }));
        self.push(Value::Obj(main_closure));
        self.frames.push(Frame {
            closure: main_closure,
            func: program.main,
            ip: 0,
            slot_base: 0,
        });
        self.run(program)
    }

    fn run(&mut self, program: &Program) -> Result<Value, String> {
        loop {
            self.maybe_gc();
            let frame = self.frames.len() - 1;
            let op = self.read_byte(program, frame);
            match op {
                OP_CONST => {
                    let idx = self.read_short(program, frame) as usize;
                    let c = self.constant(program, frame, idx);
                    let v = self.materialize(c, program);
                    self.push(v);
                }
                OP_NIL => self.push(Value::Nil),
                OP_TRUE => self.push(Value::Bool(true)),
                OP_FALSE => self.push(Value::Bool(false)),
                OP_POP => {
                    self.pop();
                }
                OP_NEG => {
                    let v = self.pop();
                    let r = match v {
                        Value::Int(n) => Value::Int(n.wrapping_neg()),
                        Value::Float(x) => Value::Float(-x),
                        _ => return Err("operand of '-' must be a number".into()),
                    };
                    self.push(r);
                }
                OP_NOT => {
                    let v = self.pop();
                    self.push(Value::Bool(v.is_falsey()));
                }
                OP_ADD => self.binary_add()?,
                OP_SUB | OP_MUL | OP_DIV | OP_MOD => self.binary_num(op)?,
                OP_EQ => {
                    let b = self.pop();
                    let a = self.pop();
                    self.push(Value::Bool(self.values_equal(a, b)));
                }
                OP_NEQ => {
                    let b = self.pop();
                    let a = self.pop();
                    self.push(Value::Bool(!self.values_equal(a, b)));
                }
                OP_LT | OP_LE | OP_GT | OP_GE => self.binary_cmp(op)?,
                OP_DEF_GLOBAL => {
                    let idx = self.read_short(program, frame) as usize;
                    let name = self.const_str(program, frame, idx)?;
                    let v = self.pop();
                    self.globals.insert(name, v);
                }
                OP_GET_GLOBAL => {
                    let idx = self.read_short(program, frame) as usize;
                    let name = self.const_str(program, frame, idx)?;
                    match self.globals.get(&name) {
                        Some(v) => {
                            let v = *v;
                            self.push(v);
                        }
                        None => return Err(format!("undefined variable '{name}'")),
                    }
                }
                OP_SET_GLOBAL => {
                    let idx = self.read_short(program, frame) as usize;
                    let name = self.const_str(program, frame, idx)?;
                    if !self.globals.contains_key(&name) {
                        return Err(format!("undefined variable '{name}'"));
                    }
                    let v = self.peek(0);
                    self.globals.insert(name, v);
                }
                OP_GET_LOCAL => {
                    let slot = self.read_byte(program, frame) as usize;
                    let base = self.frames[frame].slot_base;
                    let v = self.stack[base + slot];
                    self.push(v);
                }
                OP_SET_LOCAL => {
                    let slot = self.read_byte(program, frame) as usize;
                    let base = self.frames[frame].slot_base;
                    self.stack[base + slot] = self.peek(0);
                }
                OP_GET_UPVALUE => {
                    let i = self.read_byte(program, frame) as usize;
                    let uv = self.frame_upvalue(frame, i)?;
                    let v = match self.heap.get(uv) {
                        Obj::Upvalue(Upvalue::Open(loc)) => self.stack[*loc],
                        Obj::Upvalue(Upvalue::Closed(v)) => *v,
                        _ => return Err("upvalue expected".into()),
                    };
                    self.push(v);
                }
                OP_SET_UPVALUE => {
                    let i = self.read_byte(program, frame) as usize;
                    let uv = self.frame_upvalue(frame, i)?;
                    let v = self.peek(0);
                    match self.heap.get(uv) {
                        Obj::Upvalue(Upvalue::Open(loc)) => {
                            let loc = *loc;
                            self.stack[loc] = v;
                        }
                        Obj::Upvalue(Upvalue::Closed(_)) => {
                            *self.heap.get_mut(uv) = Obj::Upvalue(Upvalue::Closed(v));
                        }
                        _ => return Err("upvalue expected".into()),
                    }
                }
                OP_JUMP => {
                    let off = self.read_short(program, frame) as usize;
                    self.frames[frame].ip += off;
                }
                OP_JUMP_IF_FALSE => {
                    let off = self.read_short(program, frame) as usize;
                    if self.peek(0).is_falsey() {
                        self.frames[frame].ip += off;
                    }
                }
                OP_LOOP => {
                    let off = self.read_short(program, frame) as usize;
                    self.frames[frame].ip -= off;
                }
                OP_CALL => {
                    let argc = self.read_byte(program, frame) as usize;
                    self.call_value(program, argc)?;
                }
                OP_CLOSURE => {
                    let idx = self.read_short(program, frame) as usize;
                    let c = self.constant(program, frame, idx);
                    let Constant::Func(fi) = c else {
                        return Err("CLOSURE operand is not a function".into());
                    };
                    let upvalue_count = program.funcs[fi].upvalue_count;
                    let mut upvalues = Vec::with_capacity(upvalue_count);
                    for _ in 0..upvalue_count {
                        let is_local = self.read_byte(program, frame);
                        let index = self.read_byte(program, frame) as usize;
                        let uv = if is_local != 0 {
                            let base = self.frames[frame].slot_base;
                            self.capture_upvalue(base + index)
                        } else {
                            self.frame_upvalue(frame, index)?
                        };
                        upvalues.push(uv);
                    }
                    let r = self.heap.alloc(Obj::Closure(Closure { func: fi, upvalues }));
                    self.push(Value::Obj(r));
                }
                OP_RETURN => {
                    let result = self.pop();
                    let base = self.frames[frame].slot_base;
                    // Every variable in this frame is about to vanish from the
                    // stack. Close any upvalue still pointing into it first.
                    self.close_upvalues(base);
                    self.frames.pop();
                    if self.frames.is_empty() {
                        self.stack.truncate(base);
                        return Ok(result);
                    }
                    self.stack.truncate(base);
                    self.push(result);
                }
                OP_CLOSE_UPVALUE => {
                    let top = self.stack.len() - 1;
                    self.close_upvalues(top);
                    self.pop();
                }
                OP_PRINT => {
                    let v = self.pop();
                    let s = self.display(v);
                    self.output.push_str(&s);
                    self.output.push('\n');
                }
                other => return Err(format!("unknown opcode {other}")),
            }
        }
    }

    fn materialize(&mut self, c: Constant, program: &Program) -> Value {
        match c {
            Constant::Nil => Value::Nil,
            Constant::Bool(b) => Value::Bool(b),
            Constant::Int(n) => Value::Int(n),
            Constant::Float(x) => Value::Float(x),
            Constant::Str(s) => Value::Obj(self.heap.alloc_str(s)),
            Constant::Func(fi) => {
                // A bare function constant carries no captured environment, so
                // its upvalue cells start closed over nil. The compiler always
                // reaches functions through OP_CLOSURE instead, which is where
                // real capture happens.
                let upvalue_count = program.funcs[fi].upvalue_count;
                let upvalues = (0..upvalue_count)
                    .map(|_| self.heap.alloc(Obj::Upvalue(Upvalue::Closed(Value::Nil))))
                    .collect();
                Value::Obj(self.heap.alloc(Obj::Closure(Closure { func: fi, upvalues })))
            }
        }
    }

    fn const_str(&self, program: &Program, frame: usize, idx: usize) -> Result<String, String> {
        match self.constant(program, frame, idx) {
            Constant::Str(s) => Ok(s),
            _ => Err("expected string constant".into()),
        }
    }

    fn call_value(&mut self, program: &Program, argc: usize) -> Result<(), String> {
        let callee = self.peek(argc);
        let Value::Obj(r) = callee else {
            return Err("can only call functions".into());
        };
        let func = match self.heap.get(r) {
            Obj::Closure(c) => c.func,
            Obj::Native(n) => return self.call_native(*n, argc),
            _ => return Err("can only call functions".into()),
        };
        let proto = &program.funcs[func];
        if argc != proto.arity {
            return Err(format!(
                "function '{}' expects {} arguments, got {}",
                proto.name, proto.arity, argc
            ));
        }
        if self.frames.len() >= MAX_CALL_DEPTH {
            return Err(CALL_DEPTH_ERROR.into());
        }
        let slot_base = self.stack.len() - argc - 1;
        self.frames.push(Frame {
            closure: r,
            func,
            ip: 0,
            slot_base,
        });
        Ok(())
    }

    /// Run a builtin: consume its arguments and the callee from the stack and
    /// leave its result in their place, exactly as returning from a normal call
    /// would. Semantics mirror the reference interpreter's `apply_native`.
    fn call_native(&mut self, native: Native, argc: usize) -> Result<(), String> {
        if argc != native.arity() {
            return Err(format!(
                "{} expects {} arguments, got {}",
                native.name(),
                native.arity(),
                argc
            ));
        }
        let args: Vec<Value> = (0..argc).map(|i| self.peek(argc - 1 - i)).collect();
        let result = match native {
            Native::Abs => match args[0] {
                Value::Int(n) => Value::Int(n.wrapping_abs()),
                Value::Float(x) => Value::Float(x.abs()),
                _ => return Err("abs expects a number".into()),
            },
            Native::Min | Native::Max => {
                let want_max = native == Native::Max;
                match (args[0], args[1]) {
                    (Value::Int(a), Value::Int(b)) => {
                        Value::Int(if (a >= b) == want_max { a } else { b })
                    }
                    _ => match (num(args[0]), num(args[1])) {
                        (Some(a), Some(b)) => {
                            Value::Float(if want_max { a.max(b) } else { a.min(b) })
                        }
                        _ => return Err("min and max expect numbers".into()),
                    },
                }
            }
            Native::Len => match args[0] {
                Value::Obj(r) => match self.heap.get(r) {
                    Obj::Str(s) => Value::Int(s.chars().count() as i64),
                    _ => return Err("len expects a string".into()),
                },
                _ => return Err("len expects a string".into()),
            },
        };
        for _ in 0..=argc {
            self.pop();
        }
        self.push(result);
        Ok(())
    }

    /// The upvalue cell at index `i` of the closure running in `frame`.
    fn frame_upvalue(&self, frame: usize, i: usize) -> Result<GcRef, String> {
        let cl = self.frames[frame].closure;
        match self.heap.get(cl) {
            Obj::Closure(c) => Ok(c.upvalues[i]),
            _ => Err("closure expected".into()),
        }
    }

    /// Capture the value-stack slot `location` as an upvalue, reusing an existing
    /// open upvalue for that slot so every closure over one variable shares a
    /// single cell (matching the reference interpreter's shared environment).
    fn capture_upvalue(&mut self, location: usize) -> GcRef {
        for &r in &self.open_upvalues {
            if let Obj::Upvalue(Upvalue::Open(loc)) = self.heap.get(r) {
                if *loc == location {
                    return r;
                }
            }
        }
        let r = self.heap.alloc(Obj::Upvalue(Upvalue::Open(location)));
        self.open_upvalues.push(r);
        r
    }

    /// Close every open upvalue pointing at or above `from`, lifting each one's
    /// current stack value into the cell so it survives the slot going away.
    fn close_upvalues(&mut self, from: usize) {
        let mut i = 0;
        while i < self.open_upvalues.len() {
            let r = self.open_upvalues[i];
            let Obj::Upvalue(Upvalue::Open(loc)) = self.heap.get(r) else {
                self.open_upvalues.swap_remove(i);
                continue;
            };
            let loc = *loc;
            if loc >= from {
                let v = self.stack[loc];
                *self.heap.get_mut(r) = Obj::Upvalue(Upvalue::Closed(v));
                self.open_upvalues.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    fn binary_add(&mut self) -> Result<(), String> {
        let b = self.pop();
        let a = self.pop();
        let r = match (a, b) {
            (Value::Int(x), Value::Int(y)) => Value::Int(x.wrapping_add(y)),
            (Value::Obj(x), Value::Obj(y)) => {
                if let (Obj::Str(sx), Obj::Str(sy)) = (self.heap.get(x), self.heap.get(y)) {
                    let joined = format!("{sx}{sy}");
                    Value::Obj(self.heap.alloc_str(joined))
                } else {
                    return Err("operands of '+' must be numbers or strings".into());
                }
            }
            _ => match (num(a), num(b)) {
                (Some(x), Some(y)) => Value::Float(x + y),
                _ => return Err("operands of '+' must be numbers or strings".into()),
            },
        };
        self.push(r);
        Ok(())
    }

    fn binary_num(&mut self, op: u8) -> Result<(), String> {
        let b = self.pop();
        let a = self.pop();
        let r = match (a, b) {
            (Value::Int(x), Value::Int(y)) => match op {
                OP_SUB => Value::Int(x.wrapping_sub(y)),
                OP_MUL => Value::Int(x.wrapping_mul(y)),
                OP_DIV => {
                    if y == 0 {
                        return Err("division by zero".into());
                    }
                    Value::Int(x.wrapping_div(y))
                }
                OP_MOD => {
                    if y == 0 {
                        return Err("modulo by zero".into());
                    }
                    Value::Int(x.wrapping_rem(y))
                }
                _ => unreachable!(),
            },
            _ => match (num(a), num(b)) {
                (Some(x), Some(y)) => match op {
                    OP_SUB => Value::Float(x - y),
                    OP_MUL => Value::Float(x * y),
                    OP_DIV => {
                        if y == 0.0 {
                            return Err("division by zero".into());
                        }
                        Value::Float(x / y)
                    }
                    OP_MOD => {
                        if y == 0.0 {
                            return Err("modulo by zero".into());
                        }
                        Value::Float(x % y)
                    }
                    _ => unreachable!(),
                },
                _ => return Err("operands must be numbers".into()),
            },
        };
        self.push(r);
        Ok(())
    }

    fn binary_cmp(&mut self, op: u8) -> Result<(), String> {
        use std::cmp::Ordering::{Greater, Less};
        let b = self.pop();
        let a = self.pop();
        let ord = match (a, b) {
            (Value::Int(x), Value::Int(y)) => x.cmp(&y),
            _ => match (num(a), num(b)) {
                (Some(x), Some(y)) => {
                    x.partial_cmp(&y).ok_or("cannot compare NaN".to_string())?
                }
                _ => return Err("operands of comparison must be numbers".into()),
            },
        };
        let result = match op {
            OP_LT => ord == Less,
            OP_LE => ord != Greater,
            OP_GT => ord == Greater,
            OP_GE => ord != Less,
            _ => unreachable!(),
        };
        self.push(Value::Bool(result));
        Ok(())
    }

    fn values_equal(&self, a: Value, b: Value) -> bool {
        match (a, b) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(x), Value::Bool(y)) => x == y,
            (Value::Int(x), Value::Int(y)) => x == y,
            (Value::Float(x), Value::Float(y)) => x == y,
            (Value::Int(x), Value::Float(y)) | (Value::Float(y), Value::Int(x)) => x as f64 == y,
            (Value::Obj(x), Value::Obj(y)) => {
                if x == y {
                    return true;
                }
                match (self.heap.get(x), self.heap.get(y)) {
                    (Obj::Str(sx), Obj::Str(sy)) => sx == sy,
                    _ => false,
                }
            }
            _ => false,
        }
    }

    fn display(&self, v: Value) -> String {
        match v {
            Value::Nil => "nil".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Int(n) => n.to_string(),
            Value::Float(x) => format_float(x),
            Value::Obj(r) => match self.heap.get(r) {
                Obj::Str(s) => s.clone(),
                Obj::Closure(c) => format!("<fn {}>", c.func),
                Obj::Native(n) => format!("<native {}>", n.name()),
                // Upvalue cells never surface as user values.
                Obj::Upvalue(_) => "<upvalue>".to_string(),
            },
        }
    }

    /// Reduce a value to a backend-independent `Outcome` for comparison.
    pub fn to_outcome(&self, v: Value) -> Outcome {
        match v {
            Value::Nil => Outcome::Nil,
            Value::Bool(b) => Outcome::Bool(b),
            Value::Int(n) => Outcome::Int(n),
            Value::Float(x) => Outcome::Float(x),
            Value::Obj(r) => match self.heap.get(r) {
                Obj::Str(s) => Outcome::Str(s.clone()),
                Obj::Closure(_) | Obj::Native(_) => Outcome::Func,
                Obj::Upvalue(_) => Outcome::Nil,
            },
        }
    }

    pub fn heap(&self) -> &Heap {
        &self.heap
    }
}

fn num(v: Value) -> Option<f64> {
    match v {
        Value::Int(n) => Some(n as f64),
        Value::Float(x) => Some(x),
        _ => None,
    }
}

/// Render a float the way both evaluators and the CLI agree on.
pub fn format_float(x: f64) -> String {
    if x == x.trunc() && x.is_finite() {
        format!("{x:.1}")
    } else {
        format!("{x}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::compile;
    use crate::lexer::tokenize;
    use crate::parser::parse;

    fn run(src: &str) -> Outcome {
        let program = compile(&parse(tokenize(src).unwrap()).unwrap()).unwrap();
        let mut vm = Vm::new();
        let v = vm.interpret(&program).unwrap();
        vm.to_outcome(v)
    }

    #[test]
    fn arithmetic() {
        assert_eq!(run("return 1 + 2 * 3 - 4;"), Outcome::Int(3));
        assert_eq!(run("return 7 / 2;"), Outcome::Int(3));
        assert_eq!(run("return 7 % 3;"), Outcome::Int(1));
        assert_eq!(run("return -5;"), Outcome::Int(-5));
    }

    #[test]
    fn comparisons_and_bools() {
        assert_eq!(run("return 1 < 2;"), Outcome::Bool(true));
        assert_eq!(run("return 2 <= 2;"), Outcome::Bool(true));
        assert_eq!(run("return 3 == 3;"), Outcome::Bool(true));
        assert_eq!(run("return !false;"), Outcome::Bool(true));
    }

    #[test]
    fn variables_and_scope() {
        assert_eq!(run("let a = 10; { let a = 1; } return a;"), Outcome::Int(10));
        assert_eq!(run("let a = 1; a = a + 41; return a;"), Outcome::Int(42));
    }

    #[test]
    fn control_flow() {
        assert_eq!(
            run("let x = 0; if (1 < 2) { x = 5; } else { x = 6; } return x;"),
            Outcome::Int(5)
        );
        assert_eq!(
            run("let i = 0; let s = 0; while (i < 5) { s = s + i; i = i + 1; } return s;"),
            Outcome::Int(10)
        );
    }

    #[test]
    fn functions_and_recursion() {
        let src = "fn fib(n) { if (n < 2) { return n; } return fib(n - 1) + fib(n - 2); } return fib(10);";
        assert_eq!(run(src), Outcome::Int(55));
    }

    #[test]
    fn closures_capture_by_value() {
        let src = "fn make(x) { fn add(n) { return n + x; } return add; } let a = make(5); return a(3);";
        assert_eq!(run(src), Outcome::Int(8));
    }

    #[test]
    fn string_concat() {
        assert_eq!(run("return \"ab\" + \"cd\";"), Outcome::Str("abcd".into()));
    }
}
