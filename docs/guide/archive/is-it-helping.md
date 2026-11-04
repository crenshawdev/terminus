# Is it helping?

Terminus logs every prompt it decides about, so you don't have to take its recall on faith. Everything here is computed locally from your own sessions, and none of it leaves your machine.

## `terminus stats`

The numbers below are an example:

```text
decisions         12
  compacted       1
  ordinary        9
injected turns    9
hits              5
false positives   4
precision         0.56
misses            0
wasted budget     2
chars injected    1840
chars referenced  1020
```

| Line | What it counts |
|---|---|
| `decisions` | Every logged prompt, including the ones that got nothing. A prompt in an excluded project isn't logged. |
| `compacted`, `ordinary` | Decisions on the one prompt after a compaction, and on any other prompt. A decision logged by a build that didn't record which it was is in neither. |
| `injected turns` | Turns handed over. |
| `hits` | Injected turns where a later turn of the same session named the same path, symbol, error or command. Tool names don't count, because `Read` and `Bash` recur in nearly every session. |
| `false positives` | Injected turns where nothing later did. |
| `precision` | Hits over hits plus false positives. It reads `(nothing labelled yet)` until something has been. |
| `misses` | Prompts where Claude later called `recall_search` for something the prompt had named and nothing handed over carried it. |
| `wasted budget` | Prompts that spent characters with no hit among their turns. |
| `chars injected`, `chars referenced` | Characters spent, and the part of them hits carried. |

A hit is a proxy. It shows the same thing came up again afterwards, which doesn't prove the injected turn is why. A false positive can still have been useful. Read these numbers as a signal to compare against, and don't treat them as a measure of the work they saved.

Labels are written only once a session has closed, 6 hours after its last turn, so every count lags your live sessions. A label is never revised.

## `terminus replay`

`replay` re-scores the logged prompts of closed sessions under other thresholds and shows how each label count would move. It opens the store read-only and writes nothing. These numbers are an example too:

```console
$ terminus replay --max-turns 2
decisions        12
hit              3 -> 5
false positive   4 -> 2
miss             0 -> 0
wasted budget    2 -> 1
changed          2 decision(s)
```

`--max-turns 0` shows the archive with injection off. Each [option](../reference/commands.md#terminus-replay) defaults to the value Terminus injects with:

| Option | Default | What it changes |
|---|---|---|
| `--ranked` | 10 | How many ranked hits the threshold is applied over. |
| `--compacted-ranked` | 50 | The wider window for the one prompt after a compaction. |
| `--entity-rank` | 3 | The rank within which one structured match is enough. |
| `--co-occurring` | 2 | How many structured matches make a hit eligible at any rank. |
| `--max-turns` | 3 | The most turns one prompt may be given. |
| `--max-candidates` | 8 | How many paths and identifiers from one prompt are looked up. |

These options change this report and nothing else. No setting, option or environment variable changes the thresholds the hooks use.
