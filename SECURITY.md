# Security Policy

## Supported versions

Only the latest release and the current `main` branch get fixes.

## Reporting a vulnerability

Please do **not** open a public issue for a security problem. Report it privately: use GitHub's
[private vulnerability reporting](https://github.com/MnkyArts/ProjectAvalanche/security/advisories/new) if it is
available to you, or send a direct message to the maintainers on the
[Discord server](https://discord.com/invite/XrFGQEfwdJ). Include what is affected, how to reproduce it and, if
you have one, a proposed fix.

We aim to answer within a week. Once a fix is released, the report can be credited to you if you like.

## Scope

In scope: the launcher and the game, in particular the code that reads the disc image and the game's files (a
crafted ISO or data file should never lead to more than an error), the launcher's handling of paths and settings,
and the CI workflow that builds the downloads.

Out of scope: problems in the original game itself, and builds that do not come from this repository's releases or
CI runs.
