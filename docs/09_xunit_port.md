# Upstream xUnit port

Status of the engine-internal tests under `test/xUnit/csharp`. A test is
ported when the API it pins exists; until then its file stays here with the
crate and milestone that will enable it. No test is dropped. Relates to #17.

`test_CommandLineParser.cs` is the first file: its 40 test cases live in
`crates/rpsh-host/tests/command_line_parser.rs`, pinned to rpsh's smaller host
API. Cases for switches the host does not implement assert the explicit
`NotImplemented` refusal (DESIGN.md §2), and the table records the rest.

| upstream file | target crate | status | issue / milestone |
|---|---|---|---|
| `test_CommandLineParser.cs` | `rpsh-host` | **ported** (40 tests) | #17 |
| `test_AstDefaultVisit.cs` | `rpsh-syntax` | pending | #33, #34 |
| `test_Binders.cs` | `rpsh-engine` | pending | M3 |
| `test_CompletionHelpers.cs` | `rpsh-commands` | pending | #168 |
| `test_CorePsPlatform.cs` | `rpsh-native` | pending | M4 |
| `test_CryptoUtils.cs` | `rpsh-commands` | pending | #132 |
| `test_ExtensionMethods.cs` | `rpsh-runtime` | pending | #36, #38 |
| `test_Feedback.cs` | `rpsh-host` | pending | M6 |
| `test_FileSystemProvider.cs` | `rpsh-providers` | pending | M5 |
| `test_MshSnapinInfo.cs` | `rpsh-host` | pending | M2 |
| `test_NamedPipe.cs` | `rpsh-native` | pending | #76 |
| `test_NativeInterop.cs` | `rpsh-native` | pending | M4 |
| `test_PowerShellAPI.cs` | `rpsh-engine` | pending | M3 |
| `test_Prediction.cs` | `rpsh-commands` | pending | #168 |
| `test_ProgressNode.cs` | `rpsh-format` | pending | M6 |
| `TestProgressRawUI.cs` | `rpsh-host` | pending | M6 |
| `test_PSConfiguration.cs` | `rpsh-host` | pending | M2 |
| `test_PSObject.cs` | `rpsh-runtime` | pending | #35 |
| `test_PSVersionInfo.cs` | `rpsh-runtime` | pending | M2 |
| `test_RemoteHyperV.cs` | remoting | not Linux | M7 |
| `test_Runspace.cs` | `rpsh-engine` | pending | M3 |
| `test_SecuritySupport.cs` | `rpsh-commands` | pending | #132 |
| `test_Serialization.cs` | `rpsh-commands` | pending | M5 |
| `test_SessionState.cs` | `rpsh-engine` | pending | M3 |
| `test_Subsystem.cs` | `rpsh-native` | pending | M4 |
| `test_Utils.cs` | `rpsh-runtime` | pending | M2 |
| `test_WildcardPattern.cs` | `rpsh-runtime` | pending | M2 |

A ported file is enabled at once: its tests run in `cargo test --workspace`.
A pending file is enabled by the issue that lands its API; the upstream
`[Fact]`/`[Theory]` data is copied with the same inputs, not paraphrased.
