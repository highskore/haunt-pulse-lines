<div align="center">

<img src="docs/icon.png" width="96" alt="">

# Pulse lines

**A sparkline of each agent's tool calls per minute on its Haunt row.**

[![Haunt plugin](https://img.shields.io/badge/Haunt-plugin-D97757)](https://github.com/highskore/haunt-hackathon)
![Plugin API 0.5](https://img.shields.io/badge/plugin%20API-0.5-111)
![WASM component](https://img.shields.io/badge/WASM-component-654ff0?logo=webassembly&logoColor=white)
![MIT licence](https://img.shields.io/badge/licence-MIT-6b7280)

</div>

Each agent row gets a small moving line in its right-hand slot: tool calls per minute over the last 30 minutes, scaled to that agent's busiest minute. A busy agent draws a lively line. One that is running but has made no tool call for 5 minutes goes flat and grey, with `quiet 6m` after its meta line. One that makes the same call over and over (8 times in a row by default) turns orange, with `same Bash call ×14`. The line moves on every 30 seconds even when no events come. While the agent needs you, Haunt's own status takes the row and the line is hidden.

A call counts once, when it starts; subagents' calls count too. A new prompt starts the loop count over, and an agent waiting on you is never counted as quiet.

## Settings

```toml
[plugins."dev.haunt.pulse-lines"]
minutes = 30          # points in the line, 5 to 32
quiet_after_min = 5
loop_after = 8
notes = true          # "quiet 6m" and "same Bash call ×14"
subagents = true      # count subagents' calls
unverified = false    # also count events without the tab's nonce
```

## Capabilities

| capability | why |
|---|---|
| `events` (prompt, tool, subagent, permission request, stop) | the calls and when a turn starts, waits or ends |
| `decorate` | the sparkline and note on the row |
| `event_details` | commands and paths, to tell the same call repeated from steady work; haunt asks in red |
| `timer` | moves the line on every 30 seconds |

Spotting a loop compares each call's command or file path with the one before; nothing is stored beyond the last call. Events without the tab's nonce are ignored unless `unverified` is on.

## Build and test

```sh
cargo build --release --target wasm32-wasip2   # or ./build.sh
cargo test                                     # unit tests
haunt plugin install .
```

`e2e/pulse_lines.rs` runs the built plugin against a real Haunt core: copy it into a Haunt checkout's `cli/tests/` and run `HAUNT_PLUGINS_DIR=<this repo's parent> cargo test -p haunt-cli --test pulse_lines`.

Needs Haunt with plugin API 0.5 (`api = "0.5"`).

## Licence

MIT, see `LICENSE`.
