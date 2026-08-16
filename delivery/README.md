# Public capability manifest

[`capabilities.json`](capabilities.json) publishes the forward delivery graph of
this repository in a machine-readable form. It is a public integration surface,
not a claim that planned code already exists.

The states have deliberately narrow meanings:

- `planned` — specified, but at least one prerequisite or dispatch decision is
  still missing;
- `ready` — may be implemented from the current public specification;
- `blocked` — a non-empty public `blockers` list identifies why implementation
  cannot proceed;
- `verified` — merged code has a measured content digest and successful CI on
  the exact merge SHA recorded in `verification`.

All states except `verified` must have `verification: null`. A feature PR must
not mark itself `verified`: its final merge SHA does not exist yet. Evidence is
settled by a later, reviewable manifest change after the merge and its required
checks succeed. Required checks are declared on the capability, outside its
evidence, and the evidence must name that exact set.

`content_scope` and `required_checks` are versioned capability policy, not
evidence-selected input. Schema version 1 fixes the exact required-check set,
and a settlement that adds an already-verified capability or changes any part
of its declaration while moving it to `verified` is rejected. Declaration and
implementation therefore remain separate reviewable transitions.

`content_digest` is reproducible. It is SHA-256 over the sorted, deduplicated
records returned by `git ls-tree -rz --full-tree <merge_sha> -- <content_scope>`,
with every record NUL-terminated. NUL output makes the bytes independent of Git
path quoting configuration. Every declared pathspec must match at least one
tree object. The validator checks that the commit exists locally, is an
ancestor of `HEAD`, and that this digest matches.

The public validator cannot authenticate GitHub's remote CI state. It validates
the attestation structure and local Git evidence; consumers must independently
query the CI provider and verify the immutable SHA, successful run and exact
required-check set. Branch names, tags, agent reports and `ready` are not
evidence.

The schema is descriptive and portable. The stdlib-only validator additionally
checks strict keys, duplicate JSON keys, identifier formats, dependency
existence, cycles, status/evidence consistency, exact CI SHA binding and
agreement with `TODO.md`:

```bash
python3 scripts/validate_capabilities.py
python3 -m unittest discover -s tests -p 'test_capability_manifest.py'
```

Optional daemon work and direct-mode compatibility are separate lanes. They do
not silently block the core `serve` → TUI/`ctl` → skill graph.
