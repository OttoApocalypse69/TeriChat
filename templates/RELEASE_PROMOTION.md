# Release Promotion Record

- Component/version/build ID:
- Source commit/tree:
- Source channel → destination channel:
- Environment:
- Artifact target and immutable digest(s):
- Build configuration/provenance:
- Updater and platform-signing evidence:
- Trusted gate-policy revision:

## Candidate evidence

Current CI, supported client/server compatibility, package install/update, Staging results, migration tests, restore drill, and relevant scheduled findings.

## Safety and data

- Known S0/S1: none, or explain why promotion must stop.
- Accepted lower-severity limitations:
- Schema/cache compatibility:
- Rollback or roll-forward procedure:
- Data-loss implications of restore:
- Emergency disable behavior:

## Approval and rollout

- Authorized human approver:
- Approval timestamp and exact artifact identity:
- Rollout cohort or single-node procedure:
- Health signals and abort criteria:
- Post-deployment verification:

If bytes change through rebuilding/repackaging, this record is no longer evidence for the new artifact. Re-run artifact-dependent checks; do not edit the digest to imply earlier checks tested it.
