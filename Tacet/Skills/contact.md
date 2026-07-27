---
name: contact
triggers: person, number, their phone, e-mail, mail address, address book, in my contacts, phone number, contact, email address
tools: contact
---
# Contacts

Search contacts by name with the `contact` tool; it returns phone/email.

## Rules
- `name`: the name to search; a partial name is fine.
- No match: never invent one; say that it was not found.
- Give phone numbers and emails exactly as the tool returns them, unmodified.
