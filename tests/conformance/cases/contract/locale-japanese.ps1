# rpsh-conformance: level=c2 LANG=ja_JP.UTF-8
"$env:LANG|$env:LC_ALL"
('カ','か','ｶ','a','Ａ','A' | Sort-Object -Unique -Stable) -join '|'
('カ','か','ｶ','a','Ａ','A' | Sort-Object -CaseSensitive -Stable) -join '|'
'か' -eq 'カ'
"$PSCulture|$PSUICulture|$((Get-Culture).LCID)"
[CultureInfo]::CurrentCulture.DateTimeFormat.ShortTimePattern
