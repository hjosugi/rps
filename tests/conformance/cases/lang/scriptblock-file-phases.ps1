# rpsh-conformance: level=c2
param($tag = 'root')
clean { 'discard'; $script:trace += 'c' }
begin { $script:trace = 'b'; "begin=$tag/$input" }
process { $script:trace += 'p'; "process=$_/$input" }
end { "end=$trace/$input" }
