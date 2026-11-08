# AI agent in the system

The AI agent is **part of the system, not of one application**: the
`agentd` service holds agent sessions, the model sits behind one
provider-independent interface (Claude API today, a local model later) and
system services supply the tools. The agent never has more rights than the
user who opened the session, and it **cannot even technically write
outside the assigned workspace** - its file capabilities are read-only.
Security rests on capabilities and user confirmation, **not on the model
obeying**.

Version 1 (this page). Code: `libs/agentproto` (model interface, tools,
protocols, rules, limits, fake provider, Claude API codec, key
encryption), `libs/agentsh` (shell helper: checking a proposed command
with the shlang parser), `user/agentd` (the service),
`user/init/src/agent.rs` (the gate: starting `agentd`, issuing session
capabilities, system tools), tests `user/init/src/tests/agent.rs`.

<figure class="diagram">
<svg viewBox="0 0 760 380" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Parts of the agent layer">
<defs><marker id="ag1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="ag1a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="170" height="110" rx="6"/><text class="t b" x="95" y="45" text-anchor="middle">client</text><text class="m" x="95" y="66" text-anchor="middle">shell (ai), tests,</text><text class="m" x="95" y="84" text-anchor="middle">GUI later</text><text class="m" x="95" y="106" text-anchor="middle">confirms and performs</text><text class="m" x="95" y="122" text-anchor="middle">writes outside workspace</text>
<rect class="acc" x="250" y="20" width="250" height="210" rx="6"/><text class="t b" x="375" y="45" text-anchor="middle">agentd</text><text class="m" x="375" y="68" text-anchor="middle">sessions: conversation, steps</text><text class="m" x="375" y="86" text-anchor="middle">rules: free / confirm / deny</text><text class="m" x="375" y="104" text-anchor="middle">workspace: commits</text><text class="m" x="375" y="122" text-anchor="middle">token and spending limits</text><text class="m" x="375" y="140" text-anchor="middle">audit log</text><text class="m" x="375" y="158" text-anchor="middle">paths that never leave the PC</text><text class="m" x="375" y="176" text-anchor="middle">encrypted API keys</text><text class="m" x="375" y="200" text-anchor="middle">trait Provider</text>
<rect class="box" x="570" y="20" width="180" height="60" rx="6"/><text class="t b" x="660" y="45" text-anchor="middle">Claude API</text><text class="m" x="660" y="64" text-anchor="middle">TLS + HTTP, 443 only</text>
<rect class="box" x="570" y="95" width="180" height="50" rx="6"/><text class="t b" x="660" y="118" text-anchor="middle">fake model</text><text class="m" x="660" y="135" text-anchor="middle">tests, no network</text>
<rect class="box" x="570" y="160" width="180" height="50" rx="6"/><text class="t b" x="660" y="183" text-anchor="middle">local model</text><text class="m" x="660" y="200" text-anchor="middle">later</text>
<rect class="ok" x="10" y="270" width="230" height="100" rx="6"/><text class="t b" x="125" y="294" text-anchor="middle">session capabilities</text><text class="m" x="125" y="315" text-anchor="middle">FS read-only (user identity)</text><text class="m" x="125" y="333" text-anchor="middle">FS write in workspace only</text><text class="m" x="125" y="351" text-anchor="middle">system tools of the user</text>
<rect class="box" x="270" y="270" width="230" height="100" rx="6"/><text class="t b" x="385" y="294" text-anchor="middle">init (gate)</text><text class="m" x="385" y="315" text-anchor="middle">starts agentd with a manifest</text><text class="m" x="385" y="333" text-anchor="middle">mints session capabilities</text><text class="m" x="385" y="351" text-anchor="middle">service state, crashes, log</text>
<rect class="box" x="530" y="270" width="220" height="100" rx="6"/><text class="t b" x="640" y="294" text-anchor="middle">tool providers</text><text class="m" x="640" y="315" text-anchor="middle">services register themselves</text><text class="m" x="640" y="333" text-anchor="middle">JSON schema, kind of effect</text><text class="m" x="640" y="351" text-anchor="middle">a call carries session caps</text>
<path class="ln-acc" d="M180 70 L248 70" marker-end="url(#ag1a)"/><text class="m" x="186" y="62">session</text>
<path class="ln" d="M500 50 L568 50" marker-end="url(#ag1)"/>
<path class="ln" d="M500 120 L568 120" marker-end="url(#ag1)"/>
<path class="ln dash" d="M500 185 L568 185" marker-end="url(#ag1)"/>
<path class="ln" d="M385 268 L385 232" marker-end="url(#ag1)"/><text class="m" x="392" y="256">opens a session</text>
<path class="ln-acc" d="M200 268 L290 232" marker-end="url(#ag1a)"/>
<path class="ln" d="M600 268 L480 232" marker-end="url(#ag1)"/>
</svg>
<figcaption>The client opens a session through the gate in init; agentd talks to the model and tools only through session capabilities.</figcaption>
</figure>

## Terms

| Term | Meaning |
|---|---|
| **agent session** | one conversation with one user: messages, steps, workspace, limits. It belongs to the user who opened it; no other user sees it |
| **model provider** | an implementation of `agentproto::Provider` (Claude API, fake model, later local). Clients do not see it - they only talk to `agentd` |
| **tool** | an operation the model may call (`fs.read`, `ws.diff`, `sys.services`, ...). It has a name, a description, an **input schema** (JSON Schema) and a **kind of effect** |
| **kind of effect** | `read`, `write` (file system), `exec` (running something), `net`, `settings`, `propose` (only a proposal for the user). It decides whether confirmation is needed |
| **workspace** | a dataset where the agent writes **without confirmation**. Session start = a commit, every step = a commit; the session can be reverted with one command |
| **step** | one model response with all its tool calls. If a step wrote to the workspace, it becomes the commit "agent: step N" |
| **confirmation** | a question to the client with a preview of the change (diff). A confirmed action is **performed by the client with its own rights**; `agentd` cannot perform it |
| **gate** | the part of init (`user/init/src/agent.rs`) that starts `agentd` and mints capabilities with the user's identity for every session |

## Model interface (agentproto)

The model sits behind a provider-independent trait. Clients only send text
and answers to confirmations; `agentd` holds the conversation, the tools
and the call loop. A new provider (a local model, another company) can be
added **without changing clients** - implement the trait and add the name
to the selection in `agentd`.

```rust
pub trait Provider {
    /// Provider and model name (for the audit and the spending overview).
    fn name(&self) -> &str;
    fn model(&self) -> &str;
    /// Price list (micro-dollars per million tokens); fake and local model 0.
    fn price(&self) -> Price;
    /// One model call. Stream events (text, start of a tool call, ...)
    /// go to `sink` as they come; the result is the whole message.
    fn complete(&mut self, req: &Request, sink: &mut dyn FnMut(&Event)) -> Result<Response, ProviderError>;
}
```

- **Messages** (`Message`): role `user` / `assistant`, content is a list of
  blocks: `Text`, `ToolUse { id, name, input }`, `ToolResult { id, content,
  is_error }` and `Opaque` (a provider block the layer does not understand -
  e.g. a Claude thinking block with a signature; it is sent back
  **unchanged**).
- **Streaming** (`Event`): `MessageStart`, `TextDelta`, `ToolUseStart`,
  `ToolInputDelta`, `BlockStop`, `MessageDelta { stop, usage }`,
  `MessageStop`. The accumulator (`stream::Accumulator`) builds the message
  from events - the same for Claude SSE and for the fake model.
- **Stop reason** (`Stop`): `EndTurn`, `ToolUse`, `MaxTokens`, `Refusal`,
  `Other(text)`. `agentd` reports `Refusal` to the user and calls nothing
  else.
- **Usage** (`Usage`): input, output, cache write and cache read tokens.
- **Price list** (`Price`): micro-dollars per million tokens for input,
  output, cache write and cache read. The price of a call is integer
  arithmetic, no `f64`.

### Claude API

The first provider (`agentproto::claude`): `POST https://api.anthropic.com/v1/messages`,
headers `x-api-key`, `anthropic-version: 2026-10-21`, a body with `model`,
`max_tokens`, `system`, `messages`, `tools` (name, description,
`input_schema`) and `output_config.effort`. Default model
`claude-opus-5-5` (USD 4 / 20 per million input / output tokens, cache read
0.20, cache write 5). The tool results of one step go in **one** `user`
message. The codec is a pure function (host tests): building the body,
reading the response and the SSE stream (`message_start`,
`content_block_start`, `content_block_delta` with `text_delta` /
`input_json_delta`, `content_block_stop`, `message_delta`, `message_stop`,
`error`).

Transport goes through `libs/tls` + `libs/httpc`. Those still return the
whole body at once, so version 1 sends the request without `stream` and
makes events from the finished response; the SSE codec is ready for when
`httpc` can read in parts.

## Tools from services

A tool is described by `ToolSpec`: name (`group.action`), description for
the model, **input schema** (a subset of JSON Schema: `object`,
`properties`, `required`, types `string` / `integer` / `boolean` /
`array`) and kind of effect. `agentd` **checks the model's input against
the schema before calling the tool** - invalid input goes back to the
model as a tool error.

| Tool | Effect | Provided by | Description |
|---|---|---|---|
| `fs.read` | read | agentd | file contents (at most 64 KiB, offset) |
| `fs.list` | read | agentd | directory listing (name, kind, size) |
| `fs.stat` | read | agentd | path metadata |
| `fs.search` | read | agentd | search by name and text under a directory (limited depth and count) |
| `fs.write` | write | agentd | write a file (whole contents) |
| `fs.mkdir` | write | agentd | new directory |
| `fs.remove` | write | agentd | move to trash (never a permanent delete) |
| `fs.rename` | write | agentd | rename / move |
| `ws.diff` | read | agentd | workspace changes since session start or a step |
| `ws.steps` | read | agentd | session steps (commits) |
| `shell.propose` | propose | agentd | a shlang command proposal, checked by the parser (shell mode) |
| `dev.build`, `dev.test` | exec | the compiler later | prepared, return `NOT_SUPPORTED` today |
| `sys.services` | read | init | service state from the supervisor |
| `sys.crashes` | read | init | crashes: exit code, restarts, reason for giving up |
| `sys.log` | read | init | recent supervisor events |
| `sys.net` | read | init | network state (address, gateway, DNS) |

Tasks (`taskd`) and settings are added the same way once their services
run: a service sends `agentd` a `Register` message with a list of
`ToolSpec` (protocol `agentproto::registry`), similar to an MCP server. A
call carries the **session capability** (a new FS connection with the same
identity and mounts), so the service performs the operation with the
session's rights, not with the rights of `agentd`.

### Tool rights through capabilities

`agentd` itself has **no rights** to user files - its own FS session sees
only `/system/agent` (state, audit, keys). For every session the gate in init
issues:

1. **Read-only FS** - `CONNECT_AS(user, groups, [/ -> / READ_ONLY])`. Reads
   see exactly what the user sees; the FS refuses writes through it.
2. **Workspace FS** - `CONNECT_AS(user, groups, [P -> P])` only for the
   workspace dataset `P` (the path is the same in both sessions).
3. **System tools** - a channel to the source in init bound to this
   session.

The rule check in `agentd` (`agentproto::policy`) is the **first** layer;
the second is that without a capability the operation fails even with a
bug in `agentd` or a forged call.

## Rights and the workspace

<figure class="diagram">
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Deciding on a tool call">
<defs><marker id="ag2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="ag2a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="110" width="150" height="70" rx="6"/><text class="t b" x="85" y="138" text-anchor="middle">model wants</text><text class="m" x="85" y="158" text-anchor="middle">tool_use</text>
<rect class="acc" x="200" y="100" width="170" height="90" rx="6"/><text class="t b" x="285" y="128" text-anchor="middle">check</text><text class="m" x="285" y="148" text-anchor="middle">schema, mode,</text><text class="m" x="285" y="166" text-anchor="middle">path, limits</text>
<rect class="ok" x="420" y="10" width="330" height="70" rx="6"/><text class="t b" x="585" y="36" text-anchor="middle">free</text><text class="m" x="585" y="56" text-anchor="middle">reading (except forbidden paths), workspace writes</text><text class="m" x="585" y="72" text-anchor="middle">session capability, commit per step</text>
<rect class="box" x="420" y="110" width="330" height="70" rx="6"/><text class="t b" x="585" y="136" text-anchor="middle">confirm</text><text class="m" x="585" y="156" text-anchor="middle">writes outside workspace, running, network, settings</text><text class="m" x="585" y="172" text-anchor="middle">preview to the client; the client performs it</text>
<rect class="err" x="420" y="210" width="330" height="70" rx="6"/><text class="t b" x="585" y="236" text-anchor="middle">deny</text><text class="m" x="585" y="256" text-anchor="middle">invalid input, a path that never leaves the PC,</text><text class="m" x="585" y="272" text-anchor="middle">tool not in the mode, a limit, client without UI</text>
<path class="ln" d="M160 145 L198 145" marker-end="url(#ag2)"/>
<path class="ln-acc" d="M370 120 L418 50" marker-end="url(#ag2a)"/>
<path class="ln" d="M370 145 L418 145" marker-end="url(#ag2)"/>
<path class="ln" d="M370 170 L418 240" marker-end="url(#ag2)"/>
<text class="m" x="10" y="230">Every decision and result goes</text><text class="m" x="10" y="248">to the audit log. A tool result</text><text class="m" x="10" y="266">goes back to the model as data</text><text class="m" x="10" y="284">(tool_result), never as an order.</text>
</svg>
<figcaption>Three results of the rules. agentd does not perform a confirmed action - it has no capability for it.</figcaption>
</figure>

| Operation | Default mode | Workspace |
|---|---|---|
| reading files, listing, searching | free (within the user's rights) | free |
| write, new directory, trash, rename | **confirm** with a preview | **free**, commit per step |
| a path from the "never leaves the computer" list | deny | deny |
| running a program (`dev.build`, a command) | confirm | confirm |
| network, settings | confirm | confirm |
| service state, crashes, log | free (read-only) | free |

`agentd` sends a **confirmation** to the client as `Confirm { id, action,
preview }`. The action (`agentproto::action::Action`) is an exact
description (`FsWrite { path, content }`, `FsMkdir`, `FsTrash`,
`FsRename`, `Run`). The preview is a diff of old and new contents (`-` /
`+` lines). The client asks the user; on "yes" it **performs the action
itself with its own rights** and returns the result, on "no" it returns
`Declined`. A client without a user interface (e.g. a background task)
always answers `Declined`. How the client performs an action is the shared
function `action::apply` over the `ActionFs` trait.

### Workspace

<figure class="diagram">
<svg viewBox="0 0 760 210" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Session steps in the workspace">
<defs><marker id="ag3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker><marker id="ag3a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<path class="ln" d="M30 80 L730 80"/>
<rect class="acc" x="20" y="55" width="130" height="50" rx="6"/><text class="t b" x="85" y="78" text-anchor="middle">start</text><text class="m" x="85" y="96" text-anchor="middle">commit 0</text>
<rect class="ok" x="200" y="55" width="130" height="50" rx="6"/><text class="t b" x="265" y="78" text-anchor="middle">step 1</text><text class="m" x="265" y="96" text-anchor="middle">commit</text>
<rect class="ok" x="380" y="55" width="130" height="50" rx="6"/><text class="t b" x="445" y="78" text-anchor="middle">step 2</text><text class="m" x="445" y="96" text-anchor="middle">commit</text>
<rect class="ok" x="560" y="55" width="130" height="50" rx="6"/><text class="t b" x="625" y="78" text-anchor="middle">step 3</text><text class="m" x="625" y="96" text-anchor="middle">commit</text>
<path class="ln-acc dash" d="M625 110 C 600 180, 120 180, 85 112" marker-end="url(#ag3a)"/><text class="m" x="250" y="170">revert the session: workspace as at the start (+ commit "reverted")</text>
<path class="ln dash" d="M445 50 C 430 15, 285 15, 268 52" marker-end="url(#ag3)"/><text class="m" x="300" y="22">back to step 1</text>
<text class="m" x="20" y="200">Difference between steps = DIFF between commit snapshots. Nothing is deleted permanently: overwritten files stay in snapshots.</text>
</svg>
<figcaption>Every agent step in the workspace is an FS commit; reverting restores contents from the snapshot of the start or of a chosen step.</figcaption>
</figure>

- The user assigns the workspace when opening the session
  (`ai --prostor=~/project`). **It must be a dataset** (subvolume). A
  session sees only its mounts and history (commits, snapshots, diffs) is a
  property of a dataset; for a plain directory the gate would have to mount
  the whole parent dataset for the agent, giving more rights than the user
  assigned. A new path can be created as a new dataset on request
  (`DATASET_CREATE`).
- **Session start**: `COMMIT_VERSION(P, "agent <session>: start")`.
- **Every step** that wrote to the workspace: `COMMIT_VERSION(P, "agent
  <session>: step N: <tools>")`. `agentd` commits after all calls of the
  step are done.
- **Browsing**: `ws.steps` (steps with commits), `ws.diff` (difference
  between steps or against the start). The client sees the same (`Steps`,
  `StepDiff`).
- **Revert** (`Revert { to }`, `to = 0` = the whole session): `agentd`
  computes `DIFF(P, target snapshot, live)` and undoes the changes: added
  files go to the trash, changed and deleted ones come back from the
  snapshot, directories as needed. Then a commit "agent: reverted to step
  N". Nothing is deleted permanently - the reverted state can be reverted
  again.

### Audit log

Every tool call and every model call is written to
`/system/agent/audit/<user>.log` (the FS session of `agentd`; neither the user
nor the agent can write to it). A line holds: sequence, time, session,
step, tool, **input summary** (path, length and hash of contents - not the
contents), decision (`allow`, `confirm-yes`, `confirm-no`, `deny`), result
(error code) and for model calls the provider, model, tokens, price and
**the number of bytes sent to the cloud**. A client reads only its own
user's records (`Audit { from, max }`).

## Shell helper

The `ai` command in the shell opens a session in **shell** mode: read-only
tools and `shell.propose`. A command proposal (`agentsh::validate`) is
**checked by the shlang parser** in notation B and C and converted to the
other notation; an invalid proposal goes back to the model with the parser
report so it can fix it. Commands that delete, move or overwrite are marked
as risky.

```shell-b
ai "which files in ~/Documents are larger than 100 MB"
```

- **A natural-language question** -> an answer and a proposed command
  (`{ prikaz, zapis, druhy_zapis, riskantni, vysvetleni }`). The command
  **does not run by itself** - the user runs (or edits) it; that is the
  confirmation.
- **Explaining an error**: `ai --vysvetli="..."` sends the last error and the
  text from the local error documentation (`shlang::explain`).
- **A multi-step task**: the model proposes a plan (a list of checked
  commands); the shell shows the whole plan as a preview, the user runs the
  steps.
- Shell mode **writes nothing** (it has no write tools), so it needs no
  confirmation dialog.
- More switches: `--rezim=diag|chat|dev`, `--prostor=~/project` (dev mode
  with a workspace), `--kroky`, `--rozdil=N`, `--vrat=N`, `--audit`,
  `--utrata`, `--klic=file`. `shsess` keeps the agent session for the next
  `ai` in the same shell session (same mode and workspace).
- The `ai` command runs inside the evaluation of a line and the shell has
  no confirmation dialog there. Sessions opened from the shell therefore
  **cannot confirm** (`can_confirm = false`): a write outside the
  workspace is refused, inside it goes through. A GUI client will have
  confirmation with a preview.

Wiring: init gives `termd` the write end of the **terminal gate**; `termd`
duplicates it for every `shsess` session. A request is an `OpenSpec` with
a reply channel attached; the gate thread in init adds the identity and
capabilities and answers `ok <number>` with the client channel. A
duplicate of the gate can do nothing more than ask for a session with the
terminal's identity.

> **Note:** until the system has sign-in, the terminal runs with the system
> identity. An FS session with the system identity cannot be minted (the FS
> refuses it), so the terminal gate opens agent sessions as the **default
> user 1000**: the agent reads only what this user may (less than the
> terminal), and the workspace must belong to that user. Once sign-in
> exists, the gate gets the identity of the signed-in user.

## Diagnostics

**diag** mode has read-only tools only: `sys.services`, `sys.crashes`,
`sys.log`, `sys.net` and reading files. The agent proposes a fix as text
(or as a command the user runs); it changes nothing itself.

## Development in the system

**dev** mode: a project workspace, reading and editing code (`fs.*` free in
the workspace), diffs (`ws.diff`), steps and revert. Building and tests are
prepared tools `dev.build` and `dev.test` with effect `exec` (confirmed);
today they return `NOT_SUPPORTED`. Once a compiler runs in the system, it
registers tools with the same names and `agentd` starts offering them.

## Security

> **Serious risk:** the model runs in the cloud. **Everything the agent
> reads goes to the provider** (tool results are part of the
> conversation). Prompt injection in a file, a log or on the web can steer
> the model into reading other files - within the user's rights. Against
> that stand the list of paths that never leave the computer, modes with a
> smaller tool set, the audit and spending limits; **it is not full read
> protection**. For sensitive data use a mode with limited reading
> (`cteni = "prostor"`) or, later, a local model.

### Prompt injection must not extend rights

- Rights are set by the **session capabilities** the gate issued for the
  user. Neither the model nor text in a file can change them: `agentd` has
  no channel to the gate through which it could ask for more.
- Writing outside the workspace goes **only through the client** after
  the user confirms with a preview. The capabilities of `agentd` are
  read-only - even if the rules in `agentd` failed, the FS refuses the
  write (test "writing outside the workspace without confirmation fails").
- Tool results go to the model as `tool_result` (data); the system prompt
  tells the model that file and log contents are untrusted data. That is
  only a help, **not a protection** - we do not rely on the model.
- `agentd` has network access only to `api.anthropic.com:443` (manifest
  `agentd.manifest.toml`, `ask = false`); the v1 agent has no web tool, so
  it cannot send data anywhere but to the provider.
- Shell command proposals never run by themselves; risky ones are marked.

### What is sent to the cloud

| Sent | Not sent |
|---|---|
| the session system prompt and tool descriptions | the API key to anyone but the provider (only in the `x-api-key` header) |
| the user's questions | contents of the paths listed below |
| tool results: contents of files read, listings, service state | the audit log, `agentd` state, keys |
| model proposals and answers (the conversation) | files the agent did not read |

The audit records the number of bytes sent with every model call.

### Paths that never leave the computer

Read tools return `ACCESS_DENIED` ("path must not leave the computer") for
paths matching the patterns (`*` inside a name, `**` any depth):

```text
/system/agent/**
/system/keys/**
**/.ssh/**
**/.gnupg/**
**/*.key
**/*.pem
**/.config/agent/**
**/hesla/**
**/passwords/**
```

The user can extend the list in the session settings (`neopusti`); it
cannot be narrowed. It applies to all providers (including a local model,
for the audit and consistency).

### Spending limits

| Limit | Default | What happens |
|---|---|---|
| output tokens per call (`max_tokens`) | 16,000 | the model stops with `MaxTokens` |
| model calls per question | 24 | the step stops with `OUT_OF_RANGE` |
| session spending | USD 1.00 | the next call is refused up front (`NO_RESOURCES`) |
| per-user daily spending | USD 5.00 | likewise; counter in `/system/agent/spend/<user>` |

Before each call `agentd` computes an **upper estimate** of the price
(input estimated from the text length + `max_tokens` output) and does not
let the call through if it would exceed a limit. After the call it adds the
actual price from `usage`.

### API key

- The user enters the key with `ai --klic=file` (the shell reads the key
  from a file and offers to delete the file; on the command line the key
  would stay in the history). The client sends the key to `agentd` with a
  `SetKey` message.
- `agentd` **encrypts** the key (ChaCha20-Poly1305) with a key derived by
  HKDF-SHA256 from the machine secret and the user number; AEAD associated
  data = version, user, provider. It stores it in
  `/system/agent/keys/<user>.<provider>`. Another user's key cannot be used
  (a different derived key, different associated data).
- The key is **never** in a readable file or in a log: the `Secret` type
  prints only `***` in `Debug`, memory is zeroed after use, the audit does
  not record the key, the client never gets it back.

> **Serious risk:** the machine secret lies on disk in `/system/agent`
> (protected by FS rights, not by hardware). Whoever reads the disk outside
> the system (another OS, a stolen disk; the FS has no data encryption yet)
> can decrypt the key. The fix is a key derived from the password at login
> or a TPM - once they exist.

## Protocols

### Client - agentd (`agentproto::proto`)

Message = a `u8` code + fields (`libs/wire`); an invalid message = an
error, never a panic.

| Client -> agentd | Meaning |
|---|---|
| `Ask { text }` | a question; agentd answers with messages up to `TurnEnd` or `Error` |
| `Confirmed { id, outcome }` | answer to `Confirm`: `Done(text)`, `Declined`, `Failed(code, text)` |
| `Steps`, `StepDiff { step }`, `Revert { to }` | browsing and reverting the workspace |
| `Audit { from, max }`, `Spend` | the user's audit records, spending and limits |
| `SetKey { provider, key }` | store the API key (encrypted) |

| agentd -> client | Meaning |
|---|---|
| `Text { text }` | model text (in parts) |
| `Tool { name, decision, summary }` | notice of a tool call |
| `Confirm { id, action, preview }` | request for confirmation |
| `Proposal { command, style, other, risky, explanation }` | command proposal (shell mode) |
| `TurnEnd { stop, usage, cost, step }` | end of the question |
| `Error { code, message }` | an error (limit, provider, ...) |
| `Steps`, `Diff`, `Reverted`, `AuditPage`, `Spend`, `KeyStored` | answers |

### Gate (init) - agentd

The bootstrap channel of `agentd`: the message `Open { user, mode,
workspace, provider, model, limits, never_leave, script }` with handles
`[client channel, FS read, FS workspace?, system tools?]` and `Provider`
with a tool provider channel. The client gets the other end of its channel
from the gate.

### Tool provider - agentd (`agentproto::registry`)

`Register { tools }` from the service, then `Call { id, tool, input }`
with a session capability handle and the answer `Result { id, is_error,
content }`.

## Tests

On the host (`cargo test -p agentproto -p agentsh`): JSON, schemas, rules,
the Claude codec (body, response, SSE, errors), price list and limits, the
fake model, the stream accumulator, key encryption (binding to the user),
checking a command proposal with the parser.

In QEMU (`cargo xtask test`, suite "agent AI") with the **fake provider**
(a deterministic script of answers and tool calls, no network):

- reading a file with a tool, the result reaches the model,
- **writing outside the workspace without confirmation fails** (the client
  declines -> the file does not change; a direct write through the
  `agentd` capability -> `ACCESS_DENIED`),
- a confirmed write outside the workspace is performed by the client,
- writing in the workspace without confirmation, a commit per step,
- reverting the whole session and going back to a step,
- the audit log contains all calls and decisions,
- a "never leaves the computer" path is not read,
- the spending limit stops further calls,
- the parser checks a shell command proposal,
- system tools (service state) and registering a foreign provider.

CI never calls the real API.

### Manual test with the real API

Manual only, with the user's key:

1. `cargo xtask run`, in the terminal store the key in a file (e.g. copy it
   with osfs to `~/anthropic.key`) and run `ai --klic=~/anthropic.key`
   (the key is encrypted; delete the file).
2. `ai "how much space does ~/Documents take"` - an answer and a proposed
   command.
3. `ai --prostor=~/trial "create notes.txt with an outline"` - a write
   without a question, `ai --kroky`, `ai --vrat` reverts the session.
4. Audit: `ai --audit`; spending: `ai --utrata`.

## Limitations of version 1

- `httpc` returns the whole body, so the Claude response is not streamed
  over the network.
- Kernel logs cannot be read from a service (no syscall); `sys.log` returns
  supervisor events. A log service is a separate task.
- Tasks (`taskd`) and settings have no tools of their own until their
  services are on `main`.
- The terminal has no hidden password input, so the key is entered via a
  file.
- The workspace must be a dataset (see above).
- Provider calls from different sessions go through one lock (one at a
  time).
- For a service tool with a write effect, confirmation only **allows** the
  call; `agentd` then calls the service with the session capability (the
  client does not perform the action).
- Path checks are textual; the FS does not follow symbolic links when
  resolving a path, so a link into a forbidden path is not read.
