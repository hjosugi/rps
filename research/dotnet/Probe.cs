// The managed side of hostfxr-probe.c: static methods a native host calls
// through function pointers. Each one stands for a kind of work rpsh would
// hand to the CLR (docs/06_dotnet_bridge.md).
using System;
using System.Management.Automation;
using System.Runtime.InteropServices;

public static class Probe
{
    // A call that needs nothing beyond the method itself.
    [UnmanagedCallersOnly]
    public static int Add(int a, int b) => a + b;

    // A BCL member outside rpsh's native shim: file I/O in CoreLib.
    [UnmanagedCallersOnly]
    public static int ReadLength(IntPtr pathUtf8)
    {
        string path = Marshal.PtrToStringUTF8(pathUtf8) ?? string.Empty;
        return System.IO.File.ReadAllText(path).Length;
    }

    // A BCL member in an assembly that is not loaded yet.
    [UnmanagedCallersOnly]
    public static int RegexMatches()
    {
        return System.Text.RegularExpressions.Regex.Matches("a1b22c333", "[0-9]+").Count;
    }

    // What a binary module needs first: System.Management.Automation.
    [UnmanagedCallersOnly]
    public static int TouchSma()
    {
        return typeof(PSObject).Assembly.GetName().Version?.Major ?? -1;
    }

    // The upstream engine itself, in process: a runspace built the way pwsh
    // builds its own, one script run in it.
    [UnmanagedCallersOnly]
    public static int RunPowerShell()
    {
        using var shell = PowerShell.Create(System.Management.Automation.Runspaces.InitialSessionState.CreateDefault2());
        var results = shell.AddScript("1 + 1").Invoke();
        return results.Count == 1 ? (int)results[0].BaseObject : -1;
    }
}
