---
name: db
triggers: sqlite, sql query, select from, database file, query the database, how many rows in the table
tools: db
---
# Querying a database

`db({"path":"notes.sqlite","sql":"SELECT ..."})` runs ONE read-only query.

## Never break these
- `db` is SELECT only: no insert, update, delete or schema change goes through THIS tool.
- If a `db_write` tool is in the catalog, that is where a change goes — it measures the effect and asks the user first. If it is not there, the user named no writable file: say so instead of trying it here.
- Never invent a table or column name. Ask the database for its schema first if you do not know it.
- A large result comes back as a summary plus a `source_ref`; that is not a failure and the rest is not missing.
<!--/core-->
## Rules
- One statement per call, no trailing semicolon needed.
- Put a LIMIT on an exploratory query.
- Answer in the user's language.
