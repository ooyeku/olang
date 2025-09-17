## IssueMap Agent Guide

Audience: AI coding agents automating typical dev workflows in a Git repo.

Key facts
- IssueMap is Git-native. Issues are YAML files under .issuemap/issues.
- Run all commands from the Git repo root. Ensure a clean worktree for merges.
- Use non-interactive flags. Avoid prompts. Do not use emojis.

Initialize (once per repo)

    git init -b main    # if repo not initialized
    issuemap init --name "<Project Name>"

Daily golden path
1) Create an issue

    issuemap create "<Title>" --type feature --priority medium --labels a,b
    issuemap list --status open

2) Branch for the issue (one branch per issue)

    issuemap branch ISSUE-XXX

3) Commit work and reference the issue ID

    git add -A
    git commit -m "ISSUE-XXX: short summary"

4) Optional time tracking

    issuemap start ISSUE-XXX
    # ...work...
    issuemap stop ISSUE-XXX

5) Keep in sync (derive status, links)

    issuemap sync --auto-update
    issuemap show ISSUE-XXX

6) Merge and close

    # From the feature branch
    issuemap merge
    # Or from main
    issuemap merge ISSUE-XXX

7) Housekeeping

    git branch -d <feature/ISSUE-XXX-short-title>
    issuemap list --status open

Common operations (non-interactive)
- Create with template:

    issuemap create "Hotfix: CSRF token mismatch" --template hotfix

- Edit fields:

    issuemap edit ISSUE-XXX --status in-progress --assignee alice --labels auth,backend

- Dependencies:

    issuemap depend ISSUE-B --on ISSUE-A
    issuemap deps ISSUE-B --graph

- Search (query DSL):

    issuemap search "type:bug AND priority:high AND updated:<7d"

- Bulk update (query-driven):

    issuemap bulk --query "label:frontend AND status:open" --set status=review

Conventions
- IDs: either ISSUE-003 or 003 (both accepted). Prefer the full form in commits.
- Commits: prefix messages with the ID: ISSUE-003: message.
- Branch names: created by the branch command and recorded on the issue.

Server (optional)
- A local server provides a REST API; not required for CLI flows.

    issuemap server start
    issuemap server status

Safety & etiquette for agents
- Always run from repo root; verify with: git rev-parse --show-toplevel
- Prefer explicit flags; avoid interactive prompts.
- After creation/edits, verify with: issuemap show ISSUE-XXX
- Before merge, ensure no unstaged changes; commit .issuemap updates if needed:

    git add .issuemap && git commit -m "Update issues"

Quick reference

    issuemap create "Title" --type feature --priority medium
    issuemap branch ISSUE-123
    git commit -m "ISSUE-123: change"
    issuemap sync --auto-update
    issuemap merge
