# Security policy

## Supported versions

Security fixes are applied to the latest released version. This experimental
project does not currently maintain older release branches.

## Reporting a vulnerability

Do not open a public issue containing an unpatched vulnerability, credentials,
private prompts, source code or rollout logs. Use GitHub's private vulnerability
reporting for this repository. Include the affected version and platform,
reproduction steps, expected impact and the smallest redacted diagnostic data
that demonstrates the issue.

If private reporting is unavailable, open a public issue requesting a private
contact channel without disclosing vulnerability details.

## Scope and operational safety

`codex-claude-mode` is an experimental local frontend. It starts the separately
installed Codex CLI and can surface requests to execute commands or modify
files. Review the selected agent, effective permission profile and every
approval before accepting it. The project does not provide a security boundary
independent of the configured Codex backend and operating system.
