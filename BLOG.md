# Building MicroClaw: An Agentic AI Assistant in Rust That Lives in Your Telegram Chats

What if your Telegram chat was a terminal? Not a dumbed-down chatbot that responds with canned text, but an actual AI agent that can run commands on your server, edit your files, search your codebase, browse the web, schedule recurring tasks, and remember what you told it three weeks ago?

That's MicroClaw -- a Rust implementation of the agentic AI-in-a-chat pattern, connecting Claude to Telegram with full tool execution. It started as a rewrite of [nanoclaw](https://github.com/gavrielc/nanoclaw/), a TypeScript project that does the same thing over WhatsApp, but rebuilt from scratch in Rust with a focus on simplicity and additional capabilities.

## The idea

Most AI chatbot integrations are thin wrappers. They take a message, forward it to an API, and return the response. One turn, no state, no agency.

MicroClaw is different. When you send it a message, it enters an **agentic loop**: Claude receives your message along with a set of tools, decides whether it needs to take action, executes tools if necessary, reads the results, decides if it needs to do more, and keeps going -- up to 25 iterations -- before finally composing a response.

Ask it to "find all TODO comments in the project and create a summary" and it will:

1. Run a grep search across your codebase
2. Read the matching files for context
3. Synthesize the results into a structured summary
4. Respond in your Telegram chat

All of that happens in a single message exchange. You send one message, you get back one answer. The multi-step reasoning happens behind the scenes.

## Why Rust?

The original nanoclaw is TypeScript. It works. But I wanted something I could deploy as a single static binary with no runtime dependencies. `cargo build --release` gives you one file. Copy it to a server, set three environment variables, and it runs.

Rust also turned out to be a surprisingly good fit for this kind of project:

- **Enums for the API protocol.** Claude's content blocks come in three flavors: text, tool_use, and tool_result. Rust's tagged enums with serde map to this perfectly. No stringly-typed checks, no runtime type confusion.
- **Trait objects for tools.** Each tool implements a `Tool` trait. The registry holds `Vec<Box<dyn Tool>>`. Adding a new tool is four lines of wiring.
- **Async without drama.** Tokio + reqwest + teloxide all play nicely together. The entire bot is a single async binary.
- **Shared state is explicit.** `Arc<Database>` makes it clear exactly which components share database access. The scheduler, the tools, and the message handler all hold their own arc -- no hidden global state.

## Architecture

The system has eight modules, and the data flows in one direction:

```
Telegram message
       |
       v
    SQLite (store message, load history)
       |
       v
    System prompt (inject memories + chat_id)
       |
       v
    Claude API -----> tool_use? -----> Execute tool
       ^                                    |
       |                                    |
       +---- feed result back --------------+
       |
       v
    end_turn? --> Send response to Telegram
```

Meanwhile, running in the background:

```
Scheduler (every 60s)
       |
       v
    Query due tasks from SQLite
       |
       v
    For each: run agentic loop --> send result to chat
```

### The agentic loop

The heart of MicroClaw lives in one function: `process_with_claude`. Here's what it does:

1. Load history from SQLite. In private chats, this is the last N messages. In groups, it's everything since the bot's last reply -- the catch-up mechanism that makes group interactions feel natural.
2. Read any saved memories (global and per-chat CLAUDE.md files) and inject them into the system prompt.
3. If there's an override prompt (from the scheduler), append it as a user message.
4. Convert the message history into Claude's message format.
5. Enter the loop: call the Claude API. If Claude responds with `stop_reason: "tool_use"`, execute the requested tools, append the results as a `tool_result` message, and call Claude again. If Claude responds with `stop_reason: "end_turn"`, extract the text and return it.

The loop has a safety cap (default 25 iterations). In practice, most interactions use 0-3 tool calls. Complex tasks like "refactor this file" might use 5-10.

### The typing indicator

A small quality-of-life feature that makes a big difference: the typing indicator stays active for the entire duration of processing. A spawned Tokio task sends `ChatAction::Typing` every 4 seconds. When the response is ready, the task is aborted.

Without this, the typing indicator would flash once when the message is received and then disappear, even if Claude is midway through a 10-tool chain. With it, the user always knows the bot is working.

### Tools

MicroClaw ships with sixteen tools across five categories:

**File system (6 tools):**

**`bash`** -- The power tool. Runs arbitrary shell commands with a configurable timeout. Claude uses this for everything from `ls` to `git status` to `python script.py`. Output is captured (stdout + stderr) and truncated at 30KB.

**`read_file`** -- Reads files with line numbers, like `cat -n`. Supports offset and limit for large files. Claude uses this before editing to understand what it's working with.

**`write_file`** -- Creates or overwrites files. Automatically creates parent directories. Claude uses this for generating new files.

**`edit_file`** -- The surgical tool. Takes a file path, an exact string to find, and a replacement string. The old string must appear exactly once in the file (enforced), preventing ambiguous edits. This is how Claude modifies existing code without rewriting entire files.

**`glob`** -- Finds files matching a pattern. `**/*.rs`, `src/components/*.tsx`, etc. Claude uses this to explore project structure.

**`grep`** -- Searches file contents with regex. Recursively walks directories, skips hidden folders and `node_modules`/`target`, and returns matches with file paths and line numbers.

**Memory (2 tools):**

**`read_memory` / `write_memory`** -- The persistence layer. Claude can save notes to CLAUDE.md files at two scopes: global (shared across all chats) and per-chat. These files are loaded into every request's system prompt, giving Claude long-term memory that survives restarts.

**Web (2 tools):**

**`web_search`** -- Searches the web via DuckDuckGo's HTML interface. The tool sends a GET request to `html.duckduckgo.com/html/?q=...`, then parses the results using regex to extract titles, URLs, and snippets. Returns up to 8 results. No API key needed.

**`web_fetch`** -- Fetches any URL and returns the plain text content. HTML tags are stripped via regex, whitespace is collapsed, and the result is truncated at 20KB. Claude uses this to read documentation, articles, or any web page the user points to.

**Messaging (1 tool):**

**`send_message`** -- Sends a Telegram message mid-conversation. This is the tool that lets Claude send progress updates: "Scanning your log files...", "Found 3 issues, analyzing...", then a final report. The chat_id is provided in the system prompt, so Claude knows where to send. If Claude only communicates via this tool, the final response can be empty -- MicroClaw handles this gracefully by not sending an empty message.

**Scheduling (5 tools):**

**`schedule_task`** -- Creates a recurring (cron) or one-time scheduled task. The user says "remind me every morning at 9am to check the build status" and Claude converts that to a cron expression, stores it in SQLite, and confirms. The scheduler picks it up on the next poll.

**`list_scheduled_tasks`** -- Lists all active and paused tasks for a chat.

**`pause_scheduled_task` / `resume_scheduled_task`** -- Pause and resume tasks. A paused task stays in the database but the scheduler skips it.

**`cancel_scheduled_task`** -- Permanently cancels a task by setting its status to "cancelled".

Adding a new tool means implementing three methods: `name()`, `definition()` (returns a JSON Schema), and `execute()` (does the work). If the tool needs shared state like the Telegram bot or database, the constructor accepts it. Register it in the tool list, and Claude automatically discovers it.

### Memory

This is the feature that makes MicroClaw feel different from a stateless chatbot.

Tell it "remember that I'm working on project Atlas and the deploy target is staging.example.com" and it will write that to a CLAUDE.md file. Next time you message it -- hours, days, or weeks later -- that context is right there in the system prompt.

The memory system has two scopes:

- **Global memory** (`data/groups/CLAUDE.md`) -- things that matter across all conversations. Your name, preferences, common projects.
- **Chat memory** (`data/groups/{chat_id}/CLAUDE.md`) -- things specific to one conversation. Project context, ongoing tasks, decisions made.

Claude manages its own memory. You don't manually edit these files (though you can). You just talk to it naturally: "remember this", "forget about that", "what do you know about my setup?" It reads and writes the memory files through the same tool system it uses for everything else.

### The scheduler

The scheduler is a background Tokio task that wakes up every 60 seconds and checks for due tasks. When it finds one:

1. It calls the same `process_with_claude` function that handles regular messages, but with the task's prompt as an override.
2. The agent runs its full tool loop -- a scheduled task can use web search, bash, file operations, anything.
3. The result is sent to the originating chat.
4. For cron tasks, the next run time is computed from the cron expression. For one-shot tasks, the status is set to "completed".

This means scheduled tasks are fully agentic. You can say "every morning at 8am, check the weather in Tokyo and give me a summary" and the bot will actually search the web, fetch a weather page, and compose a summary -- not just echo a stored message.

### Group chat catch-up

In groups, the bot only responds when @mentioned. But it stores every message. When mentioned, instead of loading the last N messages like in private chats, it loads all messages since its last reply in that group.

This means if 50 messages flew by in a group discussion and someone tags the bot asking "summarize what just happened", the bot sees all 50 messages, not just the most recent ones that fit in the context window. The catch-up query (`get_messages_since_last_bot_response`) handles this with a simple SQL query that finds the timestamp of the last bot message and loads everything since.

### Database

SQLite stores three things: chat metadata, messages, and scheduled tasks. Every message that passes through a chat (whether the bot responds or not) gets stored. When Claude processes a new message, it loads the relevant history as conversation context.

WAL mode keeps reads and writes from blocking each other, which matters when the bot is handling messages across multiple chats concurrently while the scheduler is also querying for due tasks.

The database is wrapped in `Mutex<Connection>` inside a `Database` struct, shared as `Arc<Database>` across the message handler, all scheduling tools, and the scheduler itself.

## What it looks like in practice

**Private chat -- coding help:**

> You: Can you check if there are any syntax errors in main.py?
>
> MicroClaw: *reads the file, runs `python -m py_compile main.py`*
> Found an issue on line 47 -- there's a missing colon after the `if` statement...

**Private chat -- memory:**

> You: Remember that the production database is on port 5433, not the default.
>
> MicroClaw: Noted. I've saved that to our chat memory.
>
> *(three days later)*
>
> You: What port is the prod database on?
>
> MicroClaw: The production database is on port 5433.

**Web search:**

> You: Search the web for the latest Rust release notes
>
> MicroClaw: *searches DuckDuckGo, fetches the results page*
> Here are the latest Rust release notes:
> 1. Rust 1.83 release notes -- [link] -- key highlights include...

**Scheduling:**

> You: Every morning at 9am, check the tech news and give me a summary
>
> MicroClaw: Task #1 scheduled (cron: 0 0 9 * * *). Next run: tomorrow at 09:00 UTC.
>
> *(next morning at 9am, automatically)*
>
> MicroClaw: Here's your morning tech news summary: ...

**Mid-conversation messaging:**

> You: Analyze all the log files and find security issues
>
> MicroClaw: *sends "Scanning 47 log files..."*
> MicroClaw: *sends "Found 3 suspicious entries, analyzing patterns..."*
> MicroClaw: Here's the full security report: ...

**Group chat -- on-demand help:**

> Alice: The deploy failed again
> Bob: Check the CI logs
> Alice: @microclaw can you look at deploy.yml and see what's wrong?
>
> MicroClaw: *reads deploy.yml, spots the issue*
> Line 23 references `node-version: 18` but your package.json engines field requires >=20...

## Setup

Getting started takes about two minutes:

```sh
git clone https://github.com/user/microclaw
cd microclaw
cp .env.example .env
```

Edit `.env` with three required values:
- `TELEGRAM_BOT_TOKEN` -- get one from [@BotFather](https://t.me/BotFather)
- `ANTHROPIC_API_KEY` -- from [console.anthropic.com](https://console.anthropic.com/)
- `BOT_USERNAME` -- your bot's username (without the @)

Then:

```sh
cargo run -- start
```

Or build a release binary:

```sh
cargo build --release
./target/release/microclaw start
```

The binary is self-contained. No database server to install (SQLite is bundled), no external dependencies to configure. It creates its `data/` directory on first run, starts the scheduler automatically, and begins listening for messages.

Optional configuration (via environment variables):

| Variable | Default | What it does |
|----------|---------|--------------|
| `CLAUDE_MODEL` | `claude-sonnet-4-20250514` | Which Claude model to use |
| `DATA_DIR` | `./data` | Where to store the database and memory files |
| `MAX_TOKENS` | `8192` | Max tokens per Claude response |
| `MAX_TOOL_ITERATIONS` | `25` | Safety cap on tool loops per message |
| `MAX_HISTORY_MESSAGES` | `50` | How many messages to include as context |

## What's different from the original nanoclaw

MicroClaw started as a port but has grown beyond feature parity:

| Feature | nanoclaw (TS/WhatsApp) | MicroClaw (Rust/Telegram) |
|---------|----------------------|--------------------------|
| Platform | WhatsApp | Telegram |
| Language | TypeScript | Rust |
| Deployment | Node.js runtime | Single binary |
| Tools | Similar core set | 16 tools (8 original + 8 new) |
| Web search | -- | DuckDuckGo search + URL fetch |
| Scheduling | Cron reminders | Full agentic scheduled tasks |
| Mid-message sending | -- | send_message tool |
| Group catch-up | -- | Loads all messages since last reply |
| Typing indicator | Single flash | Continuous (every 4s) |

## Limitations

Things it doesn't do (yet):

- **No image/voice/document support.** Text messages only.
- **No streaming.** The bot sends a typing indicator while Claude works, but the response arrives all at once.
- **No permission model.** Anyone who can message the bot can execute bash commands. Deploy this on a locked-down machine or behind a user allowlist.
- **Single-threaded tool execution.** Tools within a single Claude turn run sequentially. Parallel tool execution is a potential optimization.
- **Scheduler granularity.** The scheduler polls every 60 seconds, so tasks can be up to 60 seconds late.

## The numbers

~2,600 lines of Rust across 19 source files. 16 tools. 3 database tables. 1 binary. Zero runtime dependencies.

## Final thoughts

The interesting thing about building MicroClaw wasn't the Rust or the Telegram integration -- those are just plumbing. The interesting thing is how little code it takes to go from "chatbot that echoes API responses" to "agent that can actually do things."

The difference is one loop and a tool registry. That's it. The agentic loop checks `stop_reason`, executes tools if needed, and feeds results back. The tool registry maps names to implementations. Everything else -- the database, the memory system, the scheduler, the message handling -- is supporting infrastructure.

The scheduling system is a good example. It's not a separate subsystem with its own AI integration. It's just a timer that calls the same `process_with_claude` function that handles regular messages, but with a stored prompt instead of a live user message. The agent loop, the tool system, the memory -- it all reuses the same infrastructure.

The pattern is general. Swap Telegram for Slack, Discord, or a web UI. Swap the tools for whatever your domain needs. The core loop stays the same: receive message, call LLM with tools, execute tools in a loop, return response.

~2,600 lines of Rust. No frameworks. One binary. An AI agent in your pocket.
