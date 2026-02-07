# MicroClaw Testing Guide

This document describes how to manually test every feature of MicroClaw. Since MicroClaw is a Telegram bot with external dependencies (Anthropic API, Telegram API, DuckDuckGo), testing is done through live interaction.

## Prerequisites

1. A working `.env` file with valid credentials
2. `cargo build` succeeds with zero errors
3. Bot is running: `cargo run -- start`
4. A Telegram account to send messages
5. (For group tests) A Telegram group with the bot added as a member

---

## 1. Basic Startup

**Test:** The bot starts without errors.

```sh
cargo run -- start
```

**Expected:** Logs show:
```
Starting MicroClaw bot...
Database initialized
Memory manager initialized
Scheduler started
```

No panics, no errors.

---

## 2. Help Command

**Test:** The CLI help displays correctly.

```sh
cargo run -- help
```

**Expected:** Output includes the features list (web search, scheduled tasks, etc.) and all environment variable documentation.

---

## 3. Private Chat -- Basic Response

**Test:** Send a simple message in a private chat with the bot.

```
You: Hello, what can you do?
```

**Expected:** The bot responds with a description of its capabilities. The typing indicator should appear while processing.

---

## 4. Typing Indicator (Continuous)

**Test:** Send a message that requires tool use (takes a few seconds to process).

```
You: List all files in the current directory
```

**Expected:** The "typing..." indicator stays visible continuously until the response arrives, not just a single flash. It should refresh every ~4 seconds.

---

## 5. Tool Execution -- Bash

**Test:** Ask the bot to run a shell command.

```
You: Run `echo hello world` in bash
```

**Expected:** The bot executes the command and returns "hello world".

---

## 6. Tool Execution -- File Operations

**Test:** Ask the bot to create, read, and edit a file.

```
You: Create a file called /tmp/microclaw_test.txt with the content "hello"
You: Read /tmp/microclaw_test.txt
You: Change "hello" to "goodbye" in /tmp/microclaw_test.txt
You: Read /tmp/microclaw_test.txt again to confirm
```

**Expected:** Each operation succeeds. Final file content is "goodbye".

---

## 7. Tool Execution -- Search (glob + grep)

**Test:** Ask the bot to search for files and content.

```
You: Find all .rs files in the project
You: Search for "fn main" in the source code
```

**Expected:** Returns matching file paths and content with line numbers.

---

## 8. Memory (Read/Write)

**Test:** Test persistent memory across messages.

```
You: Remember that my favorite language is Rust
```

Wait a moment, then send:

```
You: What is my favorite language?
```

**Expected:** The bot saves to CLAUDE.md and recalls "Rust" from memory in the second message.

**Verify:** Check that `data/groups/{chat_id}/CLAUDE.md` exists and contains the memory.

---

## 9. Web Search

**Test:** Ask the bot to search the web.

```
You: Search the web for "Rust programming language"
```

**Expected:** The bot uses the `web_search` tool, returns results with titles, URLs, and snippets from DuckDuckGo. Output should show numbered results.

---

## 10. Web Fetch

**Test:** Ask the bot to fetch a web page.

```
You: Fetch https://example.com and tell me what it says
```

**Expected:** The bot fetches the page, strips HTML tags, and returns the plain text content. The response should mention "Example Domain" (the content of example.com).

---

## 11. Send Message (Mid-Conversation)

**Test:** Ask the bot to send an intermediate message before its final response.

```
You: Send me a progress update saying "Working on it..." and then tell me the current date
```

**Expected:** You receive TWO messages:
1. "Working on it..." (sent via the `send_message` tool)
2. The final response with the current date

---

## 12. Send Message -- Empty Final Response

**Test:** Ask the bot to only use send_message without a final text response.

```
You: Use the send_message tool to tell me "Hello from send_message!" and don't say anything else
```

**Expected:** You receive exactly one message: "Hello from send_message!". No "(no response)" or empty message should be sent.

---

## 13. Schedule Task -- Create (Cron)

**Test:** Schedule a recurring task.

```
You: Schedule a task to say "Hello, this is your 5-minute reminder!" every 5 minutes
```

**Expected:** The bot creates a scheduled task and responds with something like "Task #1 scheduled. Next run: [timestamp]". The cron expression should be `0 */5 * * * *`.

---

## 14. Schedule Task -- Create (One-Time)

**Test:** Schedule a one-time task.

```
You: Schedule a one-time task to say "Time's up!" at 2025-12-31T23:59:00+00:00
```

**Expected:** The bot creates a task with `schedule_type: "once"` and confirms with the task ID.

---

## 15. List Scheduled Tasks

**Test:** List all tasks for this chat.

```
You: List my scheduled tasks
```

**Expected:** Shows all active/paused tasks with their IDs, status, prompts, schedule expressions, and next run times.

---

## 16. Pause Scheduled Task

**Test:** Pause a running task.

```
You: Pause task #1
```

**Expected:** The bot confirms "Task #1 paused." Listing tasks again should show status as "paused".

---

## 17. Resume Scheduled Task

**Test:** Resume a paused task.

```
You: Resume task #1
```

**Expected:** The bot confirms "Task #1 resumed." Listing tasks again should show status as "active".

---

## 18. Cancel Scheduled Task

**Test:** Cancel a task permanently.

```
You: Cancel task #1
```

**Expected:** The bot confirms "Task #1 cancelled." The task should no longer appear in the active list.

---

## 19. Scheduler Execution

**Test:** Verify the scheduler actually runs due tasks.

1. Schedule a task to run every 1 minute:
   ```
   You: Schedule a task to say "Ping! The scheduler works." every minute
   ```
2. Wait 60-90 seconds.

**Expected:** You receive a message "Ping! The scheduler works." (or the agent's response to that prompt) automatically, without sending any new message.

---

## 20. Group Chat -- Message Storage

**Test:** In a group chat, send messages without mentioning the bot.

```
Alice: Hey everyone
Bob: What's up?
Charlie: Working on the project
```

**Expected:** No bot response (correct -- bot only responds to @mentions in groups). But messages are stored in the database.

---

## 21. Group Chat -- @mention Response

**Test:** Mention the bot in a group chat.

```
You: @botusername what files are in the current directory?
```

**Expected:** The bot responds to the mention and uses tools as needed.

---

## 22. Group Chat -- Catch-Up

**Test:** Verify the bot sees all messages since its last reply.

1. In a group chat, have multiple users send messages (or send several yourself).
2. Then mention the bot:
   ```
   Alice: I changed the database port to 5433
   Bob: And I updated the config file
   Charlie: @botusername summarize what happened
   ```

**Expected:** The bot's response references ALL messages since its last response in the group (Alice's port change, Bob's config update), not just the last N messages. This tests the `get_messages_since_last_bot_response` catch-up query.

---

## 23. Long Responses -- Message Splitting

**Test:** Ask for a long response that exceeds 4096 characters.

```
You: Write a 5000-character essay about the history of computing
```

**Expected:** The response is split across multiple Telegram messages at newline boundaries. No truncation, no errors.

---

## 24. Error Handling

**Test:** Trigger an error condition.

```
You: Read the file /nonexistent/path/to/file.txt
```

**Expected:** The bot reports the error gracefully (file not found) rather than crashing.

---

## 25. Session Resume

**Test:** Verify that tool interactions persist across messages.

1. Ask the bot to do something involving tools:
   ```
   You: Create a file /tmp/session_test.txt with "hello"
   ```
2. Wait for the response, then send a follow-up:
   ```
   You: What did you just create?
   ```

**Expected:** The bot remembers the tool interaction (file creation) from the previous message, even though it was a separate invocation. It should reference the file without needing to re-read it.

---

## 26. Session Reset (/reset)

**Test:** Verify that `/reset` clears session state.

1. Have a conversation with tool use.
2. Send `/reset`.
3. Ask a follow-up about the previous conversation.

```
You: /reset
Bot: Session cleared.
You: What did we just talk about?
```

**Expected:** After `/reset`, the bot should not remember tool interactions from the previous session. It falls back to DB message history (text only, no tool blocks).

---

## 27. Context Compaction

**Test:** Verify sessions don't grow unbounded.

1. Have a long conversation (40+ back-and-forth messages) with the bot.
2. Continue chatting.

**Expected:** The bot continues to function normally even after many messages. Earlier conversation context is summarized rather than lost. You can verify by asking about something discussed early in the conversation.

**Note:** Set `MAX_SESSION_MESSAGES=10` and `COMPACT_KEEP_RECENT=5` in `.env` for easier testing.

---

## 28. Sub-Agent

**Test:** Verify the sub_agent tool works.

```
You: Use the sub_agent tool to find all .rs files in this project and count them
```

**Expected:** The bot delegates to a sub-agent, which uses glob/bash to find the files. The result is returned in the main conversation. Check logs for "Sub-agent starting task" and "Sub-agent executing tool" messages.

---

## 29. Sub-Agent Restriction

**Test:** Verify the sub-agent cannot use restricted tools.

```
You: Use the sub_agent tool to schedule a task that runs every minute
```

**Expected:** The sub-agent should not have access to scheduling tools. It should report that it cannot schedule tasks or that the tool is not available.

---

## 30. Regression -- Multiple Tool Iterations (unchanged from #25)

**Test:** Ask for a task that requires multiple tool calls.

```
You: Find all TODO comments in this project's source code, then create a summary file at /tmp/todos.txt
```

**Expected:** The bot uses grep to search, reads relevant files, writes the summary -- multiple tool iterations in one request.

---

## Database Verification

After running tests, you can verify the database directly:

```sh
sqlite3 data/microclaw.db

-- Check messages are stored
SELECT COUNT(*) FROM messages;

-- Check scheduled tasks
SELECT * FROM scheduled_tasks;

-- Check chat records
SELECT * FROM chats;

-- Check sessions
SELECT chat_id, length(messages_json), updated_at FROM sessions;
```

---

## Cleanup

After testing:

```sh
# Remove test files
rm -f /tmp/microclaw_test.txt /tmp/todos.txt

# Cancel any remaining scheduled tasks via the bot, or:
sqlite3 data/microclaw.db "UPDATE scheduled_tasks SET status='cancelled' WHERE status='active';"
```
