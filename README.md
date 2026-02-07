# MicroClaw

> **Note:** This project is under active development. Features may change, and contributions are welcome!

A Rust rewrite of [nanoclaw](https://github.com/gavrielc/nanoclaw/) -- an agentic AI assistant that lives in your Telegram chats. MicroClaw connects Claude to Telegram with full tool execution: run shell commands, read/write/edit files, search codebases, browse the web, schedule tasks, and maintain persistent memory across conversations.

## How it works

```
Telegram message
    |
    v
 Store in SQLite --> Load chat history + memory
                         |
                         v
                   Claude API (with tools)
                         |
                    stop_reason?
                   /            \
              end_turn        tool_use
                 |               |
                 v               v
           Send reply      Execute tool(s)
                              |
                              v
                        Feed results back
                        to Claude (loop)
```

Every message triggers an **agentic loop**: Claude can call tools, inspect the results, call more tools, and reason through multi-step tasks before responding. Up to 25 iterations per request by default.

## Blog post

For a deeper dive into the architecture, design decisions, and what it's like to use MicroClaw in practice, read the full write-up: **[Building MicroClaw: An Agentic AI Assistant in Rust That Lives in Your Telegram Chats](BLOG.md)**

## Features

- **Agentic tool use** -- bash commands, file read/write/edit, glob search, regex grep, persistent memory
- **Session resume** -- full conversation state (including tool interactions) persisted between messages; Claude remembers tool calls across invocations
- **Context compaction** -- when sessions grow too large, older messages are automatically summarized to stay within context limits
- **Sub-agent** -- delegate self-contained sub-tasks to a parallel agent with restricted tools
- **Web search** -- search the web via DuckDuckGo and fetch/parse web pages
- **Scheduled tasks** -- cron-based recurring tasks and one-time scheduled tasks, managed through natural language
- **Mid-conversation messaging** -- the agent can send intermediate messages before its final response
- **Group chat catch-up** -- when mentioned in a group, the bot reads all messages since its last reply (not just the last N)
- **Continuous typing indicator** -- typing indicator stays active for the full duration of processing
- **Persistent memory** -- CLAUDE.md files at global and per-chat scopes, loaded into every request
- **Message splitting** -- long responses are automatically split at newline boundaries to fit Telegram's 4096 char limit

## Tools

| Tool | Description |
|------|-------------|
| `bash` | Execute shell commands with configurable timeout |
| `read_file` | Read files with line numbers, optional offset/limit |
| `write_file` | Create or overwrite files (auto-creates directories) |
| `edit_file` | Find-and-replace editing with uniqueness validation |
| `glob` | Find files by pattern (`**/*.rs`, `src/**/*.ts`) |
| `grep` | Regex search across file contents |
| `read_memory` | Read persistent CLAUDE.md memory (global or per-chat) |
| `write_memory` | Write persistent CLAUDE.md memory |
| `web_search` | Search the web via DuckDuckGo (returns titles, URLs, snippets) |
| `web_fetch` | Fetch a URL and return plain text (HTML stripped, max 20KB) |
| `send_message` | Send a Telegram message mid-conversation (progress updates, multi-part responses) |
| `schedule_task` | Schedule a recurring (cron) or one-time task |
| `list_scheduled_tasks` | List all active/paused tasks for a chat |
| `pause_scheduled_task` | Pause a scheduled task |
| `resume_scheduled_task` | Resume a paused task |
| `cancel_scheduled_task` | Cancel a task permanently |
| `get_task_history` | View execution history for a scheduled task |
| `export_chat` | Export chat history to markdown |
| `sub_agent` | Delegate a sub-task to a parallel agent with restricted tools |

## Memory

MicroClaw maintains persistent memory via `CLAUDE.md` files, inspired by Claude Code's project memory:

```
data/groups/
    CLAUDE.md                 # Global memory (shared across all chats)
    {chat_id}/
        CLAUDE.md             # Per-chat memory
```

Memory is loaded into Claude's system prompt on every request. Claude can read and update memory through tools -- tell it to "remember that I prefer Python" and it will persist across sessions.

## Scheduling

The bot supports scheduled tasks via natural language:

- **Recurring:** "Remind me to check the logs every 30 minutes" -- creates a cron task
- **One-time:** "Remind me at 5pm to call Alice" -- creates a one-shot task

Under the hood, recurring tasks use 6-field cron expressions (sec min hour dom month dow). The scheduler polls every 60 seconds for due tasks, runs the agent loop with the task prompt, and sends results to the originating chat.

Manage tasks with natural language:
```
"List my scheduled tasks"
"Pause task #3"
"Resume task #3"
"Cancel task #3"
```

## Install

### Homebrew (macOS)

```sh
brew tap everettjf/tap
brew install microclaw
```

### From source

```sh
git clone https://github.com/microclaw/microclaw.git
cd microclaw
cargo build --release
cp target/release/microclaw /usr/local/bin/
```

## Setup

### 1. Create a Telegram bot

Talk to [@BotFather](https://t.me/BotFather) on Telegram:
- `/newbot` -- follow the prompts
- Copy the bot token
- Note the bot username (without the `@`)

### 2. Get an Anthropic API key

Get one at [console.anthropic.com](https://console.anthropic.com/).

### 3. Configure

```sh
cp .env.example .env
```

Edit `.env`:

```
TELEGRAM_BOT_TOKEN=123456:ABC-DEF1234...
ANTHROPIC_API_KEY=sk-ant-...
BOT_USERNAME=my_bot
```

### 4. Run

```sh
microclaw start
```

## Configuration

All configuration is via environment variables (or `.env` file):

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `TELEGRAM_BOT_TOKEN` | Yes | -- | Telegram bot token from BotFather |
| `ANTHROPIC_API_KEY` | Yes | -- | Anthropic API key |
| `BOT_USERNAME` | Yes | -- | Bot username (without @) |
| `CLAUDE_MODEL` | No | `claude-sonnet-4-20250514` | Claude model to use |
| `DATA_DIR` | No | `./data` | Directory for SQLite DB and memory files |
| `MAX_TOKENS` | No | `8192` | Max tokens per Claude response |
| `MAX_TOOL_ITERATIONS` | No | `25` | Max tool-use loop iterations per message |
| `MAX_HISTORY_MESSAGES` | No | `50` | Number of recent messages sent as context |
| `MAX_SESSION_MESSAGES` | No | `40` | Message count threshold that triggers context compaction |
| `COMPACT_KEEP_RECENT` | No | `20` | Number of recent messages to keep verbatim during compaction |

## Group chats

In private chats, the bot responds to every message. In groups, it only responds when mentioned with `@bot_username`. All messages in groups are still stored for context.

**Catch-up behavior:** When mentioned in a group, the bot loads all messages since its last reply in that group (instead of just the last N messages). This means it catches up on everything it missed, making group interactions much more contextual.

## Usage examples

**Web search:**
```
You: Search the web for the latest Rust release notes
Bot: [searches DuckDuckGo, returns top results with links]
```

**Web fetch:**
```
You: Fetch https://example.com and summarize it
Bot: [fetches page, strips HTML, summarizes content]
```

**Scheduling:**
```
You: Every morning at 9am, check the weather in Tokyo and send me a summary
Bot: Task #1 scheduled. Next run: 2025-06-15T09:00:00+00:00

[Next morning at 9am, bot automatically sends weather summary]
```

**Mid-conversation messaging:**
```
You: Analyze all log files in /var/log and give me a security report
Bot: [sends "Scanning log files..." as progress update]
Bot: [sends "Found 3 suspicious entries, analyzing..." as progress update]
Bot: [sends final security report]
```

**Coding help:**
```
You: Find all TODO comments in this project and fix them
Bot: [greps for TODOs, reads files, edits them, reports what was done]
```

**Memory:**
```
You: Remember that the production database is on port 5433
Bot: Saved to chat memory.

[Three days later]
You: What port is the prod database on?
Bot: Port 5433.
```

## Architecture

```
src/
    main.rs              # Entry point, CLI
    config.rs            # Environment variable loading
    error.rs             # Error types (thiserror)
    telegram.rs          # Bot handler, agentic tool-use loop, session resume, context compaction, typing indicator
    claude.rs            # Anthropic Messages API client
    db.rs                # SQLite: messages, chats, scheduled_tasks, sessions
    memory.rs            # CLAUDE.md memory system
    scheduler.rs         # Background task scheduler (60s polling loop)
    tools/
        mod.rs           # Tool trait + registry (17 tools)
        bash.rs          # Shell execution
        read_file.rs     # File reading
        write_file.rs    # File writing
        edit_file.rs     # Find/replace editing
        glob.rs          # File pattern matching
        grep.rs          # Regex content search
        memory.rs        # Memory read/write tools
        web_search.rs    # DuckDuckGo web search
        web_fetch.rs     # URL fetching with HTML stripping
        send_message.rs  # Mid-conversation Telegram messaging
        schedule.rs      # 5 scheduling tools (create/list/pause/resume/cancel)
        sub_agent.rs     # Sub-agent with restricted tool registry
```

Key design decisions:
- **Session resume** persists full message history (including tool blocks) in SQLite; context compaction summarizes old messages to stay within limits
- **Direct API calls** to Anthropic (no SDK wrapper) for full control over the tool-use protocol
- **SQLite with WAL mode** for concurrent read/write from async context
- **Exponential backoff** on 429 rate limits (3 retries)
- **Message splitting** for responses exceeding Telegram's 4096 character limit
- **`Arc<Database>`** shared across tools and scheduler for thread-safe DB access
- **Continuous typing indicator** via a spawned task that sends typing action every 4 seconds

## Documentation

| File | Description |
|------|-------------|
| [README.md](README.md) | This file -- overview, setup, usage |
| [BLOG.md](BLOG.md) | Deep dive blog post about the project |
| [DEVELOP.md](DEVELOP.md) | Developer guide -- architecture, adding tools, debugging |
| [TEST.md](TEST.md) | Manual testing guide for all features |
| [CLAUDE.md](CLAUDE.md) | Project context for AI coding assistants |
| [AGENTS.md](AGENTS.md) | Agent-friendly project reference |

## License

MIT

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=everettjf/MicroClaw&type=Date)](https://star-history.com/#everettjf/MicroClaw&Date)
