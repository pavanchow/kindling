//! Text disassembler and matching assembler.
//!
//! `disassemble` turns a `Program` into a readable, line-oriented listing that
//! the CLI prints and that `assemble` parses back into an identical `Program`.
//! The text form is lossless: `assemble(disassemble(p)) == p`.

use crate::chunk::{Constant, FuncProto, Program};
use crate::opcode::*;
use std::fmt::Write as _;

pub fn disassemble(program: &Program) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "program main={} funcs={}", program.main, program.funcs.len());
    for (i, f) in program.funcs.iter().enumerate() {
        out.push('\n');
        let _ = writeln!(out, "func {i}");
        let _ = writeln!(out, "  name {}", quote(&f.name));
        let _ = writeln!(out, "  arity {}", f.arity);
        let _ = writeln!(out, "  upvals {}", f.upvalue_count);
        let _ = writeln!(out, "  constants {}", f.constants.len());
        for (ci, c) in f.constants.iter().enumerate() {
            let _ = writeln!(out, "    {ci} {}", constant_text(c));
        }
        out.push_str("  code\n");
        disassemble_code(program, f, &mut out);
    }
    out
}

/// Disassemble one function's bytecode into a readable listing. Used both by
/// the text round trip and by the CLI `disasm` mode.
pub fn disassemble_function(program: &Program, f: &FuncProto) -> String {
    let mut out = String::new();
    disassemble_code(program, f, &mut out);
    out
}

fn disassemble_code(program: &Program, f: &FuncProto, out: &mut String) {
    let mut ip = 0;
    while ip < f.code.len() {
        let op = f.code[ip];
        let mnemonic = name(op).unwrap_or("BAD");
        let start = ip;
        ip += 1;
        match operand(op) {
            Operand::None => {
                let _ = writeln!(out, "    {start:04} {mnemonic}");
            }
            Operand::Byte => {
                let b = f.code[ip];
                ip += 1;
                let _ = writeln!(out, "    {start:04} {mnemonic} {b}");
            }
            Operand::Short => {
                let v = f.read_short(ip);
                ip += 2;
                let target = match op {
                    OP_JUMP | OP_JUMP_IF_FALSE => format!("  // -> {}", ip + v as usize),
                    OP_LOOP => format!("  // -> {}", ip - v as usize),
                    _ => String::new(),
                };
                let _ = writeln!(out, "    {start:04} {mnemonic} {v}{target}");
            }
            Operand::Closure => {
                let cidx = f.read_short(ip);
                ip += 2;
                // The number of upvalue descriptor pairs is the referenced
                // function's upvalue count. We emit it explicitly (`pairs N`)
                // so the assembler never resolves forward references.
                let count = match f.constants.get(cidx as usize) {
                    Some(Constant::Func(fi)) => program.funcs[*fi].upvalue_count,
                    _ => 0,
                };
                let mut line = format!("    {start:04} {mnemonic} {cidx} pairs {count}");
                for _ in 0..count {
                    let is_local = f.code[ip];
                    let index = f.code[ip + 1];
                    ip += 2;
                    let _ = write!(line, " {is_local} {index}");
                }
                line.push('\n');
                out.push_str(&line);
            }
        }
    }
}

fn constant_text(c: &Constant) -> String {
    match c {
        Constant::Nil => "nil".to_string(),
        Constant::Bool(b) => format!("bool {b}"),
        Constant::Int(n) => format!("int {n}"),
        Constant::Float(x) => format!("float {x:?}"),
        Constant::Str(s) => format!("str {}", quote(s)),
        Constant::Func(i) => format!("func {i}"),
    }
}

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn unquote(s: &str) -> Result<String, String> {
    let bytes: Vec<char> = s.chars().collect();
    if bytes.first() != Some(&'"') || bytes.last() != Some(&'"') || bytes.len() < 2 {
        return Err(format!("bad quoted string: {s}"));
    }
    let mut out = String::new();
    let mut i = 1;
    while i < bytes.len() - 1 {
        let c = bytes[i];
        if c == '\\' {
            i += 1;
            match bytes.get(i) {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                other => return Err(format!("bad escape: {other:?}")),
            }
        } else {
            out.push(c);
        }
        i += 1;
    }
    Ok(out)
}

/// Parse a disassembly listing back into a `Program`.
pub fn assemble(text: &str) -> Result<Program, String> {
    let mut lines = text.lines().peekable();
    let mut program = Program::default();

    // header
    let header = next_meaningful(&mut lines).ok_or("empty assembly")?;
    let main = parse_kv(&header, "main")?;
    program.main = main;

    let mut funcs: Vec<FuncProto> = Vec::new();
    while let Some(line) = next_meaningful(&mut lines) {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("func ") {
            let _index: usize = rest.trim().parse().map_err(|_| "bad func index")?;
            let proto = assemble_func(&mut lines)?;
            funcs.push(proto);
        } else {
            return Err(format!("unexpected line: {trimmed}"));
        }
    }
    program.funcs = funcs;
    Ok(program)
}

type Lines<'a> = std::iter::Peekable<std::str::Lines<'a>>;

fn strip_comment(line: &str) -> &str {
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

fn next_meaningful(lines: &mut Lines) -> Option<String> {
    for line in lines.by_ref() {
        let clean = strip_comment(line);
        if !clean.trim().is_empty() {
            return Some(clean.to_string());
        }
    }
    None
}

fn peek_meaningful_starts_with(lines: &mut Lines, prefix: &str) -> bool {
    while let Some(line) = lines.peek() {
        let clean = strip_comment(line);
        if clean.trim().is_empty() {
            lines.next();
            continue;
        }
        return clean.trim().starts_with(prefix);
    }
    false
}

fn parse_kv(line: &str, key: &str) -> Result<usize, String> {
    for tok in line.split_whitespace() {
        if let Some(v) = tok.strip_prefix(&format!("{key}=")) {
            return v.parse().map_err(|_| format!("bad value for {key}"));
        }
    }
    Err(format!("missing key {key} in: {line}"))
}

fn assemble_func(lines: &mut Lines) -> Result<FuncProto, String> {
    let mut proto = FuncProto::default();

    let name_line = next_meaningful(lines).ok_or("expected name")?;
    proto.name = unquote(name_line.trim().strip_prefix("name ").ok_or("expected name")?.trim())?;

    let arity_line = next_meaningful(lines).ok_or("expected arity")?;
    proto.arity = arity_line
        .trim()
        .strip_prefix("arity ")
        .ok_or("expected arity")?
        .trim()
        .parse()
        .map_err(|_| "bad arity")?;

    let upvals_line = next_meaningful(lines).ok_or("expected upvals")?;
    proto.upvalue_count = upvals_line
        .trim()
        .strip_prefix("upvals ")
        .ok_or("expected upvals")?
        .trim()
        .parse()
        .map_err(|_| "bad upvals")?;

    let consts_line = next_meaningful(lines).ok_or("expected constants")?;
    let const_count: usize = consts_line
        .trim()
        .strip_prefix("constants ")
        .ok_or("expected constants")?
        .trim()
        .parse()
        .map_err(|_| "bad constants count")?;

    for _ in 0..const_count {
        let cl = next_meaningful(lines).ok_or("expected constant")?;
        proto.constants.push(parse_constant(cl.trim())?);
    }

    let code_line = next_meaningful(lines).ok_or("expected code")?;
    if code_line.trim() != "code" {
        return Err(format!("expected 'code', found {}", code_line.trim()));
    }

    // Code instructions run until the next `func` or end of input.
    while !peek_meaningful_starts_with(lines, "func ") {
        let Some(inst) = next_meaningful(lines) else {
            break;
        };
        assemble_instruction(inst.trim(), &mut proto)?;
    }

    Ok(proto)
}

fn parse_constant(line: &str) -> Result<Constant, String> {
    // format: <index> <type> <value...>
    let mut parts = line.splitn(3, char::is_whitespace);
    let _index = parts.next().ok_or("bad constant line")?;
    let ty = parts.next().ok_or("bad constant type")?;
    let rest = parts.next().unwrap_or("").trim();
    match ty {
        "nil" => Ok(Constant::Nil),
        "bool" => Ok(Constant::Bool(rest == "true")),
        "int" => rest.parse().map(Constant::Int).map_err(|_| "bad int".into()),
        "float" => rest
            .parse()
            .map(Constant::Float)
            .map_err(|_| "bad float".into()),
        "str" => Ok(Constant::Str(unquote(rest)?)),
        "func" => rest
            .parse()
            .map(Constant::Func)
            .map_err(|_| "bad func index".into()),
        other => Err(format!("unknown constant type {other}")),
    }
}

fn assemble_instruction(line: &str, proto: &mut FuncProto) -> Result<(), String> {
    let mut toks = line.split_whitespace().peekable();
    // Drop a leading numeric offset if present.
    if let Some(first) = toks.peek() {
        if first.parse::<usize>().is_ok() {
            toks.next();
        }
    }
    let mnemonic = toks.next().ok_or("empty instruction")?;
    let op = from_name(mnemonic).ok_or_else(|| format!("unknown opcode {mnemonic}"))?;
    proto.emit(op);
    match operand(op) {
        Operand::None => {}
        Operand::Byte => {
            let b: u8 = toks.next().ok_or("missing byte operand")?.parse().map_err(|_| "bad byte")?;
            proto.emit(b);
        }
        Operand::Short => {
            let v: u16 = toks.next().ok_or("missing short operand")?.parse().map_err(|_| "bad short")?;
            proto.emit_short(v);
        }
        Operand::Closure => {
            let cidx: u16 = toks.next().ok_or("missing closure const")?.parse().map_err(|_| "bad closure const")?;
            proto.emit_short(cidx);
            // expect `pairs N` then N pairs of (is_local index)
            let pairs_kw = toks.next().ok_or("missing pairs keyword")?;
            if pairs_kw != "pairs" {
                return Err(format!("expected 'pairs', found {pairs_kw}"));
            }
            let n: usize = toks.next().ok_or("missing pairs count")?.parse().map_err(|_| "bad pairs count")?;
            for _ in 0..n {
                let is_local: u8 = toks.next().ok_or("missing upvalue is_local")?.parse().map_err(|_| "bad is_local")?;
                let index: u8 = toks.next().ok_or("missing upvalue index")?.parse().map_err(|_| "bad upvalue index")?;
                proto.emit(is_local);
                proto.emit(index);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::compile;
    use crate::lexer::tokenize;
    use crate::parser::parse;

    fn program(src: &str) -> Program {
        compile(&parse(tokenize(src).unwrap()).unwrap()).unwrap()
    }

    #[test]
    fn text_round_trip_simple() {
        let p = program("let a = 1; let b = 2; return a + b * 3;");
        let text = disassemble(&p);
        let back = assemble(&text).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn text_round_trip_with_functions_and_closures() {
        let src = "fn make(x){ fn add(n){ return n + x; } return add; } let a = make(5); return a(3);";
        let p = program(src);
        let text = disassemble(&p);
        let back = assemble(&text).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn text_round_trip_control_flow() {
        let src = "let i=0; let s=0; while(i<10){ if(i==3){ s=s+100; } s=s+i; i=i+1; } return s;";
        let p = program(src);
        let text = disassemble(&p);
        assert_eq!(assemble(&text).unwrap(), p);
    }
}
