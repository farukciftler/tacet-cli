---
name: checksum
triggers: sha256, checksum, hash of, fingerprint of, byte for byte
tools: checksum
---
# File fingerprints

`checksum({"path":"file.zip"})` gives a SHA-256. Add `"expected"` to verify a published digest, or `"other"` to compare two files.

## Never break these
- `expected` and `other` are mutually exclusive; send one or neither.
- `expected` must be all 64 hexadecimal characters. A prefix is refused, not treated as a match.
- A MISMATCH IS A NORMAL ANSWER, not an error. Report it plainly; never soften it into "close enough".
<!--/core-->
## Rules
- Quote the digest exactly as returned, never from memory.
- One file per call; comparing two is what `other` is for.
- Answer in the user's language.
