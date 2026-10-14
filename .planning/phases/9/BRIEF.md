Write README.md for Terminus so the first screen tells an experienced engineer exactly what it is, where it stands, and what it will and won't do. Terminus is part of Foundation alongside Baley. Its README follows the same structure and rules so the two read as one body of work.

Read the whole repo before writing: the current README, any docs, the code, open issues and the git history. Every claim must be true of main today or plainly labeled as planned. If anything in this prompt disagrees with the repo, stop and ask me. Do not guess.

Facts from me, to check against the repo
- Terminus will be installed by a shell script, which is still in progress. It is not installed through npm.
- The optional model/observations add-on is being removed.
- Before release, each session will be stored as numbered block rows (session_no, block_no, bytes, hash), and the session checksum becomes a hash over the block hashes. Mention this only if the repo's docs or issues already describe it, and only as planned.
- The license is undecided. Do not change LICENSE. The README names whatever license the LICENSE file holds.

Writing rules
- Show, never claim. Do not describe Terminus as serious, honest, hype-free or anything like it, and never mention or compare other tools.
- No images, GIFs, banners, badges, emoji, superlatives, star charts or comparison tables.
- Nothing unbuilt reads as a feature. Planned work belongs in Current state or the issues.
- State limits, failure behavior and refusals with the condition that triggers them.
- No counts that go stale.
- Keep it short. Any procedure longer than a paragraph moves to a file under docs/ and gets linked.
- Add nothing about being open source or free beyond the Sponsoring line below.

Structure
1. # Terminus, then one sentence saying what it does and for whom, with no adjectives.
2. A status note in the same form as Baley's ("**Status: ...**"), stating plainly what works, what doesn't, and whether anything is released.
3. ## Trust, with this text:

Terminus collects nothing about you, sends no telemetry, and sends data only to services you configure. Because that guarantee depends on the operating system underneath, the same standard decides which platforms Terminus supports. Linux and macOS meet it. Windows fails it on privacy and on security and is not supported. A supported platform that stops meeting the standard loses support. The criteria and reasoning are in [TRUST.md](TRUST.md).

**What leaves your machine.** Build this list from the code: every destination, what is sent to it, and the setting that turns it on. If nothing leaves, write "Nothing." If the model/observations add-on still sends anything, list it here and note its removal under Current state. If the code sends anything the user did not configure, stop and tell me, because the first sentence would be false.

4. ## What it does: bullets, each one a capability that works on main today.
5. ## What it is designed to refuse: the conditions under which it refuses or does nothing.
6. ## Current state: every gap between what the docs describe and what the code does, one bullet each with its issue. Omit the section if there are none.
7. ## How it works: the mechanism. Cover the components, where data is stored and in what form, and what happens on a crash or context loss. Link any design docs.
8. ## Hosts and platforms: the hosts it works with, Linux and macOS, and any system requirements.
9. ## Installing and updating: short. If the install script isn't finished, say so and don't document an npm path.
10. ## How it is designed and built: where design decisions live, the required checks every change passes, and the issues. Community links come last, never above the docs.
11. ## Issues and contributions: "Issues are welcome: bugs, questions, and reports of anything in the docs that is wrong. Pull requests are by invitation only; open an issue first. Security problems go through [SECURITY.md](SECURITY.md), not a public issue." If there is no SECURITY.md, tell me instead of writing one.
12. ## Sponsoring: "Terminus is open source, built by one person, and sold to no one. If it is useful to you, [GitHub Sponsors](https://github.com/sponsors/crenshawdev) is the way to support it."
13. ## Lineage, only if the history shows one.
14. ## License, matching the LICENSE file.

Add TRUST.md at the repo root with exactly this content. It is shared word for word across every repo, so do not adapt it to Terminus.

```markdown
# Trust

This is the standard every project published by crenshawdev is held to, for the software itself and for the platforms it runs on. Each project's README carries a short version of it, plus a statement of exactly what that project sends off your machine.

## The software

- No telemetry, analytics or crash reporting.
- Nothing is collected about you.
- Data leaves your machine only for a service you configure, and only to that service.
- Each README states what the project sends and where. When the answer is nothing, the README says so.

## Platforms

A guarantee made by the software holds only as long as the operating system underneath doesn't undo it. Platform support is decided by three criteria, each checked against a default installation.

1. Installing and using the system does not require a vendor account.
2. The vendor does not hold a readable copy of the key to your encrypted disk.
3. Collection of usage and diagnostic data is opt-in, or can be turned off completely.

Linux and macOS meet all three. Windows fails all three.

- Setup requires a Microsoft account, and Microsoft has been removing the ways around it since October 2025.
- When you sign in with a Microsoft account, the BitLocker recovery key is uploaded to Microsoft and stored where Microsoft can read it. Microsoft has confirmed it turns these keys over to law enforcement under warrant.
- Home and Pro editions send a minimum level of diagnostic data that cannot be turned off.

Running a project inside WSL or a virtual machine on a Windows host doesn't change this. The host still holds the disk, the keys and the account.

## What unsupported means

Issues opened from Windows are closed. Patches that add Windows support are not accepted. The license lets anyone fork a project and port it, and the port is theirs to maintain.

## When a platform changes

The criteria are checked against platforms as they ship. A supported platform that stops meeting them loses support. A platform that comes to meet them can be considered.
```

Do the work on a new branch with signed commits and open a pull request. Do not merge. In the PR description, list each claim in Trust and Current state alongside the file or issue that backs it.
