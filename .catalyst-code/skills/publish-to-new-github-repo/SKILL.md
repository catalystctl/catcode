---
name: publish-to-new-github-repo
description: Commit all working-tree changes (if any), create a private GitHub repo under the user's org or account, add the remote, push, and verify — the create-and-publish counterpart of git-commit-all
---

# publish-to-new-github-repo

When the user says "commit all of git status then create a repo private repo under my <org> called <name> then push it there" (or any close variant), commit everything, create a fresh private GitHub repo, and push the current branch to it.

## When to use

- The user wants to publish the current repo to a brand-new GitHub repo (first push, no remote yet).
- Specifically: commit all → create private repo under an org or account → push.

Do NOT use for:
- Pushing to a repo that already has a remote (just `git push`).
- Updating an existing repo's contents (no `gh repo create`).
- Forking or importing (different `gh` subcommands).

## Prerequisites

- `gh` CLI installed and authenticated (`gh auth status`). Needs scopes `repo` + `workflow` (to push workflow files under `.github/`).
- For an **org** repo: the user must be a member with repo-creation rights (`gh api orgs/<org>/memberships/<user> --jq .role` → `admin` or `member` with the org allowing member repo creation). Verify before creating.

## Steps

1. **Commit all working-tree changes** (reuse the `git-commit-all` skill's logic):
   - `git status --short` + `git diff --stat` to confirm what's going in.
   - `git add --all` (modified + new + deleted).
   - One focused commit per logical change is cleaner, but if the user said "commit all," a single well-described commit is fine. Multiple coherent changes → multiple commits grouped by scope.
   - `git commit -m "<message>"`.
2. **Confirm the tree is clean** before publishing: `git status --short` (must be empty).
3. **Check the repo doesn't already exist** (avoid a confusing error):
   `gh repo view <org>/<name> --json name 2>&1` — if it resolves, STOP and ask the user (don't overwrite/push to an existing repo without confirmation).
4. **(Org repo only) verify org access**: `gh api orgs/<org>/memberships/<user> --jq '.role + " " + .state'` → expect `admin active` or `member active`.
5. **Create the private repo + add remote + push in one shot**:
   ```
   gh repo create <org>/<name> --private --source=. --remote=origin --push
   ```
   - `--source=.` uses the current dir as the repo source.
   - `--remote=origin` adds `origin` → `https://github.com/<org>/<name>.git`.
   - `--push` pushes the current branch and sets up tracking.
   - For a **user** repo (not org), drop the `<org>/` prefix: `gh repo create <name> --private --source=. --remote=origin --push`.
   - For a **public** repo, use `--public` instead of `--private`.
6. **Verify**:
   - `git remote -v` → origin points at the new repo.
   - `git log --oneline origin/<branch> -3` → the pushed commits.
   - `gh repo view <org>/<name> --json visibility,url,defaultBranchRef --jq '...'` → `visibility=PRIVATE`, correct default branch.

## Notes / gotchas

- **Default branch**: GitHub sets the default branch to whatever you push first (here `master`). If the project convention is `main`, either rename first (`git branch -m master main`) before pushing, or rename after on GitHub + locally. CI workflows that trigger on `push: branches: [main, master]` cover both.
- **`--push` pushes only the current branch.** Other local branches are NOT pushed; push them separately if needed (`git push origin <branch>`).
- **Workflow files** (`.github/workflows/*`) require the `workflow` scope on the gh token — without it the push is rejected with "refusing to allow an OAuth App to create or update workflow". Re-auth with `gh auth refresh -h github.com -s workflow` if so.
- **Large repos / LFS**: `gh repo create --push` does a normal `git push`; binaries tracked normally go up fine, but >100MB files need Git LFS configured first.
- **Pre-existing local `origin`**: if `origin` already exists (e.g. from a prior aborted attempt), `--remote=origin` errors. Remove it first: `git remote remove origin`.

## Example

User: "Commit all of git status then create a repo private repo under my catalystctl org called catcode then push it there."

```
# 1. commit all (grouped into focused commits)
git add --all
git commit -m "core: <change>"
# ...more commits as needed
git status --short          # confirm clean

# 2. pre-flight
gh repo view catalystctl/catcode --json name 2>&1 | grep -q 'Could not resolve' && echo "free to create"
gh api orgs/catalystctl/memberships/karutoil --jq '.role + " " + .state'   # admin active

# 3. create + push
gh repo create catalystctl/catcode --private --source=. --remote=origin --push
# -> https://github.com/catalystctl/catcode  (origin/master set up to track)
```
