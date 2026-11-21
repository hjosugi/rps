# Script corpus

The first corpus is the upstream PowerShell test suite at the latest stable release (7.7.0), fetched by `scripts/fetch-corpus.sh` and not committed. `scripts/analyze-corpus.py` parses every file with the reference pwsh and runs rpsh on it.

* files: **432**
* files with parse errors: **1**
* rpsh ran the script: **0**
* rpsh refused it by name: **432**
* rpsh timed out (2 s): **0**
* rpsh exited with another failure: **0**

A refusal is the first unsupported construct in the file, so each count is a lower bound. Files that contain several missing features contribute only to the first one.

## Most frequent refusals

| feature | count |
|---|---:|
| `command invocation` | 347 |
| `statement keywords (if, foreach, function, ...)` | 36 |
| `variables` | 19 |
| `type literals, casts and indexing` | 13 |
| `member access, dot-sourcing and real literals` | 9 |
| `block comments` | 8 |

## Most frequent commands

| command | count |
|---|---:|
| `Should` | 14380 |
| `It` | 6884 |
| `Join-Path` | 1090 |
| `Describe` | 909 |
| `BeforeAll` | 828 |
| `Context` | 797 |
| `New-Item` | 678 |
| `Test-Path` | 414 |
| `Remove-Item` | 411 |
| `AfterAll` | 390 |
| `ConvertTo-Json` | 345 |
| `Get-Content` | 337 |
| `ForEach-Object` | 326 |
| `Get-WebListenerUrl` | 310 |
| `TabExpansion2` | 293 |
| `ShouldBeParseError` | 289 |
| `Get-ChildItem` | 274 |
| `Get-Item` | 273 |
| `Import-Module` | 269 |
| `Where-Object` | 262 |

## Most frequent types (type literals)

| type | count |
|---|---:|
| `AstTypeInference` | 204 |
| `powershell` | 143 |
| `System.IO.Path` | 132 |
| `int` | 99 |
| `scriptblock` | 85 |
| `M` | 82 |
| `String` | 75 |
| `IO.Path` | 72 |
| `DateTime` | 66 |
| `Environment` | 60 |
| `System.Environment` | 57 |
| `System.Management.Automation.Internal.InternalTestHooks` | 52 |
| `double` | 50 |
| `Guid` | 47 |
| `long` | 43 |
| `System.Management.Automation.Platform` | 42 |
| `System.Management.Automation.Language.Parser` | 41 |
| `regex` | 40 |
| `SemanticVersion` | 38 |
| `RunspaceFactory` | 37 |

## Most frequent members

| member | count |
|---|---:|
| `Count` | 1638 |
| `Name` | 1227 |
| `new` | 701 |
| `CompletionMatches` | 490 |
| `Length` | 489 |
| `Streams` | 455 |
| `Invoke` | 442 |
| `Error` | 373 |
| `Exception` | 345 |
| `CompletionText` | 329 |
| `AddScript` | 288 |
| `ToString` | 275 |
| `Value` | 260 |
| `Create` | 253 |
| `Message` | 236 |
| `FullyQualifiedErrorId` | 230 |
| `FullName` | 218 |
| `Output` | 217 |
| `Ast` | 205 |
| `InferTypeOf` | 204 |

## Most frequent operators

| operator | count |
|---|---:|
| `Or` | 855 |
| `Plus` | 605 |
| `Ieq` | 535 |
| `Join` | 304 |
| `Ine` | 293 |
| `DotDot` | 276 |
| `And` | 186 |
| `Minus` | 112 |
| `Format` | 106 |
| `Igt` | 101 |
| `Icontains` | 100 |
| `Is` | 99 |
| `Multiply` | 90 |
| `Ilt` | 84 |
| `Ireplace` | 72 |
| `Ilike` | 72 |
| `Isplit` | 69 |
| `Imatch` | 50 |
| `QuestionQuestion` | 45 |
| `As` | 39 |

## Most frequent AST nodes

| node | count |
|---|---:|
| `StringConstantExpressionAst` | 123279 |
| `PipelineAst` | 67399 |
| `VariableExpressionAst` | 64845 |
| `CommandExpressionAst` | 50820 |
| `CommandAst` | 43151 |
| `CommandParameterAst` | 40298 |
| `MemberExpressionAst` | 16578 |
| `AssignmentStatementAst` | 15969 |
| `NamedBlockAst` | 14924 |
| `ScriptBlockAst` | 14847 |
| `ScriptBlockExpressionAst` | 13506 |
| `ConstantExpressionAst` | 12599 |
| `StatementBlockAst` | 7200 |
| `InvokeMemberExpressionAst` | 6049 |
| `HashtableAst` | 5455 |
| `BinaryExpressionAst` | 4281 |
| `TypeExpressionAst` | 3801 |
| `IndexExpressionAst` | 3514 |
| `ParenExpressionAst` | 3351 |
| `ParameterAst` | 3220 |

## Milestone alignment

The refusal list is dominated by command invocation because the corpus is Pester (Should, It, Describe, Mock). The parser gaps (statement keywords, variables, type literals) come next. In the milestone order this is still M1 for the parser, then M3/M5 for command invocation and the cmdlet surface; no milestone is reordered by this table.

The corpus is upstream tests, not PowerShell Gallery modules yet. Adding a pinned set of public modules is a follow-up; the fetch and analysis scripts do not change when it lands.
