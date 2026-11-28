# rpsh-conformance: level=c2
ls -d .
$LASTEXITCODE
diff /dev/null /dev/null
$LASTEXITCODE

function cp { 'local cp' }
function mv { 'local mv' }
function rm { 'local rm' }
function ac { 'local ac' }
cp
mv
rm
ac
