set -l start "$PWD"
set -gx JUSTTOOLS_MKCD_RESULT previous-value
mkcd 'my folder [x] café'; or exit 1
test (path basename "$PWD") = 'my folder [x] café'; or exit 1
test "$JUSTTOOLS_MKCD_RESULT" = previous-value; or exit 1
set -l inside "$PWD"
mkcd --help >/dev/null; or exit 1
mkcd --version >/dev/null; or exit 1
just --version >/dev/null; or exit 1
mkcd --print-path printed >/dev/null; or exit 1
test "$PWD" = "$inside"; or exit 1
if mkcd printed; exit 1; end
test "$PWD" = "$inside"; or exit 1
if mkcd first second; exit 1; end
test ! -e first; or exit 1
just mkcd -p parent/child; or exit 1
test (path basename "$PWD") = child; or exit 1
justmkcd -- --literal; or exit 1
test (path basename "$PWD") = --literal; or exit 1
cd "$start"
mkdir stubs
printf '%s\n' '#!/bin/sh' 'printf "<%s>\n" "$@"' 'exit 19' > stubs/claude
cp stubs/claude stubs/codex
chmod +x stubs/claude stubs/codex
set -gx PATH "$start/stubs" $PATH
claude_ --resume 'two words' '$(literal)' '' > claude-args
test $status -eq 19; or exit 1
codex_ resume 'two words' '$(literal)' '' > codex-args
test $status -eq 19; or exit 1
printf '%s\n' '<--dangerously-skip-permissions>' '<--resume>' '<two words>' '<$(literal)>' '<>' > expected-claude
printf '%s\n' '<--yolo>' '<resume>' '<two words>' '<$(literal)>' '<>' > expected-codex
cmp claude-args expected-claude; or exit 1
cmp codex-args expected-codex; or exit 1
