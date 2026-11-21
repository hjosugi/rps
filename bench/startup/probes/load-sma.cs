// Stage 2: stage 1 plus loading System.Management.Automation and touching
// one of its types.
public static class Probe
{
    public static int Main() =>
        typeof(System.Management.Automation.PSObject).Assembly.GetName().Name.Length > 0 ? 0 : 1;
}
