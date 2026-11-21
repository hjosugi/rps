# What pwsh has done by the time it runs the first statement of a script.
# Prints name=value lines for scripts/time-commands.py ("metrics": true).
$process = [System.Diagnostics.Process]::GetCurrentProcess()
"jit_methods=$([System.Runtime.JitInfo]::GetCompiledMethodCount($false))"
"jit_ms=$([System.Runtime.JitInfo]::GetCompilationTime($false).TotalMilliseconds)"
"jit_il_bytes=$([System.Runtime.JitInfo]::GetCompiledILBytes($false))"
"assemblies=$([AppDomain]::CurrentDomain.GetAssemblies().Count)"
"allocated_kib=$([GC]::GetTotalAllocatedBytes($false) / 1KB)"
"gen0_collections=$([GC]::CollectionCount(0))"
"working_set_kib=$($process.WorkingSet64 / 1KB)"
"threads=$($process.Threads.Count)"
