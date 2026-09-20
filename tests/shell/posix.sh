set -eu
start=$PWD
export JUSTTOOLS_MKCD_RESULT=previous-value
mkcd 'my folder [x] café'
test "${PWD##*/}" = 'my folder [x] café'
test "$JUSTTOOLS_MKCD_RESULT" = previous-value
inside=$PWD
mkcd --help >/dev/null
mkcd --version >/dev/null
just --version >/dev/null
mkcd --print-path printed >/dev/null
test "$PWD" = "$inside"
if mkcd printed; then exit 1; fi
test "$PWD" = "$inside"
if mkcd first second; then exit 1; fi
test ! -e first
just mkcd -p parent/child
test "${PWD##*/}" = child
justmkcd -- --literal
test "${PWD##*/}" = --literal
mkcd -p .
test "${PWD##*/}" = --literal
cd "$start"
mkdir stubs
cat > stubs/claude <<'STUB'
#!/bin/sh
printf '<%s>\n' "$@"
exit 19
STUB
cp stubs/claude stubs/codex
chmod +x stubs/claude stubs/codex
export PATH="$start/stubs:$PATH"
if claude_ --resume 'two words' '$(literal)' '' > claude-args; then exit 1; else test "$?" = 19; fi
if codex_ resume 'two words' '$(literal)' '' > codex-args; then exit 1; else test "$?" = 19; fi
printf '%s\n' '<--dangerously-skip-permissions>' '<--resume>' '<two words>' '<$(literal)>' '<>' > expected-claude
printf '%s\n' '<--yolo>' '<resume>' '<two words>' '<$(literal)>' '<>' > expected-codex
cmp claude-args expected-claude
cmp codex-args expected-codex
