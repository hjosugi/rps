# rpsh-conformance: level=c2
enum Color { Red; Green; Blue }
[Color]::Green
[Color]::Green -eq [Color]::Green
[Color]::Blue -eq [Color]::Red
([Color]::Red).GetType().Name
enum Level { Low = 5; High }
[int][Level]::High
[int][Color]::Blue
