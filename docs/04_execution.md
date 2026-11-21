# Execution

How rpsh runs a parsed script. Settles #4; DESIGN.md §3.4 summarises it.
Measurements: `bench/baselines/2026-11-21-ryzen5-4650u-execution.json`,
written by `cargo bench -p rpsh-research --bench execution`.

## What upstream does

Upstream does not choose between an interpreter and a compiler: it runs a
script block in an interpreter first and compiles it once it is hot. The
interpreter walks LINQ expression trees (`engine/interpreter`), and after
enough executions the block is compiled and the call sites switch over. The
cost is paid twice — once for interpretation, again for compilation — and
only scripts that run long enough get the second one.

rpsh starts from a cold process on every invocation. A long-running loop is
the one place upstream's JIT can win, so the choice has to keep that loop
cheap without paying a compiler at startup.

## The prototypes

Five evaluators run the two programs `bench/cases/language` already times,
built as ASTs directly so that parsing is not part of the measurement:

```powershell
# loop-sum.ps1
$sum = 0
for ($i = 0; $i -lt 1000000; $i++) { $sum += $i }
$sum

# function-calls.ps1
function Fib($n) { if ($n -lt 2) { return $n }; return (Fib ($n - 1)) + (Fib ($n - 2)) }
Fib 18
```

* **ast-names**: walk the AST; every variable is a lookup by name through a
  chain of hash-map scopes.
* **ast-slots**: resolve variables to frame slots before running, as
  upstream's `VariableAnalysis` does, and fall back to a name lookup where
  the variable is not definitely assigned.
* **stack-vm**: bytecode for an operand stack, with the same slots.
* **register-vm**: instructions name their source and destination
  registers; variables live in registers, as in Lua and Nushell's IR.
* **register-vm+call-cache**: the register VM with an inline cache at each
  call site. The cache records which function a name resolved to and a
  generation number, which a redefinition of the function bumps.

All five share the same 16-byte `Value`, the same dynamically typed
operators (`Int32` overflow promotes to `Double`, so `$sum` ends as a
`Double`), the same dynamic function lookup and positional binding, and the
same output. Each answer is checked against pwsh's before it is timed:
`499999500000` for loop-sum and `2584` for Fib 18.

Loop-sum, one million iterations, median wall time:

| candidate | time | relative |
|---|---:|---:|
| ast-names | 192.1 ms | 3.16 |
| ast-slots | 91.1 ms | 1.50 |
| stack-vm | 93.2 ms | 1.53 |
| register-vm | **60.8 ms** | **1.00** |
| register-vm+call-cache | 61.0 ms | 1.00 |

Fib 18, 13,529 calls, median wall time:

| candidate | time | relative |
|---|---:|---:|
| ast-names | 1.92 ms | 2.20 |
| ast-slots | 1.33 ms | 1.53 |
| stack-vm | 1.05 ms | 1.21 |
| register-vm | 1.10 ms | 1.26 |
| register-vm+call-cache | **873 µs** | **1.00** |

Lowering a synthetic 5,800-node script, which is the size of a profile:

| candidate | time |
|---|---:|
| ast-slots | 254 µs |
| stack-vm | 226 µs |
| register-vm | 220 µs |
| register-vm+call-cache | 230 µs |

Sizes: a stack-VM `Op` is 8 bytes, a register-VM `Ins` is 16, and `Value` is
16. Loop-sum lowers to 13 AST nodes, 21 stack operations and 14 register
instructions.

## Prior art

Nushell is the only shell here that executes a register IR. It compiles each
block at parse time and `eval_block` always runs the IR evaluator, with no AST
fallback (Nushell 0.117.0 (`a481925`),
`crates/nu-parser/src/parse_captures_compile.rs:872-876`,
`crates/nu-engine/src/eval.rs:602-613`); a block that fails to compile is
reported, not interpreted from the tree (`crates/nu-engine/src/eval_ir.rs:147-167`).
The instruction set has 58 variants and 21 literal variants; `Instruction` is
capped at 24 bytes and `RegId` is a `u32` (`crates/nu-protocol/src/ir/mod.rs:100-313`,
`:465`, `:468-511`; `crates/nu-protocol/src/id.rs:123`). Registers carry
pipeline data, variables stay in `Stack.vars` by `VarId`, branch targets are
absolute instruction indices, and strings and names go in a shared `data`
buffer addressed by `DataSlice` (`crates/nu-engine/src/eval_ir.rs:108-113,241-250`,
`crates/nu-protocol/src/engine/stack.rs:76,97`,
`crates/nu-engine/src/compile/builder.rs:595-608`, `crates/nu-protocol/src/ir/mod.rs:77-83`).

The IR is serde-serializable and is used by `view ir --json` and the plugin
protocol (`crates/nu-protocol/src/ir/mod.rs:36,79,100,468,522`,
`crates/nu-command/src/debug/view_ir.rs:161-166`,
`crates/nu-plugin-protocol/src/lib.rs:711`), but Nushell keeps no on-disk IR
or script cache. The only reuse is an in-memory block cache for the same span
in the same process (`crates/nu-parser/src/parse_captures_compile.rs:818-847`).
Nushell also commits no IR-versus-AST benchmark, so its IR is evidence that
the design is workable at shell scale, not that it is faster than a tree walk
(inference).

The other four shells walk their AST directly and keep parsed function bodies,
so a call does not re-parse. fish keeps `NodeRef<ast::BlockStatement>`
(fish 4.9.3 (`6cf3137`), `src/function.rs:31`) and walks it at call time
(`src/exec.rs:1034,1056`); brush stores an `Arc<FunctionDefinition>`
(brush `brush-shell-v0.4.0` (`96a26d0`), `brush-core/src/functions.rs:67`) and
is the only one with parse memoisation, `cached` at 64 entries in memory
(`brush-core/src/shell/parsing.rs:47-48`); yash-rs keeps
`Rc<FullCompoundCommand>` (yash-rs `yash-cli-3.6.0` (`46d2f79`),
`yash-syntax/src/syntax.rs:709`); ion walks a `Vec<Statement>`
(ion `1.0.0-alpha` (`e9b3ca4`), `src/lib/shell/flow_control.rs:96,236`). None
has an on-disk cache of parsed code; ion's only reuse is a thread-local scratch
buffer (`src/lib/memory.rs:20`). Inference: an AST walker is the default
because it is cheaper to build, and its per-node lookup cost is the floor rps
measures against.

Takeaway: rps borrows Nushell's register IR and resolved call targets (a
`DeclId` on the call instruction, `crates/nu-protocol/src/ir/mod.rs:206`), and
diverges where none of these shells does — it writes the lowered block to disk
because lowering costs 220 µs, a cost the AST walkers never pay and Nushell
pays on every start. The four tree walkers set the build-cost floor; the
measurements above are what justify paying the register IR's extra upfront
work.

## Decision

**Lower the AST to a register IR and interpret it.**

* The register form is 3.2 times faster than the name-walked AST on the
  tight loop and 1.5 times faster than the slot-resolved AST or the stack
  VM. On the call-heavy program the call cache is the fastest of the five,
  21% ahead of the plain register VM and 17% ahead of the stack VM. The win
  is not from better dispatch alone: naming registers removes the stack
  traffic and lets the interpreter keep the loop variables in one place.
* Resolution happens at lowering time, not at run time. The `ast-slots`
  prototype already shows most of the gain over names; the bytecode and IR
  forms then stop re-deciding where a variable lives on every iteration.
* Lowering costs 220 µs for a 5,800-node script. That is a real startup
  cost, so lowering is performed on first execution of a block and the
  result is cached (DESIGN.md §4 rule 3). The IR is a flat, serializable
  vector of fixed-size instructions, so the cache can be written to disk
  keyed by the script's content hash, the engine version and the target; it
  is not tied to a process.
* The call cache is kept even though it does not show on loop-sum: it turns
  a name lookup into a generation check at every call site and takes 21% off
  Fib 18 over the plain register VM. Redefining a function bumps the generation, so a cached call can
  never call the old body.
* A compiler is not required now. The register IR is the shape a later
  copy-and-patch or JIT tier would consume, so adding one does not mean
  throwing the interpreter away; it means replacing the dispatch loop for
  blocks that become hot (#147).

Rejected:

* **Walk the AST directly**, even with slots. Simpler, but 1.5 times slower
  on the loop and it leaves the tree in cache while running. It stays as
  the reference implementation in the bench.
* **Stack bytecode.** No faster than the slot-resolved AST on loop-sum and
  slower than registers on both programs; its instructions are smaller (8
  bytes against 16), which does not matter at these code sizes.
* **Compile everything on first run.** No measured prototype pays back a
  compiler on a one-shot script, and it contradicts the startup budget
  (docs/05_startup.md).
* **A tracing JIT now.** It would answer the hot-loop question directly,
  but it needs the interpreter and the caches in place first, and it is
  most of a new project. #147 owns that measurement.

## Dynamic sites and their caches

The IR is typed only where lowering could prove a type. Everything else is
a dynamic instruction with a cache. Each cache is validated before use; a
miss falls back to the slow path, which is the full PowerShell rule, and a
hit must produce the same result as the slow path.

| site | cache | invalidation |
|---|---|---|
| member get/set | declaring type, member slot, kind (property, field, method, extended) | type table generation, `Add-Member`, `Update-TypeData` |
| method call | receiver type, overload, arity and argument conversions | type table generation |
| command call | resolved command (function, alias, cmdlet, native program) and binder | command table generation, scope exit |
| conversion | source type, target type, conversion function | type table generation |
| operator | operand types, operation | never (operators are fixed) |
| index | container type, index kind | type table generation |

The generation numbers are per session state. A `function` definition, an
alias, an imported module or a type-data update bumps the one it belongs to,
and every cache that names the old world misses. This is the job the DLR's
call-site caching does upstream (`engine/runtime/Binding/Binders.cs`); here
it is explicit and small enough to test one site at a time.

## What the implementation issues take from this

| Issue | Takes |
|---|---|
| #55 | register IR, slots and frames, lowering on first execution, inline caches |
| #57 | control flow lowering: `if`, `switch`, loops, labels, `break` / `continue` |
| #58 | function and script-block bodies as IR, closures, `$args` / `$input` |
| #69 | serialize a lowered block into the on-disk script cache |
| #72 | safe points at statement boundaries, which the memory collector (#3) and Ctrl-C both use |
| #147 | when, if ever, to replace the dispatch loop with compiled code |
