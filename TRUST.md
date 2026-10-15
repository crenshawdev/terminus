# Trust

This is the standard every project published by John Crenshaw is held to, for the software itself and for the platforms it runs on. Each project's README carries a short version of it, plus a statement of exactly what that project sends off your machine.

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
