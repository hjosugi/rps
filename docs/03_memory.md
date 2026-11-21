# Memory management

How rpsh frees what scripts allocate. Settles #3; DESIGN.md §3.3 summarises
it. Measurements: `bench/baselines/2026-11-21-ryzen5-4650u-memory.json`,
written by `cargo bench -p rpsh-research --bench memory`.

## What scripts do

Probed on pwsh 7.7.0:

```text
$h = @{ n = 0 }; $h.self = $h                       → a hashtable that contains itself
$o | Add-Member -NotePropertyName me -NotePropertyValue $o   → an object that points at itself
$sb = { $sb }; & $sb                                 → a script block reachable from its own scope
1..8 | ForEach-Object -Parallel { ($using:shared).count++ … }   → 8: the same object in four threads
Start-ThreadJob { param($x) $x.added = 'by job' } -ArgumentList $h
$h.added                                             → by job: the job mutated the caller's object
```

Two requirements follow:

1. **Cycles are ordinary.** Pure reference counting would leak every one of
   them.
2. **Objects are shared between threads, by identity.** `$using:` and
   `-ArgumentList` hand the very same object to another runspace, and
   mutations are visible both ways. A design in which values belong to one
   thread and are copied when they cross would print something else.

## Candidates

| | Shared across threads | Cycles | Cost model |
|---|---|---|---|
| `Rc` per runspace, deep copy at thread boundaries | no: breaks identity | needs a collector | cheapest counts |
| `Arc` + a backup cycle collector (CPython's design) | yes | collector over containers | atomic counts, locks on mutable containers |
| tracing GC with handles | yes | yes | no counting; roots must be tracked everywhere, including Rust locals in cmdlets |
| no collection: free at exit | yes | leak until exit | nothing, until memory runs out |

## Prior art

None of the surveyed shells has a tracing collector: Nushell, fish, brush,
yash-rs and ion all free by reference count or by ownership (Nushell 0.117.0
(`a481925`), crates/nu-protocol/src/value/mod.rs:79; fish 4.9.3 (`6cf3137`),
src/env/var.rs:157; brush brush-shell-v0.4.0 (`96a26d0`), brush-core/src/env.rs:94;
yash-rs yash-cli-3.6.0 (`46d2f79`), yash-env/src/variable.rs:212; ion master
(`e9b3ca4`), src/lib/shell/variables.rs:15). They avoid cycles rather than
collecting them:

- **Nushell** copies closure captures by value at evaluation
  (`crates/nu-engine/src/eval_ir.rs:1412`), rejects capturing a `mut`
  variable at parse time (`ParseError::CaptureOfMutableVar`,
  `crates/nu-protocol/src/errors/parse_error.rs:228`), and puts Record, List
  and Binary behind copy-on-write `Arc`s
  (`crates/nu-utils/src/shared_cow.rs:10`). "Built-in values cannot form
  cycles" is the survey's inference, and it follows from those three.
- **fish** stores each variable as an immutable `Arc<[WString]>`
  (`src/env/var.rs:157`) in an `Arc`-linked chain of scopes under one global
  `Mutex` (`src/env/impl/environment.rs:144`, `:998`); lookup clones the
  value (`:162`).
- **brush** clones the whole `Shell` for subshells and command substitutions
  (`brush-core/src/shell.rs:148`, `brush-core/src/interp.rs:683`); values are
  owned, not copy-on-write (inference).
- **yash-rs** keeps one map with a per-name context stack
  (`all_variables: HashMap<String, Vec<VariableInContext>>`,
  `yash-env/src/variable.rs:212`) and calls a real `fork()` for subshells
  (`yash-env/src/system/real.rs:932`); functions are `Rc`
  (`yash-env/src/function.rs:181`).
- **ion** keeps a `Vec` of per-scope `HashMap`s
  (`members/scopes-rs/src/lib.rs:16`), clones values on read
  (`src/lib/shell/variables.rs:217`), and puts only functions behind `Rc`.

That is not evidence that rpsh can skip collection. The probes above are the
cycles these shells avoid — a hashtable that contains itself, an object that
points at itself, a script block reachable from its own scope — and each
shell avoids them by copying captures, isolating the process, cloning the
shell or treating values as immutable. PowerShell keeps identity across
closures, jobs and runspaces, so rpsh pays for `Arc` and must collect.

Takeaway: rpsh borrows the cheap part, `Arc` and copy-on-write where mutation
is rare, and adds the cycle collector that no surveyed shell needs.

## Measurements

Median nanoseconds per operation, uncontended, on the reference machine:

| operation | `Rc` + `RefCell` | `Arc` + `Mutex` | `Arc` + `RwLock` |
|---|---:|---:|---:|
| clone + drop | 3.62 ns | 9.25 ns | — |
| element read + write | 2.13 ns | 9.00 ns | — |
| element read | 0.86 ns | 8.62 ns | 8.67 ns |

For scale: pwsh runs `bench/cases/language/loop-sum.ps1`, a million
iterations of `$sum += $i`, in 1,080 ms including about 150 ms of startup
(`bench/baselines/2026-11-20-ryzen5-4650u-scaffold.json`), roughly 0.9 µs
per iteration. An atomic count pair or an uncontended lock costs tens of
nanoseconds. It is visible in a micro benchmark and invisible against the
interpreter work each operation already does; the hot-loop budget is #147's.

The cycle collector prototype (`Heap` in the bench) is CPython's algorithm
in safe Rust: every container is registered weakly, and a collection
subtracts, from each container's reference count, the references held by
other containers. What remains is held from outside — a variable, a
register, the pipeline — and everything reachable from there survives; the
rest is cyclic garbage and is broken up.

* **Garbage loop**: 1,000,000 iterations of `$h = @{…}; $h.self = $h`.
  Without a collector the loop runs in 196.1 ns per iteration and leaves
  325 MiB behind; collecting every 10,000 containers costs 197.1 ns per
  iteration and the RSS does not grow at all, because freed containers are
  reused. Collecting every 100,000 costs 337.1 ns per iteration. A
  collection every 10,000 allocations costs about the same as never
  collecting at this size, and it is the threshold the implementation
  starts with.
* **Pause**: a collection costs 212 µs with 1,000 live containers, 3.49 ms
  with 10,000 and 55.7 ms with 100,000. Linear in the live heap, as expected
  of a non-generational collector. The prototype pays for a hash map and a
  lock per container; an implementation with the count bookkeeping in the
  object header (as CPython does) is cheaper per object, but linear all the
  same. This is why the collector is generational and why the threshold
  keeps young scripts from ever reaching it.

## Decision

* **Reference counting with `Arc`** for every heap object. Immutable objects
  (strings, boxed numbers, script block bodies) need nothing more. Mutable
  containers (arrays, hashtables, lists, `PSObject` members, variable
  scopes) guard their contents with a lock, which is uncontended in the
  common case of one runspace.
* **A backup cycle collector over containers only**, by trial deletion.
  * It is triggered by the number of containers allocated since the last
    collection, so a short script never pays for it (the startup budget,
    docs/05).
  * It is generational: young containers are examined first, old ones
    rarely, because the pause is linear in what is examined.
  * It runs at safe points (statement boundaries in the VM, #55), where
    every reference the engine holds is in a variable, a register or a
    container, never only in a Rust local. A collection stops all runspaces
    of the process at their next safe point. With one runspace, which is
    nearly always, there is nothing to coordinate.
  * The threshold starts at 10,000 container allocations and is tuned in
    #149.
* **Deterministic release, .NET's observable behaviour.** An object is freed
  when its last reference goes, which is earlier than .NET's GC. Freeing
  must not do what .NET would not do at that point. For example, a
  `StreamWriter` that a script never disposes must not be flushed by being
  dropped if .NET would not flush it. Each shim type that owns a resource
  states what its drop does (#51).
* **`Send` and `Sync`.** Values are `Send + Sync`. Engine state is shared
  immutably, and each runspace owns its stack (DESIGN.md §3.6).

Rejected:

* **Thread-local `Rc` with copies at thread boundaries**: contradicts the
  probes above.
* **A tracing GC**: no counting traffic, but every handle held in Rust code
  across a call back into the engine (a cmdlet invoking a script block) has
  to be rooted explicitly. That burden falls on every cmdlet. Reconsider
  only if reference-count traffic shows up in profiles of real scripts
  (#149).
* **Never collecting**: the garbage loop above.

Left open for #149: biased reference counting (the owning thread counts
without atomics, others atomically; Swift and free-threaded CPython do this)
if atomic counts ever matter, and the collector's thresholds.
