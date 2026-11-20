# docs/01_compat_contract.md, "Culture". Culture data comes from the
# system ICU, recorded in tests/conformance/reference.txt. Under ICU 72 and
# later, en-US puts U+202F NARROW NO-BREAK SPACE before AM/PM.
"culture=$((Get-Culture).Name) ui=$((Get-UICulture).Name)"
(1234567.891).ToString('N2')
'{0:C}' -f 12.5
"$(1.5) $((1/3)) $([double]::MaxValue)"
$moment = [datetime]::new(2026, 1, 2, 15, 4, 5, [System.DateTimeKind]::Utc)
$moment.ToString()
$moment.ToString('D')
"$moment"
[cultureinfo]::CurrentCulture.DateTimeFormat.LongTimePattern
('a', 'B', 'A', 'b', '_', '1' | Sort-Object) -join ','
('a', 'B', 'A', 'b', '_', '1' | Sort-Object -CaseSensitive) -join ','
