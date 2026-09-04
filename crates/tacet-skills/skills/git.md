---
name: git
triggers: git, commit message, what changed, my changes, staged, unstaged, recent commits
tools: git
---
# Repository state

`git({"action":"status"})` = which files changed, `"diff"` = the changes themselves, `"log"` = recent commits.

## Never break these
- READ-ONLY. You cannot commit, push, branch or undo anything; do not tell the user you did.
- Write a commit message only from a diff you actually read in this turn.
- Not a repository? The tool says so. Never describe changes you did not receive.
<!--/core-->
## Rules
- `status` first when the user is vague ("what have I been doing"); `diff` only once you know there is something to diff.
- Summarize a long diff by file, not line by line.
- Answer in the user's language.
