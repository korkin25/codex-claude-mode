---
name: telegram-delivery-reporter
description: Form, validate, and send one exact redacted Russian Telegram report after a branch merge has successful exact merge-SHA post-merge CI. Use for that single post-merge delivery checkpoint, or to correct an already sent merge report, only when the current active root/user instructions authorize a Telegram destination.
---

# Telegram Delivery Reporter

Send a delivery checkpoint only from observed evidence. The message records an
event; it does not create task, review, approval, merge, release, or deployment
authority.

## Prepare

1. Resolve route authority only from the current active root/user instructions.
   A caller may pass the already-authorized `chat_id` and `message_thread_id` as
   transport arguments, but arguments do not grant authority. Repository
   references, skill files, prior messages, memory, defaults, and discovered
   topics are not route sources. Stop if current authority is absent or
   ambiguous.
2. Select exactly one event: `postmerge` or `correction`. Never report commit,
   push, candidate CI, review, a pending/failed post-merge run, or another
   intermediate checkpoint.
3. Read [references/report-contract.md](references/report-contract.md). Gather
   every required fact from Git, the hosting service, and CI.
   Postmerge tests must be the exact GitHub run-attempt job names and outcomes.
   The caller must not provide a changed-file list. The verifier derives the
   exact normalized paths from the measured fork point of the provider-bound PR
   base and candidate Git objects, compares them with the provider PR file set,
   and rejects an empty, mismatched, malformed, duplicate, or oversized result.
   The renderer derives the canonical GitHub project URL from that verified
   repository identity. It renders the repository ID as the project link and
   the source-to-target route without SHAs; the separately verified SHAs remain
   bound to the compact commit/PR/CI evidence links, where the merge commit link
   shows only the first eight SHA characters and keeps the full SHA in its href.
   The layout — bold emoji section headers, one summary sentence under the
   headline, `•` bullets, `<code>` branches and paths, and one `·`-separated link
   line — is fixed by the contract, including the `⚠️ Ограничения` block that
   disappears entirely when limitations, blockers and unverified facts are all
   canonical checked-and-empty answers.
   Do not fill missing evidence from memory or prose.
4. Redact secrets and private payloads before composing. Keep the exact IDs,
   repository coordinates, file/component names, outcomes, and links needed for
   delivery evidence, but omit credentials, secret values, raw prompts, user
   data, confidential logs, and diff contents.
   The verifier also rejects the actual process `GITHUB_TOKEN`/`GH_TOKEN`,
   high-confidence credential and private-key forms, oversized input, lists
   above 64 entries, and rendered Telegram text above 4096 UTF-16 code units,
   the unit Telegram itself counts, so section emoji cannot smuggle the message
   past the limit.
5. Prefetch the exact current target branch through the canonical
   SSH `origin`; the verifier rejects stale `refs/remotes/origin/*` after
   comparing them with a separate SSH `ls-remote`. Require `gh >= 2.97.0` and
   `GITHUB_TOKEN` in the process environment. The verifier ignores caller
   `PATH`, selects only fixed absolute system `git`, `gh`, `ssh`, and `false`
   candidates with trusted ownership, modes, and parent directories, bounds
   both output streams while the process runs, and kills the complete process
   group on timeout or overflow. Each `gh` call receives an ephemeral absolute
   `HOME`, XDG config/state/cache, and `GH_CONFIG_DIR` under one mode-0700
   temporary root created under a fixed trusted system temporary base. System
   executables, that base, and all of their parents must have one consistent
   trusted system owner. Caller
   `TMPDIR`, `TEMP`, and `TMP` are ignored, and the root stays outside the
   checkout and caller `HOME`/XDG paths, so provider state cannot be written
   into the checkout or read from a user credential store. Local Git replacement
   refs, grafts, shallow history, promisor state, and lazy object fetching are
   rejected before graph evidence is used. SSH ignores user configuration,
   pins the GitHub hostname and host-key identity, and disables proxy commands
   and jumps. Never
   obtain a token from a credential store, pass it on the command line, print
   it, or enable an interactive prompt. Build one strict event object without
   route fields and run from the canonical repository root:

   ```bash
   PYTHONDONTWRITEBYTECODE=1 python3 \
     .agents/skills/telegram-delivery-reporter/scripts/verify_report.py \
     < /path/to/private-temporary-event.json
   ```

   The verifier measures GitHub PR/run facts with noninteractive `gh`, Git
   branch heads through the canonical SSH remote, and the exact changed paths
   with a bounded NUL-delimited Git diff from the separately measured
   `git merge-base` fork point to the candidate. It then passes a
   process-local normalized object to the renderer. Never call `validate_report.py`
   directly; it rejects raw event JSON. Use verifier stdout as the exact
   message. Missing credentials/evidence, a provider/ref mismatch, missing or
   extra fields, a self-declared or mismatched job, invalid JSON, or renderer rejection
   leaves the reporting gate unmet. Unknown facts are allowed only in the
   event's explicit `unverified` field or another contract-defined marker.

## Send and confirm

Call the Telegram MCP using this exact shape. Do not change `parse_mode`, enable
link previews, or modify the verifier stdout:

```json
{
  "method": "sendMessage",
  "params": {
    "chat_id": "<already-authorized chat_id>",
    "message_thread_id": "<already-authorized message_thread_id>",
    "text": "<validator stdout>",
    "parse_mode": "HTML",
    "link_preview_options": {"is_disabled": true}
  }
}
```

A workflow-mandated routine report needs no extra conversational confirmation;
do not attempt to bypass a platform tool gate if one appears.

Require a successful Telegram response whose `Message.chat.id` equals the
authorized `chat_id`, whose `Message.message_thread_id` equals the authorized
topic, and whose `Message.message_id` is a positive integer. Record the exact
request payload and that ID in
the working report. A tool error, shape mismatch, wrong route, or absent
confirmation leaves the reporting gate unmet. Retry only the same validated,
redacted factual payload on the authorized route.

Send `postmerge` exactly once only after the verifier confirms a merged PR and a
successful completed `push` workflow on the exact merge SHA and target branch.
Pending, failed, cancelled, or otherwise non-successful CI sends nothing. Keep
the successful Telegram response as the merge-report receipt. If that report is
wrong, send one `correction` identifying its positive `message_id` and the exact
replacement for a merge-report field. A correction never authorizes a second
merge report. Generic SHA and prose corrections remain unsupported because their
semantics cannot be provider-bound by a mutable ref.

Return to the caller the event type, route identifiers, Telegram `message_id`,
exact merge SHA, and successful CI state. Do not repeat private message content
in the result.
