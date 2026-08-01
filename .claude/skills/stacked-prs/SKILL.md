---
name: stacked-prs
description: Use when opening a PR on Vesper or building a stack of dependent PRs with `gh stack` — gives the exact commands, the flags that hang or pick the wrong base, and the publish order that keeps CI to one PR at a time.
---

# Stacked PRs on Vesper

A stack is an ordered chain of PRs, each based on the one below. Use it for 3+ layers with a real
dependency (schema → service → API → UI). Genuinely independent work stays as parallel PRs.

## Setup

Needs `gh` >= 2.97.0 and the official extension:

```bash
winget upgrade --id GitHub.cli --exact --silent   # Windows; use brew/apt elsewhere
gh extension install github/gh-stack
```

Stacked PRs are in public preview (since 2026-07-30). If a command reports the repo is not
enrolled, fall back to plain `--base <branch-below>` PRs — same stack, no stack map in the UI.

## Traps

**`init` defaults the trunk to the repository default branch, which is `main` here.** `main` is the
release branch. Always pass `--base dev`, or the whole stack targets release.

```bash
gh stack init --base dev <bottom-branch>
```

**`gh stack submit` opens a fullscreen interactive editor** and waits for `Ctrl+S`. Pass `--auto` so
it never blocks.

**`--auto` creates PRs as drafts.** Add `--open` to mark them ready for review, otherwise nothing is
reviewable and nothing merges.

**`submit` publishes every branch in the stack at once.** Per-layer selection exists only in the
interactive editor, so `--auto` fires CI on all layers simultaneously. Checking out a lower layer
does **not** scope it: running `gh stack submit` from the bottom branch still publishes every branch
above it. The only way to publish one layer at a time is for the upper layers not to exist yet.
Build the stack incrementally — each `submit` creates PRs only for branches that don't have one, and
re-points the bases of the ones that do:

```bash
gh stack init --base dev layer-1     # commit layer 1 first
gh stack submit --auto --open        # 1 PR, 1 CI run
gh stack add layer-2                 # commit layer 2
gh stack submit --auto --open        # adds the 2nd PR, keeps the 1st
```

**`gh stack merge` in a non-interactive terminal uses your last-used merge method.** That is whatever
happened to run before, which may not be squash. Always state it:

```bash
gh stack merge --yes --squash
```

Merge is atomic: every PR up to the chosen one goes in, or none do. Branch protection is evaluated by
GitHub at merge time — bypassing merge requirements is not supported for stacks.

## Order of work

1. `git fetch origin`, then cut the worktree from `origin/dev` (never from a stale local branch)
2. Implement and commit the bottom layer
3. Review the local commit **before pushing** — review comes before CI, not after
4. `gh stack submit --auto --open` — publishes every branch that exists, which at this point is
   only the layer you just committed
5. `gh stack add <next>` for the layer above, and repeat from step 2
6. When the stack is green and approved, `gh stack merge --yes --squash`

## Worktrees

`.claude/worktrees/` is gitignored, so worktrees live there without polluting status:

```bash
git worktree add -b <branch> .claude/worktrees/<name> origin/dev
```

Two commands fight worktrees, because `dev` is already checked out in the main clone:

- **`gh pr merge --delete-branch`** wants to check out the base branch locally and fails with
  `fatal: 'dev' is already used by worktree at ...`. The merge itself still goes through — only the
  cleanup dies. Delete the branch explicitly afterwards with `git push origin --delete <branch>`.
- **`gh stack sync` fast-forwards the trunk branch**, which means touching the main clone's working
  tree. Don't. Rebase onto the remote-tracking ref from inside your own worktree instead, then push:

```bash
git fetch origin
git checkout <bottom> && git rebase origin/dev
git checkout <top>    && git rebase <bottom>
git push --force-with-lease --atomic origin <bottom> <top>
```

`git worktree remove` fails with `Permission denied` if your shell sits inside the worktree. `cd`
out first, then `git worktree prune` and confirm with `git worktree list`.

## Layer discipline

A change that belongs to a lower layer must be committed there and propagated up. Never patch it at
the top of the stack. `gh stack modify` restructures a stack but is interactive — avoid in automation.
