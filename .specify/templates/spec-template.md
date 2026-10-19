# SPEC: [NN] — [Module Title]

**Status:** Draft | In Review | Approved
**Crate:** `crates/[name]`
**Bounded context:** [context number and name from the data model]
**Owner:** [agent or author]

---

## 1. Purpose

[What this module is responsible for, in two or three sentences. Write what it owns, not what
it does step by step — the steps belong in `plan.md`.]

**Not this module's job:** [the nearest things a reader would wrongly assume belong here, and
which module owns them instead. This section prevents more defects than any other.]

---

## 2. Decisions this module implements

Cite, never restate. The reasoning lives in Notion.

| Ref | Decision | Consequence here |
|---|---|---|
| `[D-NN]` | [one line] | [what this module must therefore do] |

---

## 3. Behaviour

Written as EARS acceptance criteria so each one can become a test name.

- **Ubiquitous (always true)**
  - `[AC-U01]` The module SHALL [invariant that holds at all times].
- **Event-driven (when)**
  - `[AC-E01]` WHEN [trigger], the system SHALL [response].
- **State-driven (while)**
  - `[AC-S01]` WHILE [condition], the system SHALL [behaviour].
- **Unwanted (if / then)**
  - `[AC-W01]` IF [invalid input or failure], THEN the system SHALL [handling], AND the caller
    SHALL receive [error shape].

**Offline behaviour.** [What happens when this runs on a phone with no connection. If the
answer is "nothing, this is server-only", say so explicitly.]

---

## 4. Tenancy and permissions

| Question | Answer |
|---|---|
| Tenant-owned? | yes / no — platform-owned |
| Branch-scoped? | yes / no |
| Permissions required | `permission.slug` per operation |
| Visible to a cashier? | which fields are hidden, and from whom |

Article IV of the constitution applies in full. State here only what is **specific** to this
module — do not restate the general rules.

---

## 5. Open questions

| # | Question | Blocks | Raised |
|---|---|---|---|

**A spec with open questions blocking Gate 2 does not advance.** Ask; do not invent.
