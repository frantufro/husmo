# Link Database

A local, git-backed store of content an AI agent can save and retrieve, used to bring links and documents into an agent's context.

## Language

**Document**:
A saved unit of content, optionally sourced from a URL.
_Avoid_: Entry, Link, Item

**Canonical URL**:
The URL a Document was fetched from, when it has one. Pasted or typed content has no Canonical URL.
_Avoid_: Source, Link (when used as a noun for the field)

**Tag**:
A free-form label attached to a Document for organizing and filtering.

**Related**:
A deliberate, symmetric, untyped connection between two Documents, declared explicitly rather than discovered by extraction.

**Outgoing Link**:
A hyperlink found in a Document's content pointing at another page, which may optionally be archived as its own Document.

## Relationships

- A **Document** has at most one **Canonical URL**.
- A **Document** has a title, zero or more **Tags**, a saved-at timestamp, an optional summary, and an optional author.
- A **Canonical URL** identifies at most one **Document**; re-saving the same **Canonical URL** overwrites that **Document**'s content rather than creating another.
- A **Document** may be **Related** to any number of other Documents. Retrieving a Document always lists what it's **Related** to; the content of those Documents is only pulled in when explicitly requested.
- A **Document**'s content may contain **Outgoing Links**; archiving one turns it into its own Document, but this is a distinct concept from being **Related** — an **Outgoing Link** is discovered, **Related** is declared.

## Flagged ambiguities

- "link" was used informally to mean a saved web page — resolved: a saved link is just a **Document** whose **Canonical URL** is set.
- "relate documents" could have meant the same thing as archiving an **Outgoing Link** — resolved: these are distinct. **Related** is a deliberate edge between any two Documents; an **Outgoing Link** is a hyperlink discovered in a Document's content.

## Flagged ambiguities

- "link" was used informally to mean a saved web page — resolved: a saved link is just a **Document** whose **Canonical URL** is set.
