# Channel Gap vs ZeroClaw (2026-02-22)

## Source Baseline
- ZeroClaw reference: `https://github.com/zeroclaw-labs/zeroclaw`
- ZeroClaw channel modules seen in source (`/tmp/zeroclaw/src/channels`): Telegram, Discord, Slack, Matrix, WhatsApp, iMessage, IRC, Email, Nostr, plus Mattermost/Signal/Lark/DingTalk/QQ/etc.

## MicroClaw Current Status
- Already supported before this patch: Telegram, Discord, Slack, Matrix, Feishu/Lark, IRC, Web
- Added in this patch: WhatsApp (Cloud API webhook mode), iMessage, Email, Nostr, Signal, DingTalk, QQ
- Remaining parity gaps from user-requested set: none

## Why Not All-In-One Port
- ZeroClaw channel implementations are deeply coupled to its own runtime abstractions and message pipeline.
- Direct copy is high-risk (large code transplant, dependency and behavior drift).
- Safe path is incremental integration per channel with MicroClaw-native adapter contracts and storage semantics.

## Implementation Notes For Newly Added Channels
1. iMessage:
   - Outbound implemented via `osascript` (macOS Messages.app bridge)
   - Inbound not native yet (requires bridge script/daemon)
2. Email:
   - Outbound implemented via local `sendmail`
   - Inbound implemented via webhook (`/email/webhook`)
3. Nostr:
   - Inbound implemented via webhook (`/nostr/events`)
   - Outbound implemented via configurable publish command bridge (`publish_command`)

## Acceptance Criteria for Each Channel PR
- Channel adapter registered in `ChannelRegistry`
- Inbound messages persisted with unique `channel + external_chat_id` identity
- Outbound `send_message` path works via `deliver_and_store_bot_message`
- Setup wizard fields + config validation + docs
- Basic smoke test (mock or sandboxed integration)
