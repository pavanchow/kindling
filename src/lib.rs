//! Kindling: a small, from-scratch, dynamically-typed language with a bytecode
//! virtual machine and a mark-and-sweep garbage collector. Pure std, zero
//! dependencies.
//!
//! The crate exposes the full pipeline as composable pieces (lexer, parser,
//! compiler, VM, tree-walking reference interpreter, disassembler/assembler,
//! and binary serializer) plus a few convenience entry points.

#![warn(clippy::pedantic)]
// These pedantic lints are intentionally allowed for this crate:
// - the casts are deliberate and bounded (bytecode operands are range-checked
//   before narrowing, u32 length prefixes bound serialized sizes, and i64 to
//   f64 promotion is the language's defined numeric semantics);
// - almost every fallible function returns `Result<_, String>`, so per-function
//   `# Errors`/`# Panics` prose would be pure noise;
// - `must_use` and `similar_names` add churn without value at this size;
// - the opcode module is a flat table of constants that reads best as a glob;
// - the VM instruction loop is one long, deliberate dispatch match.
// - exact float comparison is the language's defined equality and its float
//   round-trip identity, so it must compare bit-for-bit.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::similar_names,
    clippy::wildcard_imports,
    clippy::too_many_lines
)]

pub mod ast;
pub mod chunk;
pub mod compiler;
pub mod disasm;
pub mod gc;
pub mod gen;
pub mod interp;
pub mod lexer;
pub mod opcode;
pub mod parser;
pub mod serialize;
pub mod value;
pub mod vm;

pub use chunk::Program;
pub use value::Outcome;

/// The result of running a program: its produced value plus anything it printed.
#[derive(Clone, Debug, PartialEq)]
pub struct RunResult {
    pub value: Outcome,
    pub output: String,
}

/// Parse and compile source into a `Program`.
pub fn compile_source(src: &str) -> Result<Program, String> {
    let tokens = lexer::tokenize(src)?;
    let ast = parser::parse(tokens)?;
    compiler::compile(&ast)
}

/// Compile and run source on the bytecode VM.
pub fn run_source(src: &str) -> Result<RunResult, String> {
    let program = compile_source(src)?;
    run_program(&program)
}

/// Run an already-compiled program on the bytecode VM.
pub fn run_program(program: &Program) -> Result<RunResult, String> {
    let mut machine = vm::Vm::new();
    let value = machine.interpret(program)?;
    Ok(RunResult {
        value: machine.to_outcome(value),
        output: machine.take_output(),
    })
}

/// Evaluate source with the independent tree-walking reference interpreter.
pub fn eval_reference(src: &str) -> Result<RunResult, String> {
    let tokens = lexer::tokenize(src)?;
    let ast = parser::parse(tokens)?;
    let mut interp = interp::Interp::new();
    let value = interp.run(&ast)?;
    Ok(RunResult {
        value: interp::to_outcome(&value),
        output: interp.take_output(),
    })
}

/// Disassemble a program into a readable, reassemblable listing.
pub fn disassemble(program: &Program) -> String {
    disasm::disassemble(program)
}

/// Parse a disassembly listing back into a program.
pub fn assemble(text: &str) -> Result<Program, String> {
    disasm::assemble(text)
}

/// Serialize a program to a compact binary blob.
pub fn serialize(program: &Program) -> Vec<u8> {
    serialize::serialize(program)
}

/// Deserialize a binary blob back into a program.
pub fn deserialize(bytes: &[u8]) -> Result<Program, String> {
    serialize::deserialize(bytes)
}
