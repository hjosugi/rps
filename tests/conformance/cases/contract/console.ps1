# docs/01_compat_contract.md, "Reference environment". With output
# redirected and no TERM, pwsh's console is 80 columns wide; default
# formatting and error views wrap at that width.
$size = $Host.UI.RawUI.WindowSize
"window=$($size.Width)x$($size.Height)"
"redirected=$([Console]::IsOutputRedirected) console-width=$([Console]::WindowWidth)"
"rendering=$($PSStyle.OutputRendering) error-view=$ErrorView"
"encoding=$([Console]::OutputEncoding.WebName) output-encoding=$($OutputEncoding.WebName)"
