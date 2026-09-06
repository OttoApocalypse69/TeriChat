# Start Here

## Teri

Open **[TERI_TODO.md](TERI_TODO.md)**. It contains the short setup sequence and one pasteable delegation prompt.

## Coding agent

Read **[AGENTS.md](AGENTS.md)** first. Then use:

- [Current decisions](12_OPEN_DECISIONS.md) — what is settled and what remains open.
- [Roadmap and setup backlog](11_ROADMAP_AND_TASKS.md) — what to build next.
- [Full starter prompt](13_AGENT_BOOTSTRAP_PROMPT.md) — product bootstrap mission.
- [Round 1](15_REPOSITORY_CHANNELS_AND_RELEASES.md) — repo, branches, environments, channels, releases, and swarm coordination.
- [Round 2](16_TESTING_REVIEW_AND_QUALITY_GATES.md) — reviews, CI, security boundaries, merge gates, and test strategy.

The archive is a specification/agent handoff pack. It does **not** contain a completed application, live workflow configuration, installed merge bot, or configured GitHub protections.

## Putting it in a repository

Extract the ZIP, then copy the **contents** of its top-level folder into the intended repository root. Keep `AGENTS.md` at that root. Include the hidden `.github` directory. Review collisions with existing files instead of overwriting them blindly.

Issue and PR templates are ready to copy. CODEOWNERS remains a setup task because real GitHub usernames/teams are not yet confirmed. No executable deployment workflow is supplied: a generic one with unknown credentials, checks, and project paths would give false confidence.

All earlier product specifications are retained. The latest decision file is carried forward. Round 1 and Round 2 refine earlier development/release guidance; the updated index, roadmap, and bootstrap prompt point to them.

For packaging integrity, see `VALIDATION_REPORT.md` and `manifest.json`. Those checks validate this pack, **not the security or build status of future software**.
