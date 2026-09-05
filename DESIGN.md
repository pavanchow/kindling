# Kindling design

This document describes how Kindling is built: the architecture, the language grammar, the bytecode format, the virtual machine execution model, the garbage collector, and why each correctness gate proves what it claims. There are no external dependencies anywhere in the system.

## Architecture

Kindling is a classic pipeline. Source text flows through five stages.

```
source -> lexer -> tokens -> parser -> AST -> compiler -> bytecode -> VM -> value
                                          \
                                           -> reference interpreter -> value
```

The lexer turns characters into tokens. The parser turns tokens into an abstract syntax tree. From the AST there are two independent consumers. The compiler lowers the AST into bytecode for the virtual machine to execute. Separately, a tree walking reference interpreter evaluates the same AST directly. The two evaluators share the value semantics of the language but share none of their execution machinery, which is the foundation of the differential correctness gate.

A compiled program is a flat table of function prototypes plus the index of the entry function. Nested functions are referenced by index through a function constant, which keeps both the binary serializer and the text assembler simple and free of recursion.

## Language grammar

The grammar below uses a star for zero or more and a question mark for optional. A program is a sequence of statements executed top to bottom.

```
program    = statement*
statement  = letStmt | fnDecl | ifStmt | whileStmt | returnStmt
           | printStmt | block | exprStmt
letStmt    = "let" IDENT "=" expression ";"
fnDecl     = "fn" IDENT "(" params? ")" block
params     = IDENT ("," IDENT)*
ifStmt     = "if" "(" expression ")" block ("else" (ifStmt | block))?
whileStmt  = "while" "(" expression ")" block
returnStmt = "return" expression? ";"
printStmt  = "print" expression ";"
block      = "{" statement* "}"
exprStmt   = expression ";"

expression = assignment
assignment = IDENT "=" assignment | equality
equality   = comparison (("==" | "!=") comparison)*
comparison = term (("<" | "<=" | ">" | ">=") term)*
term       = factor (("+" | "-") factor)*
factor     = unary (("*" | "/" | "%") unary)*
unary      = ("-" | "!") unary | call
call       = primary ("(" args? ")")*
args       = expression ("," expression)*
primary    = INT | FLOAT | STRING | "true" | "false" | "nil"
           | IDENT | "(" expression ")"
```

Precedence runs from assignment at the bottom up through equality, comparison, additive, multiplicative, unary, and call. The parser is a straightforward recursive descent parser with one function per precedence level.

### Value semantics

Both evaluators implement the same rules. Integer arithmetic uses wrapping on overflow so it can never panic. If either operand is a float, the result is a float. The plus operator concatenates two strings. Division or modulo by a zero divisor is a runtime error. The comparison operators work on numbers. Equality compares by value, with an integer and a float comparing numerically and two strings comparing by content. Only nil and false are falsey.

The value produced by a program is the value of its last statement, where an expression statement yields its value and other statements yield nil, unless an explicit return fires first. A function call yields its explicit return value, or the value of the last statement in the body, or nil. The compiler and the reference interpreter both implement this rule the same way, so their results line up.

## Bytecode and opcode format

Bytecode is a flat vector of bytes per function. The first byte of an instruction is the opcode. Operands follow inline. A short operand is two bytes, big endian. A byte operand is one byte. One opcode, the closure opcode, is variable length.

The opcode set:

```
CONST ci        push constant ci
NIL TRUE FALSE  push the literal
POP             discard the top value
NEG NOT         unary negate, logical not
ADD SUB MUL DIV MOD          arithmetic, pop two push one
EQ NEQ LT LE GT GE           comparison, pop two push one bool
DEF_GLOBAL ci   define a global named by string constant ci
GET_GLOBAL ci   push the value of a global
SET_GLOBAL ci   assign a global, leave the value on the stack
GET_LOCAL slot  push a local from a stack slot
SET_LOCAL slot  assign a local, leave the value on the stack
GET_UPVALUE i   push a captured upvalue
SET_UPVALUE i   assign a captured upvalue
JUMP off        move the instruction pointer forward by off
JUMP_IF_FALSE off   move forward by off if the top value is falsey
LOOP off        move the instruction pointer backward by off
CALL argc       call the value argc slots below the top
CLOSURE ci ...  build a closure from function constant ci
RETURN          return the top value from the current frame
PRINT           pop and print the top value
```

A function prototype holds its name, its arity, its upvalue count, its constant pool, and its code bytes. The constant pool holds nil, booleans, integers, floats, strings, and function references. Global variables are addressed by interning their name as a string constant and referencing that constant, which keeps the code stream compact and the name available for error messages.

The closure opcode carries a short constant index that points at a function reference, followed by one pair of bytes per upvalue. The first byte of a pair says whether the captured value comes from a local slot of the enclosing frame or from an upvalue of the enclosing closure, and the second byte is the index. The number of pairs equals the referenced function upvalue count.

Jumps are computed at compile time by emitting a placeholder, compiling the branch, then patching the two operand bytes with the distance. Loops emit a backward distance directly. This is the standard patch and back patch technique.

### Two serial forms

The bytecode has two lossless external representations. The disassembler renders a program as a readable, line oriented text listing with a header, a per function block, a constant pool, and one line per instruction annotated with jump targets. The assembler parses that listing back into an identical program. The serializer writes a compact little endian binary blob with a magic tag and a version byte, and the deserializer reads it back. Both forms reproduce the original program exactly.

## Virtual machine execution model

The VM is a stack machine with two stacks. The value stack holds operands and local variables. The call frame stack holds one frame per active function call. A frame records the closure being run, the function index, the instruction pointer, and the slot base, which is the value stack index where the frame slot zero lives.

Slot zero of every frame is reserved for the function or closure being executed. Parameters occupy the slots immediately after it, and further locals follow as they are declared, so a local read or write is a direct index off the slot base. When a call happens, the callee and its arguments are already on the stack in exactly the right layout to become the new frame, so a call is just pushing a frame whose slot base points at the callee. A return pops the return value, truncates the stack back to the slot base, pops the frame, then pushes the return value for the caller.

Globals live in a hash map keyed by name. Control flow is the three jump opcodes. The conditional jump leaves the tested value on the stack and the compiled code pops it in both branches, which keeps the opcode simple.

Closures are created by the closure opcode. Kindling captures upvalues by value at the moment the closure is created. This keeps the machine simple and covers the common case of a closure that reads values from its defining scope. It does mean a closure does not share later mutations of a captured variable with the enclosing scope, and that a locally defined function cannot call itself through capture. Recursion is expressed with top level functions, which resolve through the global table and so recurse cleanly. The reference interpreter models closures with a captured environment, which agrees with the VM on read only capture, the case the runtime supports and the generator exercises.

## Garbage collector design

Reference types, strings and closures, live on a managed heap. Every heap object sits in a slot addressed by a small handle. A runtime value is a copyable tagged union where the reference case holds a handle rather than owning the object, so values move freely without touching the heap.

Collection is precise mark and sweep. The mark phase clears all marks, seeds a work list from the root values, and walks it. Marking an object also enqueues the handles it contains, so a closure marks each captured upvalue that is itself a reference. The sweep phase frees every slot that was allocated but not marked and records the slot for reuse. Freed slots are recycled by later allocations.

During execution the roots are the entire value stack, every global, and the closure of every active call frame. Collection runs only at the top of the dispatch loop, a safe point where every live temporary is on the value stack and nothing is held off to the side, so nothing reachable can be missed. Auto collection triggers once allocations since the last collection cross a growing threshold, and a stress mode collects after almost every allocation for testing.

## Why each gate proves what it claims

### Differential testing

The strongest correctness claim a runtime can make is that its answers are right. Kindling checks this with an oracle that does not trust the VM: an independent tree walking interpreter that evaluates the same AST with no bytecode, no value stack, and no call frames. For every generated program the two must agree on both the produced value and any printed output. Because the two evaluators share only the language value rules and none of their execution machinery, agreement across a large, randomized program space is strong evidence that the compiler and the VM together preserve the meaning of the program. A bug in code generation, in jump patching, in slot addressing, in the calling convention, or in stack discipline would make the two disagree. The generator emits programs that always terminate and never error, covering arithmetic, variables, assignment, conditionals, bounded loops, functions, and terminating recursion, and the program count is tunable so the same gate serves both a fast build and a deep soak.

### Round trip integrity

A bytecode format is only useful if it survives being written down and read back. The text round trip asserts that assembling a disassembly reproduces the exact program, which proves the human readable form is lossless and that the disassembler and assembler are true inverses. The binary round trip asserts that deserializing a serialization reproduces the exact program, which proves the on disk form is lossless. The run after round trip check goes further and asserts that a program taken through the binary form and back computes the same result as the original, which proves the encoding preserves not just structure but behavior. Together these prove a program can be compiled once, stored or shipped, and executed later with identical results.

### Garbage collector correctness

A collector has two failure modes: freeing something still in use, and never freeing something dead. The gate checks both against a known truth. With a hand built reachability graph, where the reachable set and the dead set are known exactly, a collection must free the whole dead set and keep the whole reachable set, including objects reachable only indirectly through a closure capture. The churn test allocates piles of garbage across many rounds and asserts that a designated root is always live afterward, which rules out use after free, and that all the garbage is freed each round, which rules out leaks. The VM stress test runs a real program that allocates on every loop iteration with collection forced after almost every allocation, and asserts the final result is still correct, which proves the collector never frees a live object in the middle of execution. Knowing the exact reachable set ahead of time is what turns these from smoke tests into proofs about specific objects.
