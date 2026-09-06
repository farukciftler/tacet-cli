---
name: Something is broken
about: A tool, the shell, the sandbox, an addon or an MCP connection behaving wrongly
labels: bug
---

<!--
THE MOST USEFUL THING YOU CAN PUT HERE is what you ran and what came back. A
clear reproduction is worth more than a guess at the cause — that is not a
formality, it is how nearly every defect in this repository was actually found.

If the model simply answered badly, that is usually NOT a bug: it is a 4B model
on a laptop. But if it never got the chance to call the right tool, that IS one,
and `tacet why "<your message>"` says which it was in milliseconds.
-->

**What I ran**

```
```

**What happened**

**What I expected**

**`tacet doctor`**

<!-- Paste the output. It reports the platform, the engine features actually
compiled in, the models found and what this machine can run — which is the first
thing anyone would have to ask you otherwise. -->

```
```

**If a tool was not called: `tacet why "<the message>"`**

<!-- The router shows the model at most nine tools out of the catalog, and a tool
that is not in those nine cannot be called however well the model reasons. This
prints the ranking and the reason. -->

```
```
