// Stage 1: the runtime starts and runs an empty Main. Same apphost, same
// runtimeconfig and same assemblies on the TPA list as pwsh.
public static class Probe
{
    public static int Main() => 0;
}
