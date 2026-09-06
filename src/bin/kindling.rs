//! Kindling command-line tool: run a source file, disassemble it, or start a
//! REPL.

#![warn(clippy::pedantic)]

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use kindling::value::Outcome;
use kindling::{compile_source, disassemble, run_program, run_source};

fn usage() {
    eprintln!("kindling: a from-scratch bytecode language runtime");
    eprintln!();
    eprintln!("usage:");
    eprintln!("  kindling run <file.kdl>      compile and execute a program");
    eprintln!("  kindling disasm <file.kdl>   print the compiled bytecode");
    eprintln!("  kindling repl                start an interactive session");
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        usage();
        return ExitCode::FAILURE;
    }
    let result = match args[1].as_str() {
        "run" => cmd_run(args.get(2)),
        "disasm" => cmd_disasm(args.get(2)),
        "repl" => cmd_repl(),
        "--help" | "-h" | "help" => {
            usage();
            Ok(())
        }
        other => Err(format!("unknown command '{other}'")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn read_file(path: Option<&String>) -> Result<String, String> {
    let path = path.ok_or("expected a file path")?;
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))
}

fn cmd_run(path: Option<&String>) -> Result<(), String> {
    let src = read_file(path)?;
    let result = run_source(&src)?;
    print!("{}", result.output);
    if result.value != Outcome::Nil {
        println!("=> {}", render(&result.value));
    }
    Ok(())
}

fn cmd_disasm(path: Option<&String>) -> Result<(), String> {
    let src = read_file(path)?;
    let program = compile_source(&src)?;
    print!("{}", disassemble(&program));
    Ok(())
}

fn cmd_repl() -> Result<(), String> {
    println!("Kindling REPL. Type an expression or statement, or 'quit' to exit.");
    let stdin = io::stdin();
    let mut defs: Vec<String> = Vec::new();
    loop {
        print!("kdl> ");
        io::stdout().flush().ok();
        let mut line = String::new();
        let n = stdin.lock().read_line(&mut line).map_err(|e| e.to_string())?;
        if n == 0 {
            println!();
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "quit" || trimmed == "exit" {
            break;
        }

        let is_definition = trimmed.starts_with("let ") || trimmed.starts_with("fn ");
        let mut source = defs.join("\n");
        source.push('\n');
        source.push_str(trimmed);
        if is_definition {
            // Definitions produce no value; keep them for later lines.
            match compile_source(&source) {
                Ok(_) => defs.push(trimmed.to_string()),
                Err(e) => eprintln!("error: {e}"),
            }
            continue;
        }

        // Expression or statement: wrap so its value is returned.
        let wrapped = if trimmed.ends_with(';') || trimmed.ends_with('}') {
            source.clone()
        } else {
            format!("{source}\nreturn {trimmed};")
        };
        match compile_source(&wrapped).and_then(|p| run_program(&p)) {
            Ok(result) => {
                print!("{}", result.output);
                if result.value != Outcome::Nil {
                    println!("=> {}", render(&result.value));
                }
            }
            Err(e) => eprintln!("error: {e}"),
        }
    }
    Ok(())
}

fn render(v: &Outcome) -> String {
    match v {
        Outcome::Nil => "nil".to_string(),
        Outcome::Bool(b) => b.to_string(),
        Outcome::Int(n) => n.to_string(),
        Outcome::Float(x) => kindling::vm::format_float(*x),
        Outcome::Str(s) => format!("{s:?}"),
        Outcome::Func => "<fn>".to_string(),
    }
}
