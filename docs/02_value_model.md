# Value model

How rpsh represents PowerShell values, strings above all. Settles #2;
DESIGN.md §3.3 summarises it. Measurements:
`bench/baselines/2026-11-21-ryzen5-4650u-value-model.json`; the bench writes
`bench/results/research-value_model.json`, and that run is copied there
(DESIGN.md §7 rule 5).

## What has to be exact

Probed on pwsh 7.7.0 in the reference environment:

| Script | pwsh |
|---|---|
| `'😀'.Length`, `'日本語'.Length`, `"e$([char]0x301)".Length` | 2, 3, 2 |
| `'😀'[0]` | `[char]` 0xD83D, a lone high surrogate |
| `"a$([char]0xD800)b".Length` | 3: a lone surrogate is a valid string |
| the same string written to stdout | bytes `61 EF BF BD 62`: U+FFFD only at the UTF-8 boundary |
| comparer of `@{}` (private `_keycomparer`) | `System.OrdinalIgnoreCaseComparer` |
| `'ﬀ' -eq 'ff'`, `"e$([char]0x301)" -eq 'é'`, `'ß' -eq 'ss'` | True, True, False |
| `@(& {}).Count`, `@($null).Count` | 0, 1 |
| `$x = 1; $r = [ref]$x; $r.Value = 2; $x` | 2 |

Two consequences shape everything below. Strings are sequences of UTF-16
code units, whatever they are stored as. And there are two different string
comparisons: hashtable keys use ordinal-ignore-case (simple case folding,
no culture data), while `-eq` and friends use culture-aware comparison,
which is ICU's (#36).

Value types follow upstream's copy rule. A mutable struct is copied when it
is assigned (`$q = $p; $q.X = 7` leaves `$p.X` alone), an element read
through an indexer is a copy (`$list[0].X = 99` changes nothing), and
`[datetime]` is not copied at all (`[object]::ReferenceEquals($d, $e)` is
True after `$e = $d`). Upstream decides with `IsValueTypeMutable`
(`engine/runtime/Binding/Binders.cs:2038`): primitives, enums and
`readonly` structs are immutable and shared; any struct with a writable
field is copied.

## Strings

Three candidates, each implementing the operations scripts use most, and
checked against a UTF-16 reference before being timed (`verify` in the
bench):

* **utf16**: `Rc<[u16]>`, .NET's own representation.
* **wtf8**: WTF-8 bytes (UTF-8 that can also encode a lone surrogate),
  with the UTF-16 length and an ASCII flag computed once. Reading code unit
  *i* of a non-ASCII string walks from the start.
* **compact**: an ASCII string as bytes, any other string as UTF-16. The
  idea of Java's compact strings and V8's one-byte strings, with ASCII
  instead of Latin-1 as the narrow set so that writing UTF-8 stays a copy.

Median nanoseconds per operation on the reference machine. The corpora are
an 84-character ASCII log line, a 38-character Japanese sentence, and an
79-unit line with emoji (`CORPORA` in the bench).

### ASCII

| operation | utf16 | wtf8 | compact |
|---|---:|---:|---:|
| from UTF-8 | 143.0 ns | 47.2 ns | **29.3 ns** |
| write UTF-8 | 39.8 ns | 5.7 ns | **3.2 ns** |
| `$s[$i]`, every unit | **6.8 ns** | 8.0 ns | 10.7 ns |
| `Substring(2, 10)` | **7.3 ns** | 19.3 ns | 19.9 ns |
| `IndexOf` | 235 ns | **215 ns** | 256 ns |
| ordinal-ignore-case | 109 ns | **11.4 ns** | 11.6 ns |
| ordinal-ignore-case hash | 88.9 ns | **80.5 ns** | 82.8 ns |
| `$s += 'a'`, 2,000 times | 162 µs | **123 µs** | 166 µs |
| payload, one string | 168 B | **84 B** | **84 B** |

### Japanese

| operation | utf16 | wtf8 | compact |
|---|---:|---:|---:|
| from UTF-8 | 97.7 ns | **51.1 ns** | 110 ns |
| write UTF-8 | 251 ns | **90.7 ns** | 255 ns |
| `$s[$i]`, every unit | **4.5 ns** | 2,414 ns | 6.3 ns |
| `Substring(2, 10)` | **7.5 ns** | 263 ns | 25.2 ns |
| `IndexOf` | **107 ns** | 388 ns | 121 ns |
| ordinal-ignore-case | 25.8 ns | 235 ns | **23.4 ns** |
| ordinal-ignore-case hash | 591 ns | 714 ns | **569 ns** |
| `$s += 'あ'`, 2,000 times | **189 µs** | 12.9 ms | 214 µs |
| payload, one string | 76 B | 114 B | **76 B** |

### Emoji

| operation | utf16 | wtf8 | compact |
|---|---:|---:|---:|
| from UTF-8 | 146 ns | **41.3 ns** | 204 ns |
| write UTF-8 | 290 ns | **72.9 ns** | 336 ns |
| `$s[$i]`, every unit | **7.0 ns** | 7,901 ns | 8.5 ns |
| `Substring(2, 10)` | **7.5 ns** | 158 ns | 23.7 ns |
| `IndexOf` | **229 ns** | 420 ns | 265 ns |
| ordinal-ignore-case | 134 ns | 365 ns | **114 ns** |
| ordinal-ignore-case hash | 132 ns | 288 ns | **131 ns** |
| payload, one string | 158 B | **89 B** | 158 B |

What the numbers say:

* **wtf8 loses on non-ASCII indexing; on ASCII it is competitive.**
  Visiting every code unit of the Japanese line costs 2.4 µs, against 4.5 ns
  for utf16 and 6.3 ns for compact; the emoji line is 7.9 µs. On the ASCII
  line it is 8.0 ns against 6.8 ns for utf16, and its `IndexOf` is the
  fastest of the three. The loop is quadratic: `$s += 'あ'`
  2,000 times takes 12.9 ms, against 189 µs and 214 µs. Breadcrumbs (Swift's
  index cache) would fix the asymptotics, at the price of a second index
  structure per string and of code that has to keep it right.
* **compact wins at the boundaries for ASCII.** Creating a string from
  UTF-8 input costs 29.3 ns (utf16: 143 ns), and writing it out is a copy,
  3.2 ns (utf16: 39.8 ns). Paths, identifiers, JSON keys, log lines and most
  native command output are ASCII, and they cross that boundary all the
  time.
* **compact costs about what utf16 costs for most other operations**,
  because then it *is* utf16. The measured exceptions are small in absolute
  terms but not within noise: `Substring` on the Japanese line is 25.2 ns
  against 7.5 ns, indexing the ASCII line is 10.7 ns against 6.8 ns, and
  creating a string from emoji UTF-8 is 204 ns against 146 ns. None of these
  is more than a few tens of nanoseconds, and none is on a path a script
  performs per character; #149 revisits them if a profile disagrees.
* **Memory**: an ASCII string takes one byte per code unit in compact and
  wtf8, two in utf16.

**Decision: compact.** A string is immutable and is either ASCII bytes or
UTF-16 code units, never both, and ASCII content is always stored narrow
(a substring or concatenation that turns out ASCII is narrowed). An
ordinal-ignore-case key comparison therefore never has to compare across
forms. Culture-aware comparison (`-eq` and friends, #36) must: `'ﬀ' -eq 'ff'`
is true even though the two strings take different forms, so the ICU path
transcodes as needed rather than using the narrow/wide shortcut. `.Length`,
indexing and `Substring` are O(1) and exact in both forms; a lone surrogate
lives in the wide form; UTF-8 output turns lone surrogates into U+FFFD as
.NET does.

Rejected: utf16 everywhere (twice the memory for ASCII, and a conversion on
every input and output of ASCII text); wtf8 (O(n) indexing; offsets have to
be translated between bytes and code units; concatenation must merge
surrogate halves); a rope. The `+=` measurement shows the quadratic cost a
rope would defer, but a rope is a builder, not a `System.String`: `.Length`,
indexing and `Substring` have to stay O(1) and exact, so a rope needs a
per-node length tree and a flattening policy, and every boundary that
expects a flat string pays to flatten it. A `StringBuilder` is the type that
should own that optimization; the immutable value stays compact. #36 and
#149 can revisit if profiles show concatenation dominance.

Latin-1 as the narrow set would keep `é` and `ü` narrow, but makes UTF-8
output a transcoding; ASCII stays unless the corpus (#18) shows Latin-1 text
to be common.

## Value layout

`Value` is 16 bytes: a tag and an 8-byte payload.

* **Inline**: `$null`, AutomationNull (its own tag, so that `@(& {})` stays
  empty), `[bool]`, `[char]`, every integer type up to 64 bits, `[single]`,
  `[double]`, and the immutable 8-byte structs `[datetime]` (ticks and
  kind) and `[timespan]`.
* **One pointer** to a heap object: strings, `[decimal]` and `[guid]` (16
  bytes each), `[bigint]`, arrays, hashtables, `PSObject`, script blocks,
  CLR handles.

Measured: summing 1,000,000 `Int32` values costs 847 µs in a 16-byte
`Value` and 1.21 ms in a 24-byte one, 1.42 times as much, from cache
pressure alone. The skeleton's current `Value` (`Int32` or an owned
`String`) is 24 bytes.

The pointer must be thin. `Rc<[u16]>` is a fat pointer, 16 bytes on its own,
which would make `Value` 24 bytes; a pointer to a string object that owns a
separate buffer costs a second allocation and a second indirection. So the
heap string is one allocation holding its header (length, form) and its
data. That needs either a small audited `unsafe` module in `rpsh-runtime` or
a vetted crate that provides thin dynamically-sized allocations; the choice
is made in #35 under the dependency rules of #7. The rest of the workspace
keeps `unsafe_code = "deny"`.

## Prior art

* **Nushell 0.117.0 (`a481925`)** — `Value` is a 17-variant enum, every variant
  carries a `Span` (`crates/nu-protocol/src/value/mod.rs:79-222`), strings are
  owned `String`, and records, lists and binary hold `SharedCow` over
  `Arc` (`crates/nu-protocol/src/value/record.rs:18-20`;
  `crates/nu-utils/src/shared_cow.rs:10-12`). `str length` and `str substring`
  default to UTF-8 bytes (`crates/nu-command/src/strings/str_/length.rs:36-50`;
  `crates/nu-command/src/strings/str_/substring.rs:183-210`). `Value` has a
  compile-time upper bound, `size_of::<Value>() <= 56`
  (`crates/nu-protocol/src/value/mod.rs:383-386`), not an exact size.
* **fish 4.9.3 (`6cf3137`)** — `WString` is `widestring`'s `Utf32String`, so
  strings are UTF-32 code points (`crates/widestring/src/lib.rs:15`).
  Undecodable bytes map one per byte into a private-use range and back, a
  reversible round trip (`crates/widestring/src/lib.rs:36-50, 133, 219`).
* **brush 0.4.0 (`96a26d0`)** and **yash-rs 3.6.0 (`46d2f79`)** — Rust
  `String` throughout (`brush-core/src/variables.rs:597-605`;
  `yash-env/src/variable/value.rs:23-27`). For non-UTF-8 command substitution,
  yash substitutes U+FFFD (`yash-semantics/src/expansion/initial/command_subst.rs:175-176`)
  and brush errors, an inference from `read_to_string`
  (`brush-core/src/commands.rs:779`), not a behaviour read in brush source.
* **ion master (`e9b3ca4`)** — `small::String`
  (`members/types-rs/src/types.rs:12`); script input panics on invalid UTF-8
  (`src/lib/parser/terminator.rs:130`).

None transfers. PowerShell's `'😀'.Length` is 2 because length counts UTF-16
code units; UTF-8 bytes (Nushell) would count 4 and UTF-32 code points (fish)
1, and indexing would follow those units. The surveyed encodings are therefore
evidence for the compact ASCII/UTF-16 string, not an argument against it.
rpsh borrows the compile-time size-assertion habit, with a 16-byte budget
rather than Nushell's 56, and rejects every surveyed representation: UTF-8
bytes, UTF-32 and plain owned `String`, each of which gives up `.Length` or
the lone surrogate.

## PSObject

Upstream wraps lazily. `PSObject.AsPSObject` (`engine/MshObject.cs:1027`)
creates a wrapper on demand, and members added to an object that was never
wrapped are kept in a `ConditionalWeakTable` keyed by the object itself
(`s_instanceMembersResurrectionTable`, `MshObject.cs:928`), so they stay
attached however often the object is wrapped and unwrapped.

rpsh keeps values unwrapped too. Every heap object's header has room for a
pointer to its instance members, null until `Add-Member` or a type name is
added: the same semantics as upstream's table without a global weak map. An
inline value that gets members (`5 | Add-Member … -PassThru`) is boxed into
a `PSObject` heap object, as upstream returns a `PSObject` there.
`[pscustomobject]` is a heap object with ordered properties.

## Value types, `[ref]`

Every value type the native shim provides — `DateTime`, `TimeSpan`,
`Guid`, `Decimal`, `KeyValuePair`, the integers — is immutable in .NET, so
none of them needs copy-on-assignment. Mutable structs exist only behind the
CLR bridge, which applies upstream's rule when such a value is assigned
(#126). `[ref]` to a variable refers to the variable's cell, so variables
are cells in their scope (#56).

## What the implementation issues take from this

| Issue | Takes |
|---|---|
| #35 | 16-byte `Value` with a size assertion; the tag list above; the compact string; one-allocation heap objects with a members pointer |
| #36 | two comparisons: ordinal-ignore-case for keys, ICU for operators |
| #43 | hashtable keys hashed over folded UTF-16 code units, with an ASCII fast path |
| #49 | shim value types are immutable values |
| #126 | the mutable-struct copy rule at the bridge |
| #149 | revisit: small strings inline, Latin-1 narrow form |
