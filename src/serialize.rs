//! Binary serialization of a compiled `Program`.
//!
//! The format is a compact little-endian encoding. `deserialize(serialize(p))`
//! reproduces `p` exactly, so a program can be compiled once, written to bytes,
//! read back, and executed with identical results.

use crate::chunk::{Constant, FuncProto, Program};

const MAGIC: &[u8; 4] = b"KDLB";
const VERSION: u8 = 1;

const TAG_NIL: u8 = 0;
const TAG_BOOL: u8 = 1;
const TAG_INT: u8 = 2;
const TAG_FLOAT: u8 = 3;
const TAG_STR: u8 = 4;
const TAG_FUNC: u8 = 5;

pub fn serialize(program: &Program) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(MAGIC);
    b.push(VERSION);
    put_u32(&mut b, program.main as u32);
    put_u32(&mut b, program.funcs.len() as u32);
    for f in &program.funcs {
        put_str(&mut b, &f.name);
        put_u32(&mut b, f.arity as u32);
        put_u32(&mut b, f.upvalue_count as u32);
        put_u32(&mut b, f.constants.len() as u32);
        for c in &f.constants {
            put_constant(&mut b, c);
        }
        put_u32(&mut b, f.code.len() as u32);
        b.extend_from_slice(&f.code);
    }
    b
}

pub fn deserialize(bytes: &[u8]) -> Result<Program, String> {
    let mut c = Cursor::new(bytes);
    let magic = c.take(4)?;
    if magic != MAGIC {
        return Err("bad magic: not a Kindling bytecode blob".into());
    }
    let version = c.u8()?;
    if version != VERSION {
        return Err(format!("unsupported bytecode version {version}"));
    }
    let main = c.u32()? as usize;
    let func_count = c.u32()? as usize;
    let mut funcs = Vec::with_capacity(func_count);
    for _ in 0..func_count {
        let name = c.string()?;
        let arity = c.u32()? as usize;
        let upvalue_count = c.u32()? as usize;
        let const_count = c.u32()? as usize;
        let mut constants = Vec::with_capacity(const_count);
        for _ in 0..const_count {
            constants.push(c.constant()?);
        }
        let code_len = c.u32()? as usize;
        let code = c.take(code_len)?.to_vec();
        funcs.push(FuncProto {
            name,
            arity,
            upvalue_count,
            code,
            constants,
        });
    }
    Ok(Program { funcs, main })
}

fn put_u32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}

fn put_str(b: &mut Vec<u8>, s: &str) {
    put_u32(b, s.len() as u32);
    b.extend_from_slice(s.as_bytes());
}

fn put_constant(b: &mut Vec<u8>, c: &Constant) {
    match c {
        Constant::Nil => b.push(TAG_NIL),
        Constant::Bool(v) => {
            b.push(TAG_BOOL);
            b.push(u8::from(*v));
        }
        Constant::Int(n) => {
            b.push(TAG_INT);
            b.extend_from_slice(&n.to_le_bytes());
        }
        Constant::Float(x) => {
            b.push(TAG_FLOAT);
            b.extend_from_slice(&x.to_bits().to_le_bytes());
        }
        Constant::Str(s) => {
            b.push(TAG_STR);
            put_str(b, s);
        }
        Constant::Func(i) => {
            b.push(TAG_FUNC);
            put_u32(b, *i as u32);
        }
    }
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Cursor { data, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.pos + n > self.data.len() {
            return Err("unexpected end of bytecode".into());
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, String> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn i64(&mut self) -> Result<i64, String> {
        let s = self.take(8)?;
        let mut arr = [0u8; 8];
        arr.copy_from_slice(s);
        Ok(i64::from_le_bytes(arr))
    }

    fn f64(&mut self) -> Result<f64, String> {
        let s = self.take(8)?;
        let mut arr = [0u8; 8];
        arr.copy_from_slice(s);
        Ok(f64::from_bits(u64::from_le_bytes(arr)))
    }

    fn string(&mut self) -> Result<String, String> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| "invalid utf8 in bytecode".into())
    }

    fn constant(&mut self) -> Result<Constant, String> {
        let tag = self.u8()?;
        match tag {
            TAG_NIL => Ok(Constant::Nil),
            TAG_BOOL => Ok(Constant::Bool(self.u8()? != 0)),
            TAG_INT => Ok(Constant::Int(self.i64()?)),
            TAG_FLOAT => Ok(Constant::Float(self.f64()?)),
            TAG_STR => Ok(Constant::Str(self.string()?)),
            TAG_FUNC => Ok(Constant::Func(self.u32()? as usize)),
            other => Err(format!("unknown constant tag {other}")),
        }
    }
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
    fn binary_round_trip() {
        let src = "fn fib(n){ if(n<2){return n;} return fib(n-1)+fib(n-2); } let s = \"hi\"; return fib(7);";
        let p = program(src);
        let bytes = serialize(&p);
        let back = deserialize(&bytes).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn rejects_bad_magic() {
        assert!(deserialize(b"nope").is_err());
    }
}
