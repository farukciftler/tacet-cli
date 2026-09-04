---
name: shell
triggers: run the command, shell command, command line, terminal command
tools: shell
---
# Running a command

`shell({"command":"ls","args":["-la"]})` runs ONE program the user has allowed.

## Never break these
- `command` is a CLOSED list the user installed. A name outside it is refused; do not try variants.
- `args` is an array, one argument per element. Never write a shell line with pipes, `&&` or quotes in it.
- Report what the command actually printed. Never describe output you did not receive.
<!--/core-->
## Rules
- A non-zero exit is information, not a crash; say what failed.
- One command per turn.
- Answer in the user's language.
