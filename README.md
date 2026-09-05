# Kindling

Kindling is a small, from-scratch, dynamically typed programming language with a real bytecode compiler, a stack based virtual machine, and a mark and sweep garbage collector. It is written in pure Rust with zero external dependencies on the 2021 edition.

Try it live in your browser at https://pavanchow.github.io/kindling/. The playground shows the whole pipeline, source to tokens to AST to bytecode to a running VM, and lets you step one instruction at a time while watching the value stack, the call frames, and the garbage collector.

## What it is

Kindling is a complete language runtime that fits in your head. It has a hand written lexer, a recursive descent parser, a single pass compiler that emits its own opcode set, and a clox style virtual machine that executes that bytecode with call frames, globals, locals, closures, and a precise garbage collector. It ships with a CLI that can run a program, disassemble it, or start a REPL.

## The gap it fills

Most teaching interpreters stop at a tree walker, and most production runtimes are far too large to read in an afternoon. Kindling sits in the middle. It is a genuine bytecode runtime with the parts that matter (a compiler, a serializable bytecode format, a stack VM, and a collector) kept small enough to read end to end.

A person reaches for Kindling to learn exactly how a language becomes bytecode and how that bytecode runs, or to embed a tiny scripting layer into a Rust program without pulling in a dependency tree. An AI agent reaches for it for the same embedding reason plus one more: the runtime is small, deterministic, sandboxed (no file, network, or system access from inside a Kindling program), and its bytecode is a stable serializable format. An agent can compile a plan to bytes, ship the bytes, and run them anywhere with identical results, and it can trust those results because two independent evaluators are checked against each other on every build.

## Quickstart

```
cargo build --release
./target/release/kindling run examples/factorial.kdl
./target/release/kindling disasm examples/factorial.kdl
./target/release/kindling repl
```

Run the test suite, including the correctness gate:

```
cargo test
KINDLING_FUZZ_OPS=4000 cargo test --release
```

## The language

Kindling is dynamically typed with integers, floats, booleans, strings, nil, and functions. Source files use the `.kdl` extension.

```
// variables and arithmetic
let x = 10;
let y = x * 2 + 5;

// conditionals
if (y > 20) {
  print "big";
} else {
  print "small";
}

// loops
let i = 0;
let sum = 0;
while (i < 10) {
  sum = sum + i;
  i = i + 1;
}

// functions and recursion
fn fib(n) {
  if (n < 2) { return n; }
  return fib(n - 1) + fib(n - 2);
}

// closures capture surrounding values
fn make_adder(k) {
  fn adder(n) { return n + k; }
  return adder;
}
let add10 = make_adder(10);
print add10(32);

// the program produces the value of its last statement, or an explicit return
return fib(20);
```

Operators are the four usual arithmetic operators plus modulo, the six comparisons, logical not, and numeric negation. The plus operator also concatenates two strings. Integer division and modulo by zero are runtime errors. Only nil and false are falsey.

## The API

The crate exposes the whole pipeline. The common entry points are:

```rust
use kindling::{run_source, compile_source, run_program, eval_reference};
use kindling::{disassemble, assemble, serialize, deserialize};

// compile and run in one step, returns value plus printed output
let result = run_source("return 6 * 7;")?;
assert_eq!(result.value, kindling::Outcome::Int(42));

// compile once, then run, disassemble, or serialize
let program = compile_source("let a = 1; return a + 2;")?;
let listing = disassemble(&program);        // readable, reassemblable text
let same = assemble(&listing)?;             // text back to a Program
let bytes = serialize(&program);            // compact binary blob
let restored = deserialize(&bytes)?;        // binary back to a Program
let run = run_program(&restored)?;

// evaluate the same source with the independent reference interpreter
let reference = eval_reference("return 6 * 7;")?;
```

Individual stages (`lexer`, `parser`, `compiler`, `vm`, `interp`, `gc`, `chunk`, `opcode`, `disasm`, `serialize`) are public modules if you want to drive them directly.

## The correctness gate

Correctness is enforced by three machine checkable gates that run as tests. See DESIGN.md for why each gate proves what it claims.

1. Differential testing. For hundreds of randomly generated programs, the bytecode VM result must equal the result of a completely independent tree walking reference interpreter over the same AST. Two independent evaluators agreeing is the oracle. Program count is set with the `KINDLING_FUZZ_OPS` environment variable.

2. Round trip integrity. Disassembling and reassembling a program reproduces it exactly, serializing and deserializing a program reproduces it exactly, and running a program through the binary round trip gives the same answer as running it directly.

3. Garbage collector correctness. With a known reachability graph, a collection frees exactly the unreachable objects and keeps every reachable one, a churn test proves a known root is never freed and known dead objects never leak, and a VM stress test forces a collection after almost every allocation to prove the collector never frees a live object during real execution.

Every module also carries unit tests for its own behavior.

## Layout

```
src/lexer.rs      source text to tokens
src/parser.rs     tokens to an AST
src/ast.rs        AST node types
src/compiler.rs   AST to bytecode
src/opcode.rs     the opcode set and operand shapes
src/chunk.rs      bytecode containers (Program, FuncProto, Constant)
src/vm.rs         the stack based virtual machine
src/interp.rs     the independent tree walking reference interpreter
src/gc.rs         the mark and sweep garbage collector
src/disasm.rs     text disassembler and assembler
src/serialize.rs  binary serializer and deserializer
src/gen.rs        random program generator for the differential gate
src/bin/kindling.rs  the command line tool
docs/index.html   the browser playground
```

## License

MIT.
