# Delivery report contract

Send one Russian Telegram HTML message only after a branch merge has successful
exact merge-SHA post-merge CI. Never report commit, push, candidate CI, review,
pending/failed post-merge CI, or a plan as completed. The renderer alone creates
the supported `<b>`, `<code>`, and `<a>` tags. It HTML-escapes every dynamic
value and href attribute, renders changed file paths separately inside `<code>`,
renders the verified `owner/name` repository ID as a compact link to its
canonical GitHub project, and emits one compact `·`-separated link line without a
raw URL line. The visible merge route is only
`source_branch → target_branch`; immutable candidate and merge SHAs remain
provider-bound evidence but are not repeated in that route.

## Layout

The rendered message follows one fixed skeleton: a bold emoji headline naming the
repository, one summary sentence directly under it, then bold emoji section
headers whose facts are `•` bullets, and one compact link line last. Section
emoji and their order are part of the contract; only the `⚠️` block is optional.

```html
✅ <b>{repository}: merge завершён</b>
{change — одно предложение с итогом}

🎯 <b>Зачем</b>
{reason}

🧩 <b>Что изменилось</b>
• {component}

📄 <b>Изменённые файлы</b>
• <code>{changed path}</code>

🧪 <b>Проверено</b>
• PR #{number}: merged, <code>{source_branch}</code> → <code>{target_branch}</code>
• {job}: {conclusion}

🔐 <b>Безопасность и полномочия</b>
• {security_authority_impact}

⚠️ <b>Ограничения</b>
• Ограничения: {limitations}
• Блокеры: {blockers}
• Непроверенное: {unverified}

🔗 <b>Ссылки</b>
<a href="{project_url}">{repository}</a> · <a href="{commit_url}">commit {short_merge_sha}</a> · <a href="{pr_url}">PR #{number}</a> · <a href="{ci_url}">CI {state}</a>
```

`short_merge_sha` is the first eight characters of the verified merge SHA; the
full immutable SHA stays inside the `href`. The PR number is taken from the
already verified PR URL, never from separate caller prose. Branch names and
changed paths are the only `<code>` spans in the merge report.

`limitations`, `blockers`, and `unverified` remain required evidence fields. A
field whose value is a canonical checked-and-empty answer — `нет`,
`нет известных`, or `отсутствуют`, compared case-insensitively and ignoring
trailing spaces and dots — carries no limitation and is not rendered. When all
three are such answers the whole `⚠️` block disappears together with its blank
separator, exactly as an empty limitation section must. Any other text is a real
limitation and stays visible.

Section emoji are ordinary text, not markup: they are never escaped away, never
counted as tags, and the length gate measures them as Telegram does — in UTF-16
code units, so a non-BMP emoji costs two units.

The trusted verifier accepts exact-key JSON objects. Route fields are always
forbidden because routing is transport authority, not report content.

| Event | Required evidence fields |
|---|---|
| `postmerge` | repository/change/reason/components; security-authority impact; tests/limitations/blockers/unverified; source/target branches, reviewed candidate SHA, merge SHA, merge commit/PR URLs and successful merge-SHA CI; changed files are forbidden in caller input and added only from verified Git evidence |
| `correction` | positive prior merge-report message ID, repository/merge context SHA, controlled merge-report field type/name, old and replacement values, disposition of other facts, typed source-bound evidence object and unverified facts |

Missing and extra keys fail validation. Each identity, SHA and GitHub URL must
bind consistently. Before rendering, the verifier independently measures the
canonical SSH target head and its ancestry, plus GitHub merged-PR and workflow
identity, head, branch, event, terminal conclusion, and exact-attempt jobs.
`postmerge.tests` must exactly equal the provider's job names and outcomes;
arbitrary local-check prose is rejected. Changed files are derived with a
bounded NUL-delimited Git diff between the measured fork point of the PR's
provider-confirmed base and candidate SHAs and the candidate SHA, and must
exactly match the provider PR file set. The provider publishes the PR file set
as the three-dot comparison, so the fork point is measured as a separate
`git merge-base` of the provider-confirmed base and candidate commits, must
resolve to exactly one locally existing commit, and is recorded as a verifier
check; a base SHA that moved after the PR opened therefore cannot inject the
inverted contents of unrelated target-branch commits. Missing local merge-base
history fails closed. Caller-provided
files, an empty or mismatched list, non-normalized paths, duplicates, and more
than 64 paths are rejected. The renderer accepts only the
verifier's process-local normalized object and rejects raw sibling fields.

The provider runs noninteractively from `GITHUB_TOKEN`; caller `PATH`, credential
stores, GUI prompts, HTTPS Git credentials, and secret rendering are forbidden.
Provider commands use trusted absolute system executables with bounded streams
and whole-process-group termination. Each `gh` call receives one ephemeral
mode-0700 root with isolated absolute `HOME`, XDG and `GH_CONFIG_DIR` paths under
a fixed trusted system temporary base. Caller temp and user configuration paths
are ignored. Canonical SSH measurements ignore user SSH configuration, pin
`github.com`, require strict host-key checking, and disable proxy commands and
jumps. Local replacement refs, grafts, shallow history, promisor state, and lazy
fetching are rejected before graph evidence is used.

The verifier reads at most 128 KiB of event JSON, accepts no more than 64 values
per report array, and rejects the final rendered HTML above Telegram's
4096-unit `sendMessage` limit, measured in UTF-16 code units, without truncation. It rejects the actual
process `GITHUB_TOKEN`/`GH_TOKEN` and
high-confidence credential/private-key patterns in input and output without
echoing the detected value. Use `нет` or `нет известных` only after checking;
otherwise put the unknown fact explicitly in `unverified`.

## Successful postmerge

Send exactly once only when all of these facts are provider-confirmed:

- the PR is `MERGED` with the exact source and target branches and reviewed
  candidate SHA;
- the PR's merge commit is the reported exact merge SHA;
- the current SSH target head equals or contains that merge SHA, and the fetched
  target tracking ref equals the measured live SSH head;
- the selected GitHub Actions run has that merge SHA and target branch, event
  `push`, status `completed`, and conclusion `success`;
- `tests` exactly lists every job and its provider outcome from that exact run
  attempt;
- `files` is the non-empty normalized path set measured from the exact
  provider-bound PR base and candidate Git objects, not caller prose.

Include, in the layout above, what was merged and why, logical components and a
separate `📄` changed-file section, a separate `🔐` security/trust/
capability/approval/authority section, actual checks, limitations, blockers and
unverified facts, a clickable canonical project identity, source/target context,
reviewed candidate and merge SHAs bound behind compact evidence links, and
clickable merge commit, PR and Actions run URLs. Candidate CI is not post-merge
evidence. Pending, failed, cancelled, skipped, neutral, stale, timed-out or
otherwise non-successful workflow conclusions produce no Telegram message.

The caller records the successful Telegram response as the receipt for
`(repository, merge_sha, route)`. Do not send a second postmerge report for the
same receipt key. The verifier proves report eligibility but cannot make
Telegram `sendMessage` transactional; the controller must preserve the receipt
across restart before attempting another send.

## Correction

Send a correction only for an already delivered merge report and identify its
positive `prior_message_id`:

The correction uses the same skeleton: an `✏️` headline naming the repository, a
summary line carrying the prior positive receipt and the exact merge context SHA,
a `🔧 Исправленное поле` section with the field, its old and replacement values
and the evidence source, a `🧪 Остальные факты` section, an optional
`⚠️ Ограничения` section that disappears when `unverified` is a canonical
checked-and-empty answer, and one `🔗 Ссылки` line with the project and the
compact provider-evidence link. It uses the same renderer-only tag, escaping and
length rules as the original merge report.

Only merge-report branch fields (`source_branch`, `target_branch`), URLs
(`commit_url`, `pr_url`, `ci.url`), and `ci.state` are correctable. Evidence must
bind to the same merge `context_sha`. Branch and PR corrections use the merged
PR; commit corrections use the exact merge commit; CI corrections use a
successful completed target-branch `push` run on the merge SHA. Generic SHA and
prose corrections remain rejected. A correction does not permit another
postmerge report and cannot create merge or CI evidence retroactively.
