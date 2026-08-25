# Public Repository Publication Policy

OpenMirai is public. Every committed file, branch, pull-request revision,
workflow artifact, screenshot, fixture, example, and log must be safe for
unrestricted publication and permanent redistribution.

## Default classification

- Material already available in this repository's public default branch,
  public issues, public pull requests, or an external public specification is
  **public source material**.
- Content from private repositories, local workspaces, chats, meetings,
  internal knowledge bases, unpublished branches, customer work, or private
  infrastructure is **private source material**.
- If the classification is unclear, treat the material as private.

Private source material must never be copied wholesale into OpenMirai. A fact
learned privately may appear only when its publication has been explicitly
approved and it can be independently expressed and verified from OpenMirai's
public code or public product contract. Approval applies to the exact facts,
not to an entire source document.

## Never publish

- credentials, tokens, private keys, cookies, connection strings, or real
  secret values;
- customer names or data, contracts, financial/legal records, founder or team
  discussions, private meeting notes, or incident details;
- private repository names or links, internal document paths, local workspace
  names, personal home-directory paths, or unpublished branch content;
- private hosts, IP addresses, tailnet details, deployment identifiers, account
  numbers, or infrastructure screenshots;
- internal roadmaps, pricing, product plans, cross-repository architecture, or
  capability claims that have not been approved for public release;
- raw logs, database extracts, screenshots, or fixtures captured from a private
  environment, even when they appear harmless.

Use synthetic identities, paths, hosts, payloads, and screenshots in examples.
Redaction is not sufficient when surrounding context still identifies a person,
customer, private project, or internal system.

## Required publication workflow

1. Write from public OpenMirai code and public contracts. Do not begin by
   copying a private document and editing it down.
2. Record any non-public fact proposed for publication in the pull-request
   description together with explicit owner approval.
3. Review the complete diff, including generated files, images, fixtures,
   examples, workflow artifacts, and deleted-then-readded content.
4. Run `bash tools/check_publication_safety.sh`.
5. Complete the public-repository safety checklist in the PR template.
6. Require a human reviewer to confirm the public/private boundary before
   merge. Automated checks supplement this review; they cannot classify prose.

Repository administrators must protect `main` with a ruleset that requires a
pull request, a human approval, and the `public repository boundary` status
check; dismisses stale approvals when new commits arrive; and blocks branch
deletion and force-pushes. GitHub Secret Protection and push protection must be
enabled with bypass restricted to designated reviewers.

Documentation about private products, company operations, customers, or
cross-repository implementation details stays in the private documentation
system. OpenMirai may document only its own public API and generic integration
contracts.

## Branches and history

Treat every pushed branch and every PR revision as public. Deleting a branch or
removing a file in a later commit does not retract content from Git history,
forks, caches, notifications, or review artifacts. Local drafts and abandoned
branches are unapproved source material until they pass this policy again.

## If private material is found

1. Stop publishing and notify the repository owner privately.
2. Identify whether credentials or personal/customer data are involved without
   reposting the material in an issue or chat.
3. Rotate exposed credentials immediately; history rewriting is not a
   substitute for rotation.
4. Decide the containment and history-rewrite procedure with the owner. Do not
   force-push or delete history without explicit approval and a recovery plan.
5. Re-run the boundary check and manually inspect the cleaned repository and
   affected PR artifacts before resuming publication.

This policy also applies when documentation is generated or summarized by an
AI agent. The agent may not infer publication permission from filesystem access.
