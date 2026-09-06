# Architecture Decision Records

Architecture Decision Records (ADRs) capture decisions that are important, durable, and expensive to rediscover.

They are not a diary and should not be created for routine implementation choices.

## When to create an ADR

Create an ADR when a decision:

- changes an accepted architecture or trust boundary;
- selects/replaces a foundational dependency or protocol;
- changes authoritative data/storage semantics;
- changes public compatibility/versioning policy;
- changes production topology or release strategy;
- changes security/recovery/identity policy;
- changes licensing or another difficult-to-reverse project boundary;
- intentionally supersedes a previous ADR.

Examples likely to deserve ADRs:

- selecting OpenMLS/RustCrypto for TeriCrypt;
- selecting the Recovery Vault envelope format once serialized;
- selecting NATS for a specific durable distribution boundary;
- selecting an SFU architecture;
- selecting the public license;
- changing the database authority model.

## Naming

Use zero-padded sequential identifiers:

```text
0001-use-postgresql.md
0002-use-openmls.md
0003-recovery-vault-format.md
```

`0000-template.md` is the template and does not consume a decision number.

## Status values

Use one of:

- `Proposed`
- `Accepted`
- `Rejected`
- `Deprecated`
- `Superseded by ADR-XXXX`

Only **Accepted** ADRs are authoritative decisions.

## Source precedence

For architectural policy conflicts, use this order unless an explicit owner instruction says otherwise:

```text
explicit owner-approved current decision
        ↓
Accepted ADR / locked decision register
        ↓
versioned current contracts/protocols
        ↓
AGENTS.md operational policy
        ↓
architecture/specification documents
        ↓
roadmap/proposals
        ↓
implementation comments / old brainstorming
```

Implementation and tests remain the source of truth for what is *currently implemented*. An ADR can say what must be true without proving the code already satisfies it.

## Process

1. Copy `0000-template.md` to the next ADR number.
2. Fill in context, decision, alternatives, and consequences.
3. Mark it `Proposed` while under discussion.
4. Obtain the required owner/maintainer approval for the affected boundary.
5. Change to `Accepted`, `Rejected`, or another final status.
6. Update `12_OPEN_DECISIONS.md` and relevant specs when it resolves/supersedes an existing decision.
7. Never silently edit history to make an old decision appear to have always been different. If the decision materially changes, create a new ADR and supersede the old one.

## Keep ADRs concise

An ADR should explain **why** the decision exists and what constraints it creates. Detailed implementation manuals belong in subsystem documentation.
