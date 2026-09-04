---
name: find-file
triggers: find the file, which file, where is the file, search my files, look for a file, locate the file
tools: find_file
---
# Finding files

`find_file` searches the user's OWN files on this device. It is not a web search.

## Never break these
- `find_file({"pattern":"budget"})` searches names; add `"search_content":true` to look INSIDE files.
- Never name a file the tool did not return. An invented filename sends the next turn reading a document that does not exist.
- The tool reports "nothing found" itself; never guess a name to be helpful.
<!--/core-->
## Rules
- One pattern per call: a word the user actually said, not a whole sentence.
- Found the file and the user wants its contents? That is a second call, `read_document`, with the path this tool returned.
- Answer in the user's language.
