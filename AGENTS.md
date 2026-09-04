# Repository instructions

This file is the source of truth for repository-wide instructions followed by coding agents.

## Agent configuration

- Put tool-neutral skills, prompts, and other reusable agent assets under `.agents/`.
- Expose a shared asset in `.codex/` or `.claude/` with a relative symbolic link instead of copying it.
- Keep only genuinely tool-specific configuration in `.codex/` and `.claude/`.
- Keep repository-wide conduct and workflow rules in this `AGENTS.md`.
- Keep the root `CLAUDE.md` as a symbolic link to `AGENTS.md` so Codex and Claude Code receive the same rules.
- Resolve every repository-owned symbolic link within the repository, and verify that it remains valid from a fresh checkout.

## Architecture guardrails

Before planning or modifying product code, formats, CLI behavior, build configuration, or release automation, read and follow [`.agents/rules/architecture.md`](.agents/rules/architecture.md). During review, verify every applicable guardrail against its linked ADR. If a proposed change conflicts with an accepted ADR, pause implementation and propose a superseding ADR first.

## Change workflow

Every change starts with a GitHub issue and uses one dedicated branch, one matching worktree, and one pull request. If the issue number is unavailable, stop before editing tracked files and ask for it.

### Branch names

Use `<category>-<issue-number>-<short-description>`, where the description is short lowercase ASCII kebab-case.

| Category | Use for |
| --- | --- |
| `add` | New features or user-visible capabilities |
| `docs` | Documentation-only changes |
| `fix` | Bug fixes and regressions |
| `cicd` | CI/CD workflows, release automation, and delivery infrastructure |
| `chore` | Maintenance, dependencies, refactoring, and other changes |

Examples: `add-123-mcap-info`, `docs-124-cli-examples`, `fix-125-empty-topic-list`, `cicd-126-release-checks`, and `chore-127-update-dependencies`.

Branch names must match:

```text
^(add|docs|fix|cicd|chore)-[0-9]+-[a-z0-9]+(?:-[a-z0-9]+)*$
```

### Worktrees

Keep the primary checkout on `main` for synchronization and inspection. Make every tracked-file change in `.worktrees/<branch-name>`, with a directory name exactly matching the branch name.

Start from the latest `origin/main` without making the issue branch track `main`:

```bash
git fetch origin main
git worktree add .worktrees/add-123-mcap-info \
  --no-track -b add-123-mcap-info origin/main
cd .worktrees/add-123-mcap-info
git branch --show-current
```

Before editing, confirm that the current branch and worktree names match. Keep build artifacts inside that worktree so concurrent tasks do not interfere with one another.

### Pull requests

- Keep the branch limited to its one linked issue.
- Run every relevant check in `CONTRIBUTING.md` before requesting review.
- Push the branch with `git push --set-upstream origin <branch-name>` so it tracks its same-named remote branch.
- Open the pull request into `main` and put `Closes #<issue-number>` in its body. Each pull request closes exactly one issue.
- After merge, update the primary checkout, remove the completed worktree, and delete the merged local branch.

## Engineering rules

Use the required checks and design rules in [CONTRIBUTING.md](CONTRIBUTING.md). Before an architectural change, also read [CONTEXT.md](CONTEXT.md) and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
