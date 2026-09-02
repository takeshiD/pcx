# Development workflow

This document is the source of truth for branches, worktrees, and pull requests in this repository.

## One issue, one branch, one pull request

Every change must have a GitHub issue before implementation begins. Use one dedicated branch and one pull request for that issue.

- Keep unrelated changes in separate issues, branches, and pull requests.
- Never reuse a merged or closed branch for another issue.
- Put `Closes #<issue-number>` in the pull request body. Each pull request closes exactly one issue.

## Branch names

Use this format:

```text
<category>-<issue-number>-<short-description>
```

Allowed categories are:

| Category | Use for |
| --- | --- |
| `add` | New features or user-visible capabilities |
| `docs` | Documentation-only changes |
| `fix` | Bug fixes and regressions |
| `cicd` | CI/CD workflows, release automation, and delivery infrastructure |
| `chore` | Maintenance, dependencies, refactoring, and other changes |

Write the description in lowercase kebab-case using ASCII letters and numbers. Keep it short and specific. When a change spans categories, choose the category that describes its primary purpose.

Examples:

```text
add-123-mcap-info
docs-124-cli-examples
fix-125-empty-topic-list
cicd-126-release-checks
chore-127-update-dependencies
```

Branch names must match this regular expression:

```text
^(add|docs|fix|cicd|chore)-[0-9]+-[a-z0-9]+(?:-[a-z0-9]+)*$
```

## Worktrees

Keep the primary checkout on `main` for synchronization and inspection. Make every tracked-file change in a dedicated worktree, including documentation and CI/CD changes.

Create worktrees under `.agent/worktrees/`. The worktree directory name must exactly match its branch name:

```text
.agent/worktrees/<branch-name>
```

Start each branch from the latest `origin/main`. For example, for issue 123:

```bash
git fetch origin main
git worktree add .agent/worktrees/add-123-mcap-info \
  --no-track -b add-123-mcap-info origin/main
cd .agent/worktrees/add-123-mcap-info
git branch --show-current
```

Before editing, confirm that the current branch and worktree names match. Keep generated files and build artifacts inside that worktree so concurrent tasks cannot interfere with one another.

## Pull requests and cleanup

Push the issue branch and set its same-named remote branch as upstream:

```bash
git push --set-upstream origin add-123-mcap-info
```

Open the pull request from the issue branch into `main`. Keep it limited to the linked issue and include the information required by [CONTRIBUTING.md](../CONTRIBUTING.md). Run the relevant required checks before requesting review.

After the pull request is merged, return to the primary checkout, update `main`, and remove the completed worktree and local branch:

```bash
git switch main
git pull --ff-only origin main
git worktree remove .agent/worktrees/add-123-mcap-info
git branch -d add-123-mcap-info
```
