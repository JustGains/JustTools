# JustCommit

JustCommit is a one-shot commit helper. It verifies the requested working
directory, resolves the Git repository root, stages the complete working tree,
requires an OpenRouter key, builds a bounded digest of the resulting index, asks
a fast inexpensive model for a summary and commit message, and creates the
commit without opening an editor.

## Quick start

Set the key in the environment so it does not need to appear in shell history:

```powershell
$env:OPENROUTER_API_KEY = "your-key"
justcommit
```

```sh
export OPENROUTER_API_KEY="your-key"
justcommit
```

The key may instead be supplied with `--api-key`. JustCommit does not print or
store it, include it in a repair brief, or write it to the repository.

Useful modes:

```sh
justcommit --dry-run
justcommit --push
justcommit --staged
justcommit -C ../another-repository
justcommit --model google/gemini-3.1-flash-lite
justcommit --no-patches
```

By default, JustCommit runs `git add --all` before analysis. `--all` remains an
accepted compatibility flag, while `--staged` preserves and uses only the
existing staged selection. Default dry runs also stage the complete working
tree; use `justcommit --staged --dry-run` when the index must remain unchanged.
`--dry-run` prevents both the commit and push.

With `--push`, JustCommit runs a plain `git push` only after the commit succeeds.
It uses the repository's normal remote, branch, and upstream configuration. If
the push fails, the error identifies the commit that remains safely available
locally.

## Fast by construction

JustCommit never captures or uploads the full diff. Its large-tree path is
bounded in every place where repository size could otherwise become model
input or process memory:

- Git emits `--name-status -z --no-renames`, which JustCommit consumes as a
  stream while counting every staged path.
- Directory/extension maps and representative path sets have fixed limits.
- At most 64 representative paths are retained.
- At most 12 text patches are sampled, in parallel, and each patch is capped at
  6 KiB. Git is stopped as soon as a sample reaches its cap or takes 750 ms.
- Commit instructions are capped at 48 KiB and model responses at 512 KiB.
- Binary, generated, build, dependency, credential, environment, certificate,
  and likely-secret paths are excluded from patch sampling. `--no-patches`
  sends only bounded names and counts.
- Rename detection, external diff drivers, and text-conversion drivers are
  disabled for the scan. Renames may therefore appear as an add/delete pair,
  which avoids the expensive similarity pass on enormous changes.

The complete staged index is still represented by exact file/status counts;
only the evidence sent to the model is sampled. Immediately before committing,
JustCommit compares `git write-tree` results and aborts if another process or
the user changed the index while OpenRouter was responding.

## Model choice

The default is `google/gemini-2.5-flash-lite:nitro`. At the time of selection,
[OpenRouter listed Gemini 2.5 Flash Lite](https://openrouter.ai/google/gemini-2.5-flash-lite/pricing)
as an ultra-low-latency, cost-efficient model with a 1M-token context and
substantially lower input/output prices than newer Flash Lite generations. The
[:nitro routing suffix](https://openrouter.ai/docs/faq#what-are-model-variants)
asks OpenRouter to prefer the highest-throughput provider.

The prompt is intentionally much smaller than the available context window.
Override the model normally when a repository needs more reasoning quality:

```sh
justcommit --model anthropic/claude-haiku-4.5
justcommit --model google/gemini-3.5-flash-lite
```

## Repository commit rules

JustCommit checks these files at the repository root in order:

1. `.cursor/rules/git-commit-structure.mdc`
2. `.gitmessage`

The first existing file wins. Without either, the model is asked for a concise
conventional-style imperative subject and a short explanatory body when useful.
Repository filenames and sampled patches are explicitly treated as untrusted
data rather than model instructions.

## Errors and agent repair

Normal failures include a self-contained `JUSTCOMMIT REPAIR BRIEF`. It identifies
the repository and diagnostic, tells an agent to preserve unrelated changes,
forbids credential handling and committing, and distinguishes source failures
from external credential, billing, network, and provider failures.

Use `--repair` to send that brief directly to the first installed supported
agent (Codex, then Claude):

```sh
justcommit --repair
justcommit --repair --repair-with codex
justcommit --repair --repair-with claude
```

Codex runs as `codex exec -C <repository> -`; Claude runs as `claude -p` in the
repository. The repair agent never receives the OpenRouter key and is told not
to alter the staged selection or create a commit. It reports any repaired files
that should be reviewed and staged before JustCommit is rerun.

For manual piping, the ordinary error output already contains the complete
repair brief:

```sh
justcommit 2>&1 | codex exec -C . -
justcommit 2>&1 | claude -p
```
