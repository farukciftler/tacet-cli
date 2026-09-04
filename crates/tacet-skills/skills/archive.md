---
name: archive
triggers: zip file, unzip, unpack, extract the archive, inside the zip, compressed archive
tools: archive
---
# Zip archives

`archive({"path":"backup.zip","action":"list"})` names what is inside; `"extract"` unpacks it.

## Never break these
- `list` decodes nothing: its sizes are the archive's OWN claim, not measured facts. Say "declared" if you quote them.
- `extract` unpacks into a NEW subfolder and never overwrites anything. You cannot choose where.
- A refused archive is a refusal, not a bug: an entry escaping the folder, a symlink, a size cap or a failed CRC stops the whole archive.
<!--/core-->
## Rules
- A long listing comes back as a count plus a `source_ref`; never invent entry names.
- Want the contents of one entry? Extract first, then `read_document` on the path.
- Answer in the user's language.
