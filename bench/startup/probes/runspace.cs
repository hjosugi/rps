// Stage 3: stage 2 plus the engine: the session state pwsh builds
// (InitialSessionState.CreateDefault2), an opened runspace and one script
// run in it. No ConsoleHost, no profile, no PSReadLine.
using System.Management.Automation;
using System.Management.Automation.Runspaces;

public static class Probe
{
    public static int Main()
    {
        using var runspace = RunspaceFactory.CreateRunspace(InitialSessionState.CreateDefault2());
        runspace.Open();
        using var shell = PowerShell.Create();
        shell.Runspace = runspace;
        shell.AddScript("exit").Invoke();
        return 0;
    }
}
