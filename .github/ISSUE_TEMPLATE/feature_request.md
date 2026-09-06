---
name: A tool or a capability is missing
about: Something you wanted the assistant to do and it could not
labels: enhancement
---

<!--
WHAT DECIDES THIS is usually not "is it useful" but "can the model be made to
call it reliably". A tool that exists and is never selected is worse than no
tool: it takes one of nine slots in the prompt from something that would have
been called. So the question below about the sentence you would type is the
important one.
-->

**What I wanted to do**

**The sentence I would type to ask for it**

<!-- Literally. In your own language. The router matches against this. -->

**Why an existing tool does not cover it**

**Does it need the network, or a file outside the working directory?**

<!-- Both are possible and both change the answer: outbound tools live behind an
addon gate and an approval, and only two crates in the tree may open a socket. -->
