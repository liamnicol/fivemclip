# Code signing

FiveMClip releases are unsigned. This is a deliberate "not yet" rather than an
oversight, and this note records why, so the decision does not have to be
re-derived later.

## What being unsigned actually costs

**SmartScreen.** Windows shows "Windows protected your PC" on install.
Reputation is tracked per-certificate *and* per-file-hash, so an unsigned build
starts from zero on every single release. A signed build accumulates reputation
against the certificate and carries it forward. For a project that ships
updates, that carry-over is the real value of signing - more than the first-run
experience.

**Antivirus heuristics.** This is the one people forget, and for this app it may
bite harder than SmartScreen. An unsigned binary that captures the screen,
records audio, spawns a subprocess and writes files to disk looks a great deal
like a RAT to a heuristic scanner. Signing reduces this materially, though it
never eliminates it.

## Why there is no cheap option any more

Before June 2023 you could buy an OV certificate as a `.pfx` file for about $80
a year and sign with it directly. That is gone. The CA/Browser Forum now
requires *every* code signing private key to live on FIPS 140-2 Level 2
hardware - a physical token or a cloud HSM. File-based certificates no longer
exist at any validation level, which means most signing guides written before
2023 describe something you cannot buy.

## The constraint that decides the approach

**A USB token cannot be plugged into a GitHub Actions runner.**

Since the whole point of the pipeline is that CI produces the installer, a
hardware token in a drawer is not a solution - it would mean signing by hand on
a local machine for every release. Signing in CI requires a cloud signing
service, which rules out the default product most certificate authorities will
try to sell you.

## Options

| | Cost | Company required | Clears SmartScreen |
| --- | --- | --- | --- |
| Unsigned | free | no | no |
| Azure Trusted Signing | ~$10/mo | yes, with verifiable history | over time |
| OV + cloud HSM (SSL.com eSigner, DigiCert KeyLocker) | $250-500/yr | usually | over time |
| EV + cloud HSM | $350-700/yr | yes, strictly | immediately |

**Azure Trusted Signing** is far the best value if you qualify: Microsoft's own
service, integrates with CI cleanly, and priced like a subscription rather than
a certificate. The catch is identity verification - organisations have needed
several years of verifiable history, which rules out a company registered last
week. Individual developer accounts exist; check the current requirements
directly, as they have moved more than once.

**EV** is the only thing that removes the warning on day one. Everything else
earns trust through download volume, which for a single community's tool may
take a long time or never arrive.

## Switching it on

`.github/workflows/build.yml` carries a commented-out Azure Trusted Signing
block. Add the secrets it names, uncomment it, and set `signCommand` under
`bundle.windows` in `src-tauri/tauri.conf.json`.

**Keep the timestamp URL.** Without timestamping, every signature becomes
invalid the day the certificate expires - retroactively breaking installers
people have already downloaded. With it, they stay valid indefinitely.

## Until then

- Put a screenshot of the SmartScreen dialog in your install instructions with
  "click More info, then Run anyway". Removing the surprise removes most of the
  friction.
- Upload each release to VirusTotal and link the result. A clean scan across 70
  engines is worth more to a sceptical user than a certificate they cannot
  inspect anyway.
- Report false positives to Microsoft's malware analysis portal. Turnaround is
  usually a day or two.
- Keeping the repository public is itself a trust argument. For a screen
  recorder, "you can read exactly what it does" carries real weight, and it is
  something a certificate does not give you.
