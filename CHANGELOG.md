# Changelog

## [0.11.0](https://github.com/Atharva-Kanherkar/bridge-harness/compare/v0.10.0...v0.11.0) (2026-11-28)


### Features

* **account:** sign in to Bridge with WorkOS AuthKit ([#1211](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1211)) ([ee43059](https://github.com/Atharva-Kanherkar/bridge-harness/commit/ee430597cf374011d585cdea8d4d0ff058185927)), closes [#1209](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1209)
* **memory:** a Taste tab to approve, write, trace and delete taste rules ([#1216](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1216)) ([b28dc3c](https://github.com/Atharva-Kanherkar/bridge-harness/commit/b28dc3cc19c13a676557e021d55d6ce03107c7dc))
* **memory:** coding agents get your taste rules on every harness ([#1218](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1218)) ([35dd214](https://github.com/Atharva-Kanherkar/bridge-harness/commit/35dd214df1eb877abf1b650a6421c26c74050bb5))
* **memory:** ledger layers, project scopes, evidence links and outcome scores ([#1210](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1210)) ([5bba6bd](https://github.com/Atharva-Kanherkar/bridge-harness/commit/5bba6bd0bb29441f7a3099c8b67de5bef86f9f1f))
* **release:** build an ad-hoc signed DMG for sharing without a Developer ID ([#1193](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1193)) ([e060978](https://github.com/Atharva-Kanherkar/bridge-harness/commit/e060978ee55aba8526b9afad1a814e7ebe87f26e))
* **reviews:** hide new findings that match ones you keep dropping ([#1214](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1214)) ([4bf9e3f](https://github.com/Atharva-Kanherkar/bridge-harness/commit/4bf9e3fa4f6c4ec533165f466fd5bfaea62004ab))
* **reviews:** the reviewer follows your taste rules ([#1217](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1217)) ([f50500a](https://github.com/Atharva-Kanherkar/bridge-harness/commit/f50500a4147a488d54069f0b4df60b088b209e02))
* **taste:** export and import taste rules as AGENTS.md, CLAUDE.md and BUGBOT.md ([#1220](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1220)) ([1836946](https://github.com/Atharva-Kanherkar/bridge-harness/commit/18369469e0d206d0dfc88b2ef3de13abe9fa5abf))
* **taste:** learn review rules from your decisions, proposed with evidence ([#1215](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1215)) ([d8998bf](https://github.com/Atharva-Kanherkar/bridge-harness/commit/d8998bfcbf4dbce87bcb97ec51adf79e5b81d6f9))
* **taste:** one signal log of what you keep, edit, drop and fix ([#1212](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1212)) ([ae878f7](https://github.com/Atharva-Kanherkar/bridge-harness/commit/ae878f7dfa1a300231170aa34d967d088e48f175))
* **taste:** outcome scores, auto-pause and expiry for taste rules ([#1219](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1219)) ([ed41125](https://github.com/Atharva-Kanherkar/bridge-harness/commit/ed41125077a3200da10c937b3429dd5e7276d104))
* **taste:** suggest what to do about tool requests you keep answering the same way ([#1221](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1221)) ([bd61c16](https://github.com/Atharva-Kanherkar/bridge-harness/commit/bd61c16b41b9762c08342440a2079e14f9aba7d6))


### Bug Fixes

* **reviews:** anchor findings on the faulty line, not the block's closing brace ([#1195](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1195)) ([b0872b1](https://github.com/Atharva-Kanherkar/bridge-harness/commit/b0872b16aa35d95e8b749718ce1cbd6eb5e72fe2))

## [0.10.0](https://github.com/Atharva-Kanherkar/bridge-harness/compare/v0.9.0...v0.10.0) (2026-11-27)


### Features

* **accounts:** use more than one subscription per coding agent ([#868](https://github.com/Atharva-Kanherkar/bridge-harness/issues/868)) ([8c9187b](https://github.com/Atharva-Kanherkar/bridge-harness/commit/8c9187b96b2d43ba8cc197aa1273e769092e93a3))
* add Pi coding-agent integration ([#828](https://github.com/Atharva-Kanherkar/bridge-harness/issues/828)) ([8ba7a96](https://github.com/Atharva-Kanherkar/bridge-harness/commit/8ba7a966ddec968770ac99eca920de28a1a5e6ea))
* **apps:** app events into memory, scoped by each app's manifest ([#1009](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1009)) ([a834c58](https://github.com/Atharva-Kanherkar/bridge-harness/commit/a834c58c17be9b1f68d525efde3f8579acbd413c))
* **apps:** app manifests in core, with tool gating and core-owned pins and badges ([#1000](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1000)) ([8bbb82f](https://github.com/Atharva-Kanherkar/bridge-harness/commit/8bbb82fb8c6b366e06fc37b780f0cbf7e0319dc1))
* **apps:** My apps in the Store and launcher, with Surface opens ([#1070](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1070)) ([1627a9f](https://github.com/Atharva-Kanherkar/bridge-harness/commit/1627a9f6b46224255d3667ab8549c3f13d5cf7e6))
* **apps:** one sidebar, a ⌘K launcher, per-chat dock panes and the Store ([#995](https://github.com/Atharva-Kanherkar/bridge-harness/issues/995)) ([c8282f1](https://github.com/Atharva-Kanherkar/bridge-harness/commit/c8282f1c5b867d5f6d8b7dc68085698d00022318))
* **apps:** open locked personal apps and approve every write exactly ([#1063](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1063)) ([feb159f](https://github.com/Atharva-Kanherkar/bridge-harness/commit/feb159fbe91033e28a6cb09efb23c9c8d7353d53))
* **apps:** pin and unpin apps from the launcher grid and the sidebar ([#1017](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1017)) ([3b6fdc1](https://github.com/Atharva-Kanherkar/bridge-harness/commit/3b6fdc1d1e05c6b3c5165edaeccfb5ed734a60f1))
* **apps:** run OpenCode connections through a gated courier ([#1067](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1067)) ([2f061f8](https://github.com/Atharva-Kanherkar/bridge-harness/commit/2f061f84674b7a610a6c6b9af56cd7c7d6698cd5))
* **apps:** run personal apps from every agent through one router ([#1049](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1049)) ([9195e61](https://github.com/Atharva-Kanherkar/bridge-harness/commit/9195e61e2d911c607995c3317ade46c470ad8598))
* **apps:** store personal apps as locked manifest versions ([#1029](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1029)) ([f3beda0](https://github.com/Atharva-Kanherkar/bridge-harness/commit/f3beda089dedd759df7af347af4dcfe089f42786))
* **attention:** agents can ask for the person ([#937](https://github.com/Atharva-Kanherkar/bridge-harness/issues/937)) ([785d1e4](https://github.com/Atharva-Kanherkar/bridge-harness/commit/785d1e48e7e29b20f93f23928d7f31d6fcc4b760))
* **attention:** browser notifications for the web client ([#936](https://github.com/Atharva-Kanherkar/bridge-harness/issues/936)) ([294b0b5](https://github.com/Atharva-Kanherkar/bridge-harness/commit/294b0b531ca58c134119f39b73fc7c3cf3b36d31))
* **attention:** core attention ledger and read methods ([#915](https://github.com/Atharva-Kanherkar/bridge-harness/issues/915)) ([020666c](https://github.com/Atharva-Kanherkar/bridge-harness/commit/020666c130497b858abfe433061e4eeaf8aa4b3c))
* **attention:** desktop notifications read the attention ledger ([#916](https://github.com/Atharva-Kanherkar/bridge-harness/issues/916)) ([617c3bc](https://github.com/Atharva-Kanherkar/bridge-harness/commit/617c3bce39e59abad2a34f69cf2bee7bdf86a241))
* **attention:** escalate stranded worker notices, with Deliver now ([#917](https://github.com/Atharva-Kanherkar/bridge-harness/issues/917)) ([afed69d](https://github.com/Atharva-Kanherkar/bridge-harness/commit/afed69dca29b1ccc827dc50c504ec643da453728))
* **attention:** macOS banners open the chat and answer approvals ([#939](https://github.com/Atharva-Kanherkar/bridge-harness/issues/939)) ([95e1ce2](https://github.com/Atharva-Kanherkar/bridge-harness/commit/95e1ce295e47feb281e201ae1c116af3d6f2bbf9))
* **attention:** notification sounds, per-kind settings and quiet hours ([#934](https://github.com/Atharva-Kanherkar/bridge-harness/issues/934)) ([59579c9](https://github.com/Atharva-Kanherkar/bridge-harness/commit/59579c9148964ca80aac5f2dab058e32db068636))
* **attention:** one client posts OS banners ([#935](https://github.com/Atharva-Kanherkar/bridge-harness/issues/935)) ([472df49](https://github.com/Atharva-Kanherkar/bridge-harness/commit/472df4918c1a67e99941de25376ede5d4808e1ab))
* **attention:** one notification policy for toasts and banners ([#931](https://github.com/Atharva-Kanherkar/bridge-harness/issues/931)) ([e9641ea](https://github.com/Atharva-Kanherkar/bridge-harness/commit/e9641eaff31f87396def50aa3d20bcca866f0b7f))
* **attention:** one toast card and stack for every kind ([#932](https://github.com/Atharva-Kanherkar/bridge-harness/issues/932)) ([d883905](https://github.com/Atharva-Kanherkar/bridge-harness/commit/d883905fab9aa6e73ea6c2214f3ed658029c602d))
* **attention:** sessions/wait and bridge exec --wait ([#938](https://github.com/Atharva-Kanherkar/bridge-harness/issues/938)) ([cc71916](https://github.com/Atharva-Kanherkar/bridge-harness/commit/cc7191661af38666e245f784dd131dd6886ed1f4))
* **attention:** unread dots in the sidebar and a Dock badge ([#929](https://github.com/Atharva-Kanherkar/bridge-harness/issues/929)) ([b6fb4dc](https://github.com/Atharva-Kanherkar/bridge-harness/commit/b6fb4dccd449e270f00f4d96e12f5014a3b2ecb3))
* **browser:** plain browser only, throwaway clones switched off ([#1025](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1025)) ([4641519](https://github.com/Atharva-Kanherkar/bridge-harness/commit/4641519724b03575bb3dab2a6d6eb64769e5bf26))
* **browser:** support Safari fresh sign-in in browser copies ([#831](https://github.com/Atharva-Kanherkar/bridge-harness/issues/831)) ([c4d481d](https://github.com/Atharva-Kanherkar/bridge-harness/commit/c4d481d877b0da88f73cfe7e61aa7cc64e6e89a6))
* **changes:** count what the chat changed, not the whole folder ([#1114](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1114)) ([559b305](https://github.com/Atharva-Kanherkar/bridge-harness/commit/559b305a923008c8d80f72cc40ad3db9f98745e9))
* **chat:** a background-work island instead of the activity card ([#957](https://github.com/Atharva-Kanherkar/bridge-harness/issues/957)) ([0c53fe0](https://github.com/Atharva-Kanherkar/bridge-harness/commit/0c53fe0f581d87414865aace4d489a76102c552d))
* **chat:** jump to latest message button ([#900](https://github.com/Atharva-Kanherkar/bridge-harness/issues/900)) ([b80b960](https://github.com/Atharva-Kanherkar/bridge-harness/commit/b80b960ffedded990918cdbe9494c36dc132de02))
* **chat:** reply to an earlier agent message ([#1179](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1179)) ([b5cca64](https://github.com/Atharva-Kanherkar/bridge-harness/commit/b5cca6414cc6db73a13b3f65daf7923a005e6aa2))
* **chat:** retry or edit a failed turn from its error card ([#954](https://github.com/Atharva-Kanherkar/bridge-harness/issues/954)) ([9c13125](https://github.com/Atharva-Kanherkar/bridge-harness/commit/9c131258d88391b720fff8fd259f0bb22181627e))
* **chat:** run shell blocks from chat in the dock terminal ([#1073](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1073)) ([710d932](https://github.com/Atharva-Kanherkar/bridge-harness/commit/710d9326fa3ae23494d565609004298ab4b9296c))
* **chat:** show a transcript-shaped skeleton while a chat opens ([#875](https://github.com/Atharva-Kanherkar/bridge-harness/issues/875)) ([308f046](https://github.com/Atharva-Kanherkar/bridge-harness/commit/308f046cc89d4160a4c95f5e83a7806310ef871f))
* **claude:** support Claude Fable 5.2 ([#942](https://github.com/Atharva-Kanherkar/bridge-harness/issues/942)) ([1746a80](https://github.com/Atharva-Kanherkar/bridge-harness/commit/1746a80f493e5c40fdecabd465a8e66ccacf801e))
* **compaction:** ask the harness to compact a long chat at an idle turn end ([#1086](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1086)) ([9291625](https://github.com/Atharva-Kanherkar/bridge-harness/commit/9291625bedc7c62aaee2ffaf6dbbc24bf395659f))
* **connectors:** send exact Outgoing calls through the harness's MCP client ([#1027](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1027)) ([211aa6f](https://github.com/Atharva-Kanherkar/bridge-harness/commit/211aa6f62445e7c197da782a0100631197e37f8e))
* **context:** add the bridge tokens report and keep workers out of the chat launchers ([#1133](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1133)) ([7cab584](https://github.com/Atharva-Kanherkar/bridge-harness/commit/7cab584bd935023181c156fbe79bbb6f035e116e))
* **context:** record a token estimate per history entry and the window used per request ([#1024](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1024)) ([7ede49c](https://github.com/Atharva-Kanherkar/bridge-harness/commit/7ede49c39a8af73ed8ce9296aec4e61b19d228b0))
* **controls:** make Stop and steering instant on every provider ([#843](https://github.com/Atharva-Kanherkar/bridge-harness/issues/843)) ([d2fa5a7](https://github.com/Atharva-Kanherkar/bridge-harness/commit/d2fa5a70a78dca2c2fd442f420ccd5ee28b8be4e))
* **defaults:** per-project new chat defaults with chat-scoped access ([#844](https://github.com/Atharva-Kanherkar/bridge-harness/issues/844)) ([0f31272](https://github.com/Atharva-Kanherkar/bridge-harness/commit/0f312725db81127e8c5fd245dfe81074b6708fb3))
* **delegation:** bridge-wait holds worker results and delivers them once ([#940](https://github.com/Atharva-Kanherkar/bridge-harness/issues/940)) ([8add2bd](https://github.com/Atharva-Kanherkar/bridge-harness/commit/8add2bd27a03151d3b021c915abba3f30a8afe69))
* draft and preview personal MCP apps ([#1031](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1031)) ([b1bbd8e](https://github.com/Atharva-Kanherkar/bridge-harness/commit/b1bbd8e58a517f225fc0845316a505c887d862e3))
* **github:** merge box, action sheet and the pane's motion system ([#1034](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1034)) ([ed2bc70](https://github.com/Atharva-Kanherkar/bridge-harness/commit/ed2bc709c4fe8b3607b98fdc03f7745049878a51))
* **github:** resolve conversations, inline images and video, and direct commenting in the pane ([#1075](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1075)) ([42ba631](https://github.com/Atharva-Kanherkar/bridge-harness/commit/42ba6311b3a4c381f365492598ff67ac723cea05))
* **history-tools:** chats can search and read the person's other chats ([#1011](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1011)) ([bb668a1](https://github.com/Atharva-Kanherkar/bridge-harness/commit/bb668a129ade364d104c3daabf5b4bb6bde2801a))
* **import:** adopt Conductor's workspaces in place ([#1093](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1093)) ([55a8db6](https://github.com/Atharva-Kanherkar/bridge-harness/commit/55a8db6908b4ee162cd50d0e6f5c09aab1177547))
* **import:** bring Claude Code chats over with tools, thinking, images and subagents ([#1065](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1065)) ([eacb95e](https://github.com/Atharva-Kanherkar/bridge-harness/commit/eacb95e0edf30e13d0311d9d59ebd57e0cfcb96f))
* **import:** bring Orca's worktrees, chats and multi-agent runs into Bridge ([#1127](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1127)) ([867d3a7](https://github.com/Atharva-Kanherkar/bridge-harness/commit/867d3a7a43a5033cf77c0ad8809fa56e8e604105))
* **import:** bring Superset's workspaces, chats and agent configurations into Bridge ([#1131](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1131)) ([f977db6](https://github.com/Atharva-Kanherkar/bridge-harness/commit/f977db60691e23a8c5627aea54116dbeb6938499))
* **import:** bring T3 Code's threads and worktrees into Bridge ([#1097](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1097)) ([584cb04](https://github.com/Atharva-Kanherkar/bridge-harness/commit/584cb0436dee2cb012f081a20ab94cbcf24f5f1c))
* **import:** import Codex chats with the one button, and continue them on their Codex thread ([#1081](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1081)) ([cfbc964](https://github.com/Atharva-Kanherkar/bridge-harness/commit/cfbc964aaf116545146bd896b7aac1a0a8fbf128))
* **import:** keep talking to an imported chat, natively when Claude Code still has it ([#1078](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1078)) ([7af4b0d](https://github.com/Atharva-Kanherkar/bridge-harness/commit/7af4b0d8dd18e74e4f41550f3a8a249a9b064816))
* **import:** Move to Bridge, find your tools, review by project, import in batches ([#1066](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1066)) ([baa14f1](https://github.com/Atharva-Kanherkar/bridge-harness/commit/baa14f182168bf0a81b4f311e68015984d2faec8))
* **import:** one transfer bundle for every source, merged by native session ([#1064](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1064)) ([6846ef2](https://github.com/Atharva-Kanherkar/bridge-harness/commit/6846ef21896fb895a318f90c78d7feccf84cc838))
* **keys:** let agents use any API key on sites you allow, without seeing it ([#955](https://github.com/Atharva-Kanherkar/bridge-harness/issues/955)) ([c159c4c](https://github.com/Atharva-Kanherkar/bridge-harness/commit/c159c4ceede717c52f0ac386364fd147c1cb254d))
* **latency:** record where every send spends its time, from click to first token ([#1047](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1047)) ([067b893](https://github.com/Atharva-Kanherkar/bridge-harness/commit/067b8938dd060da79aa7bc43362a2beb8379d1e4))
* **limits:** resume chats when a usage limit resets, or switch to a backup model ([#1005](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1005)) ([4a92b45](https://github.com/Atharva-Kanherkar/bridge-harness/commit/4a92b457fcbf110a706c696dd974c3979f428879))
* make harness subagents durable child sessions ([#948](https://github.com/Atharva-Kanherkar/bridge-harness/issues/948)) ([980b793](https://github.com/Atharva-Kanherkar/bridge-harness/commit/980b7939eedd3510b444c75c281df6a869a7321a))
* **mcp:** change Bridge's own settings from a chat ([#959](https://github.com/Atharva-Kanherkar/bridge-harness/issues/959)) ([32cf3f2](https://github.com/Atharva-Kanherkar/bridge-harness/commit/32cf3f27fa2b65972493ba6fc03d5a561bdc553a))
* **memory:** dream nightly to merge, prune and review, with a report and undo ([#1187](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1187)) ([f308870](https://github.com/Atharva-Kanherkar/bridge-harness/commit/f308870b42456c2507d16a942e98533075e25509))
* **memory:** extract once a chat goes quiet and reconcile against known memories ([#1184](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1184)) ([4adbf4b](https://github.com/Atharva-Kanherkar/bridge-harness/commit/4adbf4b443d8c9005ed3a94b52574619a5b8c3af))
* **memory:** harden account memory and give agents memory tools ([#891](https://github.com/Atharva-Kanherkar/bridge-harness/issues/891)) ([3397a55](https://github.com/Atharva-Kanherkar/bridge-harness/commit/3397a55ab6c2daabf0d859c1790aa29766ec9dc3))
* **memory:** page memory lists and review proposals in bulk ([#1153](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1153)) ([8ff15fb](https://github.com/Atharva-Kanherkar/bridge-harness/commit/8ff15fb55bf358a710dff1c7ecddb1110785cebf))
* **memory:** retention, a separate proposal allowance, and restore ([#1181](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1181)) ([dc6409e](https://github.com/Atharva-Kanherkar/bridge-harness/commit/dc6409eaf1346478f0449e10fd8bfc315e6dfe2d))
* **memory:** run every memory job on one memory model ([#1166](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1166)) ([c507a9c](https://github.com/Atharva-Kanherkar/bridge-harness/commit/c507a9c98e3797381905e10d3d5b5e61ca3247f7))
* **menu-bar:** show unread chats in the native menu bar ([#930](https://github.com/Atharva-Kanherkar/bridge-harness/issues/930)) ([ad2b5e4](https://github.com/Atharva-Kanherkar/bridge-harness/commit/ad2b5e4fa94c7dba009100fbc21240f725999004))
* **mission-control:** measured stage and queue instead of the split tree ([#979](https://github.com/Atharva-Kanherkar/bridge-harness/issues/979)) ([1abe7bd](https://github.com/Atharva-Kanherkar/bridge-harness/commit/1abe7bdabb91121464792a17f06a262a9ee93a83))
* **notes:** [[links]] autocomplete, backlinks panel, nested page tree and image captions/resize ([#883](https://github.com/Atharva-Kanherkar/bridge-harness/issues/883)) ([ab6228f](https://github.com/Atharva-Kanherkar/bridge-harness/commit/ab6228f2e19114ee2c53fde6e0044fe69fda4856))
* **notes:** add the notes backend: local page docs, opt-in passes and agent tools ([#869](https://github.com/Atharva-Kanherkar/bridge-harness/issues/869)) ([a389fd6](https://github.com/Atharva-Kanherkar/bridge-harness/commit/a389fd68172d2afaac34c8e4e519f628d9d4abf6))
* **notes:** add the Notes page: capture, two views, tasks and Start ([#872](https://github.com/Atharva-Kanherkar/bridge-harness/issues/872)) ([3c40624](https://github.com/Atharva-Kanherkar/bridge-harness/commit/3c4062439e5a80837f26826c0918adb2e34ba993))
* **notes:** agent drawings that fit the page and check themselves ([#986](https://github.com/Atharva-Kanherkar/bridge-harness/issues/986)) ([2f77f6b](https://github.com/Atharva-Kanherkar/bridge-harness/commit/2f77f6b523d896d2e5c3301b71e659cc4f4ef15b))
* **notes:** block editor, streaming Structure and a collapsible list ([#877](https://github.com/Atharva-Kanherkar/bridge-harness/issues/877)) ([094cce7](https://github.com/Atharva-Kanherkar/bridge-harness/commit/094cce70710daeff4b27b5983a6ea1668d85bbe9))
* **notes:** board, calendar and list views, filters and saved views ([#903](https://github.com/Atharva-Kanherkar/bridge-harness/issues/903)) ([ff2a85d](https://github.com/Atharva-Kanherkar/bridge-harness/commit/ff2a85d1cf83f41a7cc1fa7691ebadb6172fe311))
* **notes:** database pages, the table view and row properties ([#901](https://github.com/Atharva-Kanherkar/bridge-harness/issues/901)) ([4956ab1](https://github.com/Atharva-Kanherkar/bridge-harness/commit/4956ab1f324ffe06656ac9cccfbeb5912d123bbb))
* **notes:** database pages, typed properties and the values index ([#892](https://github.com/Atharva-Kanherkar/bridge-harness/issues/892)) ([173f57c](https://github.com/Atharva-Kanherkar/bridge-harness/commit/173f57c26a3fc9b76c409c35d2665d1b0f477746))
* **notes:** database queries, protocol methods and agent property filters ([#895](https://github.com/Atharva-Kanherkar/bridge-harness/issues/895)) ([93875b4](https://github.com/Atharva-Kanherkar/bridge-harness/commit/93875b412853703d372c74d524ad5717a41ffde3))
* **notes:** draw Mermaid, diagram, math and HTML fences as chat does ([#927](https://github.com/Atharva-Kanherkar/bridge-harness/issues/927)) ([f737e99](https://github.com/Atharva-Kanherkar/bridge-harness/commit/f737e992c816552424a993d89b349c90632b40b8))
* **notes:** Excalidraw drawings that people and agents both draw and act on ([#953](https://github.com/Atharva-Kanherkar/bridge-harness/issues/953)) ([eb953e5](https://github.com/Atharva-Kanherkar/bridge-harness/commit/eb953e569cb959c31b9479124ffce72ff357740b))
* **notes:** link pages from chat and open them in Bridge ([#926](https://github.com/Atharva-Kanherkar/bridge-harness/issues/926)) ([e399339](https://github.com/Atharva-Kanherkar/bridge-harness/commit/e399339c838ab7ce42025c317bdf112668be5105))
* **notes:** page history with restore, and the phase 1 checks in the real app ([#876](https://github.com/Atharva-Kanherkar/bridge-harness/issues/876)) ([c102e5c](https://github.com/Atharva-Kanherkar/bridge-harness/commit/c102e5c1c0b1172e81cf2549264fa6d413648a98))
* **notes:** search, backlinks and validated parent moves ([#881](https://github.com/Atharva-Kanherkar/bridge-harness/issues/881)) ([c52223f](https://github.com/Atharva-Kanherkar/bridge-harness/commit/c52223ffc6575e9ebd992308bbacd2de41685ce3))
* **notes:** show a rejected file value as an import notice ([#918](https://github.com/Atharva-Kanherkar/bridge-harness/issues/918)) ([8c11bdb](https://github.com/Atharva-Kanherkar/bridge-harness/commit/8c11bdbd63be62a2a190e22f33db8118e4405d3e))
* **omarchy:** the Omarchy edition of Bridge, and an Omarchy mode for the landing page ([#829](https://github.com/Atharva-Kanherkar/bridge-harness/issues/829)) ([a81d0cc](https://github.com/Atharva-Kanherkar/bridge-harness/commit/a81d0cc091428e1d41de9e5be242d6a5350b772b))
* **opencode:** support OpenCode 2.x servers ([#837](https://github.com/Atharva-Kanherkar/bridge-harness/issues/837)) ([228557d](https://github.com/Atharva-Kanherkar/bridge-harness/commit/228557daf411620686fae0f7cc0b6ace1c7f7b72))
* **outgoing:** let chats draft messages for people, with tables, code and images ([#924](https://github.com/Atharva-Kanherkar/bridge-harness/issues/924)) ([614f0f4](https://github.com/Atharva-Kanherkar/bridge-harness/commit/614f0f4efcbdfc66dffba807fb90be2c96991cae))
* **outgoing:** send Slack and GitHub drafts with exact approval ([#985](https://github.com/Atharva-Kanherkar/bridge-harness/issues/985)) ([67de964](https://github.com/Atharva-Kanherkar/bridge-harness/commit/67de96485684a74ab5a75b5b3e882b5ca88e99aa))
* **picker:** one segmented effort control, access mode only in the picker ([#1160](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1160)) ([b9c629b](https://github.com/Atharva-Kanherkar/bridge-harness/commit/b9c629b13405367574520c8594b2037b19a75370))
* **pulls:** rename surface and speed up reopening ([#951](https://github.com/Atharva-Kanherkar/bridge-harness/issues/951)) ([d268367](https://github.com/Atharva-Kanherkar/bridge-harness/commit/d2683672eb6b991cdc60d65d0b004af0216ae9e3))
* render charts, diagrams and sourced findings inline with MCP Apps ([#824](https://github.com/Atharva-Kanherkar/bridge-harness/issues/824)) ([270156f](https://github.com/Atharva-Kanherkar/bridge-harness/commit/270156f321a904d6dcee8fa117fa41f08c8855b2))
* **reviews:** a Reviews dashboard for every review, what needs you, and how each reviewer does ([#1155](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1155)) ([a2f9e81](https://github.com/Atharva-Kanherkar/bridge-harness/commit/a2f9e81be3e9b1ddde0142bd15c550772f9dc537))
* **reviews:** add a Second opinion button to the Review menu ([#1186](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1186)) ([ba40195](https://github.com/Atharva-Kanherkar/bridge-harness/commit/ba401955e38aa37216e6394ad51d9c61553e19d0))
* **reviews:** draft triage and Bridge-posted COMMENT reviews ([#1132](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1132)) ([6c339d1](https://github.com/Atharva-Kanherkar/bridge-harness/commit/6c339d1552d5d7505126c522f2532a8cba6cb01b))
* **reviews:** opt-in auto-review on push and incremental re-review ([#1138](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1138)) ([5b9c777](https://github.com/Atharva-Kanherkar/bridge-harness/commit/5b9c777590d20abcbfe9c5e9b4ac66afd47ca23d))
* **reviews:** prepare a detached worktree at the PR head and prefetch context ([#1126](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1126)) ([aec224a](https://github.com/Atharva-Kanherkar/bridge-harness/commit/aec224a620b7194c29504cad322a4bce6d475943))
* **reviews:** review job runner, durable queue and reviews protocol ([#1124](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1124)) ([100f469](https://github.com/Atharva-Kanherkar/bridge-harness/commit/100f469bba36469c9c9ef1607b5dbaf86b24aaa1))
* **reviews:** Reviews section in the GitHub pane, draft triage, and review job settings ([#1135](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1135)) ([a9ae932](https://github.com/Atharva-Kanherkar/bridge-harness/commit/a9ae9320f8cefb6d3bf28246ad16c34ad24a884e))
* **reviews:** second opinion runs two harnesses at once and merges what both flagged ([#1143](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1143)) ([f8a4871](https://github.com/Atharva-Kanherkar/bridge-harness/commit/f8a4871f77f943bde1da619f0a825cc0dd3594ac))
* **reviews:** watch a reviewer work, set each reviewer's model in the Reviews app, and hand reviews to the PR's chat ([#1163](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1163)) ([573f446](https://github.com/Atharva-Kanherkar/bridge-harness/commit/573f44690ee6bac59fadef4ce8eca564554ab7c9))
* **settings:** redesign settings with a full left rail and rebuild the model picker ([#1104](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1104)) ([3f03f02](https://github.com/Atharva-Kanherkar/bridge-harness/commit/3f03f02bffcb2a70139519d9f2d48dcd9c59f91a))
* **sidebar:** chat titles that reword to fit the rail ([#896](https://github.com/Atharva-Kanherkar/bridge-harness/issues/896)) ([6874408](https://github.com/Atharva-Kanherkar/bridge-harness/commit/687440838580806c8015004ab67cdf1d4e18c4de))
* **sidebar:** collapse an expanded chat group with Show less ([#859](https://github.com/Atharva-Kanherkar/bridge-harness/issues/859)) ([9e27df5](https://github.com/Atharva-Kanherkar/bridge-harness/commit/9e27df54f4f444598d37ea47e48361cdbc3438c2))
* **sidebar:** fold Active into Mission Control and make it collapsible ([#1107](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1107)) ([2fe925c](https://github.com/Atharva-Kanherkar/bridge-harness/commit/2fe925c08146b7cacd99d45fd8fd97522478d7be))
* **sidebar:** show linked pull request status on chat rows ([#996](https://github.com/Atharva-Kanherkar/bridge-harness/issues/996)) ([e14ba33](https://github.com/Atharva-Kanherkar/bridge-harness/commit/e14ba337fa21c4f3094556b905f118389df1cff9))
* **storage:** quiet loading state and a fresh assistant rail per visit ([#944](https://github.com/Atharva-Kanherkar/bridge-harness/issues/944)) ([d9fdbb7](https://github.com/Atharva-Kanherkar/bridge-harness/commit/d9fdbb7c751b3db21eea2f8049fb1cfe632e3bc0))
* **titles:** name every chat, and backfill the ones that only say Chat ([#863](https://github.com/Atharva-Kanherkar/bridge-harness/issues/863)) ([f5e024e](https://github.com/Atharva-Kanherkar/bridge-harness/commit/f5e024ed87e5b38dcdc323004c82d2e360bf53bf))
* **trails:** faster trail loading, a cleaner timeline and a trail card from the sidebar ([#1136](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1136)) ([2131ea6](https://github.com/Atharva-Kanherkar/bridge-harness/commit/2131ea6bec27ee6d90ee1f6524e41c866b477c86))
* **trails:** GitHub issues on trails, fresh PR state, and a recall eval ([#999](https://github.com/Atharva-Kanherkar/bridge-harness/issues/999)) ([2b59b18](https://github.com/Atharva-Kanherkar/bridge-harness/commit/2b59b1849c015dcfd4ca6e06bc29c30734797fa9))
* **trails:** one page per piece of work, joining its chats, notes, PRs and drafts ([#993](https://github.com/Atharva-Kanherkar/bridge-harness/issues/993)) ([7f56838](https://github.com/Atharva-Kanherkar/bridge-harness/commit/7f56838d86ebd0e5f27491a96a6938d96c677bb4))
* **transcript:** open diagrams and Mermaid fullscreen with zoom ([#921](https://github.com/Atharva-Kanherkar/bridge-harness/issues/921)) ([19efaf6](https://github.com/Atharva-Kanherkar/bridge-harness/commit/19efaf61b95c49002a54291ae107a0399e0d55e7))
* **ui:** enforce the design system with lint and move every screen onto the primitives ([#1171](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1171)) ([1761e9c](https://github.com/Atharva-Kanherkar/bridge-harness/commit/1761e9cf122df5e63fe2d84265c3b3bc4f20a892))
* **ui:** make tool calls readable with intent, command steps, drawn output and a Now card ([#834](https://github.com/Atharva-Kanherkar/bridge-harness/issues/834)) ([a403da6](https://github.com/Atharva-Kanherkar/bridge-harness/commit/a403da6943d97836ad017aedd4e37bccc35f52e6))
* **ui:** one skeleton kit for every loading state ([#950](https://github.com/Atharva-Kanherkar/bridge-harness/issues/950)) ([166528a](https://github.com/Atharva-Kanherkar/bridge-harness/commit/166528af2e542995b2df0462fc24fd5b9177296f))
* **ui:** one type scale, softer graphite and paper, sentence-case labels ([#1079](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1079)) ([07913b4](https://github.com/Atharva-Kanherkar/bridge-harness/commit/07913b46870697117d1a274696d6684ad24771ec))
* **ui:** T3-inspired chat UI with turn changes, Running, chat overview, worker lineage, model picker and ⌘K actions ([#1096](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1096)) ([5e8d9e2](https://github.com/Atharva-Kanherkar/bridge-harness/commit/5e8d9e25913b4b6ecb95cacacebb5d0fbb33976b))
* **usage:** add Screen Time with your time, agent time and an all-time total ([#1110](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1110)) ([2f09d64](https://github.com/Atharva-Kanherkar/bridge-harness/commit/2f09d640a0c1d70a8ee341babd323e64a4bedc36))
* **usage:** break Screen Time down by project, pull request, hour and harness ([#1182](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1182)) ([cea1363](https://github.com/Atharva-Kanherkar/bridge-harness/commit/cea1363a59123a536da165bb40c08f6021e36e28))
* **web:** run Bridge in a browser tab against a real daemon ([#839](https://github.com/Atharva-Kanherkar/bridge-harness/issues/839)) ([5c2e131](https://github.com/Atharva-Kanherkar/bridge-harness/commit/5c2e1316ca7626ce733306708c4be95cef6cc2f0))
* **welcome:** calmer start screen modeled on T3 Code ([#1109](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1109)) ([a9d2f76](https://github.com/Atharva-Kanherkar/bridge-harness/commit/a9d2f76ae77d62635911ee3b9ce1476e70b455fd))
* **welcome:** stream rotating project lines into the new-chat headline ([#1161](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1161)) ([e077e4a](https://github.com/Atharva-Kanherkar/bridge-harness/commit/e077e4a78ef90ba770a86d5e154bcc49c1c3a155))
* **work-surface:** move PRs and background work into a composer shelf and a Work pane ([#978](https://github.com/Atharva-Kanherkar/bridge-harness/issues/978)) ([7040e76](https://github.com/Atharva-Kanherkar/bridge-harness/commit/7040e76f774fc2dcf3627b6702596318bcc9a4f8))
* **workflows:** event triggers, templates, and runs you can depend on ([#1134](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1134)) ([dcc7d28](https://github.com/Atharva-Kanherkar/bridge-harness/commit/dcc7d2896242adc2232d117f73210b6f74fc0c98))
* **workflows:** Gmail, Linear and Notion event triggers ([#1141](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1141)) ([cf26aff](https://github.com/Atharva-Kanherkar/bridge-harness/commit/cf26aff1019511fb76b21925db8350bc1501c640))
* **workflows:** repeatable jobs that run on your own harness ([#854](https://github.com/Atharva-Kanherkar/bridge-harness/issues/854)) ([6d2b045](https://github.com/Atharva-Kanherkar/bridge-harness/commit/6d2b0456701792c4804c14a92fdbc9969ca16129))


### Bug Fixes

* **accounts:** launch extra provider sign-in from Bridge ([#1032](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1032)) ([8a469c1](https://github.com/Atharva-Kanherkar/bridge-harness/commit/8a469c17a8da8659f6db010c73d576e8f02ff181))
* **accounts:** show sign-in progress and every account's limits ([#1033](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1033)) ([a03b1f3](https://github.com/Atharva-Kanherkar/bridge-harness/commit/a03b1f310e468daa2fe2b4a8fadbde686dcd7f0e))
* **agents:** clean up the worker pane and stop duplicate permission cards ([#1102](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1102)) ([df6638d](https://github.com/Atharva-Kanherkar/bridge-harness/commit/df6638d34ed17f2ef2d722b06adc75381d534833))
* **agents:** list harness subagents in the Agents pane ([#1095](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1095)) ([a7c17af](https://github.com/Atharva-Kanherkar/bridge-harness/commit/a7c17af946020c1e2f897931da8e95057507cb2c))
* **agents:** show a starting state with the worker's name and topic ([#841](https://github.com/Atharva-Kanherkar/bridge-harness/issues/841)) ([2ea5bab](https://github.com/Atharva-Kanherkar/bridge-harness/commit/2ea5bab7a49f2fd1751f8e19dba676807579c705))
* **apps:** clear the clippy ratchet regressions from the apps registry ([#1012](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1012)) ([176dc8f](https://github.com/Atharva-Kanherkar/bridge-harness/commit/176dc8f8ff445fd2cbec2f096aeceb0dfb350f1b))
* **attention:** read chats stop counting as unread, and mark read or unread by hand ([#997](https://github.com/Atharva-Kanherkar/bridge-harness/issues/997)) ([f67f698](https://github.com/Atharva-Kanherkar/bridge-harness/commit/f67f6987a8f9f7d99e7f119718888cc3d29a98fc))
* **auth:** recover Claude chats after browser sign-in ([#820](https://github.com/Atharva-Kanherkar/bridge-harness/issues/820)) ([0ae9639](https://github.com/Atharva-Kanherkar/bridge-harness/commit/0ae963905c3c181773123aa7d7e45dd29da3f664))
* **bridged:** retry interrupted reads on the WebSocket transport ([#923](https://github.com/Atharva-Kanherkar/bridge-harness/issues/923)) ([36ee082](https://github.com/Atharva-Kanherkar/bridge-harness/commit/36ee082f2dd4befe631651de45e3e6fab95d0b75))
* **browser:** keep the native page inside its pane at any UI zoom ([#1048](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1048)) ([fc27d45](https://github.com/Atharva-Kanherkar/bridge-harness/commit/fc27d45aab1db17c85ba3d18b8f97d801c66252d))
* **browser:** only request a clone for signed-in state or an explicit ask ([#842](https://github.com/Atharva-Kanherkar/bridge-harness/issues/842)) ([8847ae1](https://github.com/Atharva-Kanherkar/bridge-harness/commit/8847ae196c0e410a72e12c4c62c18a5d27715c73))
* **chat:** keep the agent's status in the transcript, not the activity card ([#956](https://github.com/Atharva-Kanherkar/bridge-harness/issues/956)) ([aa10185](https://github.com/Atharva-Kanherkar/bridge-harness/commit/aa10185e622d1c0a4ddf5d70187e618e2db7e68f))
* **chat:** send a pasted image with no text ([#1072](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1072)) ([d885640](https://github.com/Atharva-Kanherkar/bridge-harness/commit/d885640c3e2a85c5d1e001da8dc29fad0c6da747))
* **chats:** start notes, trails and storage chats on the saved chat defaults ([#1007](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1007)) ([4b2b997](https://github.com/Atharva-Kanherkar/bridge-harness/commit/4b2b997d62d43d2fe4867a9d85d2e3dbc6b1e61a))
* **chat:** stop a new chat from naming a model it is about to drop ([#922](https://github.com/Atharva-Kanherkar/bridge-harness/issues/922)) ([88764c9](https://github.com/Atharva-Kanherkar/bridge-harness/commit/88764c9cbb1b77553e4c15a6dbc89f7ab46e01ae))
* **claude:** keep connector discovery off the chat launch path ([2badaea](https://github.com/Atharva-Kanherkar/bridge-harness/commit/2badaea0e3af7fce4600d51475934fc11b4fdeef))
* **claude:** load a repository's .claude settings only for trusted folders ([#947](https://github.com/Atharva-Kanherkar/bridge-harness/issues/947)) ([83ca513](https://github.com/Atharva-Kanherkar/bridge-harness/commit/83ca513cf09babe73a0973326c6a017dfe07dbc4))
* **claude:** stop drawing each thought twice ([#822](https://github.com/Atharva-Kanherkar/bridge-harness/issues/822)) ([4e72ae6](https://github.com/Atharva-Kanherkar/bridge-harness/commit/4e72ae682c6cfa52cfef15cf286c4b7c3d181846))
* **codex:** drop the ignored instructions field and record the prefix experiment ([#835](https://github.com/Atharva-Kanherkar/bridge-harness/issues/835)) ([58752be](https://github.com/Atharva-Kanherkar/bridge-harness/commit/58752be39683c2b89529602414544bffe5213e96))
* **composer:** remove the chat input scrollbar ([#1157](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1157)) ([7fd0eac](https://github.com/Atharva-Kanherkar/bridge-harness/commit/7fd0eac5ccdef64bf06fdfe2d37da2b71f7365f0))
* **composer:** resize oversized pasted images and lift the daemon frame cap ([#893](https://github.com/Atharva-Kanherkar/bridge-harness/issues/893)) ([9d7ed64](https://github.com/Atharva-Kanherkar/bridge-harness/commit/9d7ed649213468778716524433d08f79345bdab2))
* **context-lens:** stop clipping at every size and restyle to the Bridge chrome ([#1010](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1010)) ([999dc3b](https://github.com/Atharva-Kanherkar/bridge-harness/commit/999dc3b8c6438784d19ce9d6fde2a5803a4eda3e))
* **core:** bind completion proof to checkout contents ([#1170](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1170)) ([1950852](https://github.com/Atharva-Kanherkar/bridge-harness/commit/1950852b6b49034bf71b17420b406867775cb6ec))
* **dock:** stop tab badges spilling over neighbouring tabs ([#1189](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1189)) ([dc5901a](https://github.com/Atharva-Kanherkar/bridge-harness/commit/dc5901a2c0fdc47033b5a7d0229f21f4ddf9aab5))
* **editor:** make the code and changes panes usable under a writing agent ([#1111](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1111)) ([d49cb24](https://github.com/Atharva-Kanherkar/bridge-harness/commit/d49cb246ecbfbcd0cafba31ea5a6d75e1eadc75b))
* **github:** back off the PR poller on failed reads and rate limits ([#1185](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1185)) ([0f9bdeb](https://github.com/Atharva-Kanherkar/bridge-harness/commit/0f9bdeb9d4861ab72975326f4ea0eb443aaea288))
* **github:** show one quiet loading state while the pane connects ([#1158](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1158)) ([10c0f42](https://github.com/Atharva-Kanherkar/bridge-harness/commit/10c0f42ed2ae198e18e3f589fdd30834c9603ee3))
* **import:** disable imported-chat cleanup during startup ([#1105](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1105)) ([a968cc4](https://github.com/Atharva-Kanherkar/bridge-harness/commit/a968cc4333fa70db98f5c92c6003d1f22baa7ada))
* **import:** file imported chats under their projects and stop importing runs as chats ([#1103](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1103)) ([193288f](https://github.com/Atharva-Kanherkar/bridge-harness/commit/193288fd72cf20430bdd5ec8ab0835fafd904cef))
* **import:** make removing imported runs fast so it never stalls Bridge ([#1108](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1108)) ([3356575](https://github.com/Atharva-Kanherkar/bridge-harness/commit/33565753d912a3c089b9c7e6e8d41c50699bf670))
* **import:** one Import button, run in the background, no Review step ([#1071](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1071)) ([c5a031d](https://github.com/Atharva-Kanherkar/bridge-harness/commit/c5a031d8d6f8436a04fb4aefc4425083d35a4fb2))
* **import:** read newer tool databases that still have every column Bridge reads ([#1183](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1183)) ([ea7386f](https://github.com/Atharva-Kanherkar/bridge-harness/commit/ea7386f2526cac563da0262f26aa8e6025ec51ee))
* **import:** read the account folders in Settings, for Claude Code, Codex and OpenCode ([#1089](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1089)) ([4ae3f40](https://github.com/Atharva-Kanherkar/bridge-harness/commit/4ae3f40ad102f142586e7eb3f1606b20df951180))
* **import:** start git through the counted constructor in the worktree test helper ([#1094](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1094)) ([bf59b15](https://github.com/Atharva-Kanherkar/bridge-harness/commit/bf59b154f150a6c95588152fd16af8c661147ee9))
* **layout:** keep the dock, chat header and screens intact in narrow and zoomed windows ([#1177](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1177)) ([e93f067](https://github.com/Atharva-Kanherkar/bridge-harness/commit/e93f067148a53727ff07da1a6cc12a62c2c561d5))
* **limit-recovery:** retry a recovery when the store is busy ([#1140](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1140)) ([4712e14](https://github.com/Atharva-Kanherkar/bridge-harness/commit/4712e14801ac42692f69fca729e0d7944f6088c0))
* **memory:** recall from questions, keep dated personal facts, and search before answering ([#1190](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1190)) ([3a6f653](https://github.com/Atharva-Kanherkar/bridge-harness/commit/3a6f6538468361d507256f12704e8b1a791c49a3))
* **menu-bar:** show every account's limits in the native menu and settings ([#1035](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1035)) ([e0a5e2b](https://github.com/Atharva-Kanherkar/bridge-harness/commit/e0a5e2b5d4d7fbe38a550ea2524fbae1970f1e7c))
* **mission-control:** show each tile's pull requests and background work ([#992](https://github.com/Atharva-Kanherkar/bridge-harness/issues/992)) ([4a40ccc](https://github.com/Atharva-Kanherkar/bridge-harness/commit/4a40cccefcae08dfd3be8ab10274df0a9fcddd03))
* **mission-control:** show only agents that are really working ([#1003](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1003)) ([98cd7d6](https://github.com/Atharva-Kanherkar/bridge-harness/commit/98cd7d64776ccf7bd3665059957f6e4d6e08982c))
* **model-picker:** give symbols and labels consistent spacing ([#1106](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1106)) ([19e07f2](https://github.com/Atharva-Kanherkar/bridge-harness/commit/19e07f2ca625be076543f1bee53829f7d3558b1a))
* **notes:** draw loading skeletons and a way out of every failed read ([#958](https://github.com/Atharva-Kanherkar/bridge-harness/issues/958)) ([1017dc4](https://github.com/Atharva-Kanherkar/bridge-harness/commit/1017dc45a333b489d37844e5d959e70107caf94c))
* **notes:** prevent duplicate tasks and show linked chat progress ([#884](https://github.com/Atharva-Kanherkar/bridge-harness/issues/884)) ([0fa0d32](https://github.com/Atharva-Kanherkar/bridge-harness/commit/0fa0d32469e6ed92baa1c6cf246c1e73f5ffacd9))
* **notes:** rebuild a damaged index when the store opens ([#919](https://github.com/Atharva-Kanherkar/bridge-harness/issues/919)) ([a1b07a2](https://github.com/Atharva-Kanherkar/bridge-harness/commit/a1b07a2d6755fd3a5afd07581839b93c92a647f3))
* **opencode:** run read-only workers in the sandbox, substitute incompatible pins, clearer launch failure card ([#1002](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1002)) ([4367ff6](https://github.com/Atharva-Kanherkar/bridge-harness/commit/4367ff68513ec267b4226a4d6f5c86760856f8b6))
* **orchestration:** uncap workers, make waits see the queue, and recover failed workers ([#1090](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1090)) ([11c0de7](https://github.com/Atharva-Kanherkar/bridge-harness/commit/11c0de77addec685501509a4789c67d6beb821a9))
* **orchestrator:** clear the nine permission and completion bottlenecks ([#1013](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1013)) ([9c53c72](https://github.com/Atharva-Kanherkar/bridge-harness/commit/9c53c72742e4e07e7cbde4769c9e18b8c7ee1ea9))
* **policy:** let Full access allow OpenCode's web search consent ([#1165](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1165)) ([034b368](https://github.com/Atharva-Kanherkar/bridge-harness/commit/034b3683657d6f88a99a21b88e383fad215cc769))
* **questions:** never auto-answer an agent's question, and answer every shape ([#1074](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1074)) ([e2f3df4](https://github.com/Atharva-Kanherkar/bridge-harness/commit/e2f3df40dcab468216c5fd1c69e0b5638310040b))
* read loosely written reviews and show plain error messages everywhere ([#1180](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1180)) ([9d73e88](https://github.com/Atharva-Kanherkar/bridge-harness/commit/9d73e88fb94ed7d652e4f1f5ea2098fc88ad5d66))
* **release:** unblock the macOS release gate on APFS and the stale visual bundle ([#1192](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1192)) ([0ab3833](https://github.com/Atharva-Kanherkar/bridge-harness/commit/0ab38338157aada833526c0fa14ad919a9a79e6d))
* render GitHub comments properly and stop the model picker crushing its list ([#1176](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1176)) ([2d39439](https://github.com/Atharva-Kanherkar/bridge-harness/commit/2d394395ca9da43cb43e7fd8536600d80ac69977))
* restore the EINTR retry and visual bundle undone by a squash ([#933](https://github.com/Atharva-Kanherkar/bridge-harness/issues/933)) ([51b8dfe](https://github.com/Atharva-Kanherkar/bridge-harness/commit/51b8dfe4475d4f381b55fb8a69f940db2a38c41c))
* **reviews:** answer every review start in the pane, and show the hidden More actions menu ([#1154](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1154)) ([e715269](https://github.com/Atharva-Kanherkar/bridge-harness/commit/e715269cf5741aa64636ed94816c105b891a757f))
* **reviews:** headless offline reviewer session and findings contract, fixes OpenCode reviews ([#1129](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1129)) ([9ff4f8d](https://github.com/Atharva-Kanherkar/bridge-harness/commit/9ff4f8dd732f6a29a3cd2c01525df72ef5554a9f))
* **runtime:** keep background process activity visible and truthful ([#928](https://github.com/Atharva-Kanherkar/bridge-harness/issues/928)) ([7b42c43](https://github.com/Atharva-Kanherkar/bridge-harness/commit/7b42c43b1c2198099335333a725a03a9f7415fcf))
* **sessions:** keep the conversation across model switches ([#1087](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1087)) ([fe6e6ad](https://github.com/Atharva-Kanherkar/bridge-harness/commit/fe6e6adb324727f2c33a753fe9d0e21be063ab40))
* **settings:** give Settings the sidebar's place and center its pages ([#1113](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1113)) ([9201f1c](https://github.com/Atharva-Kanherkar/bridge-harness/commit/9201f1c156854c4c0188af1f31ff00bfbf446cb0))
* **settings:** move Back to app below window controls ([#1173](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1173)) ([029b13a](https://github.com/Atharva-Kanherkar/bridge-harness/commit/029b13acde72f1cb5855ec3da1ae7219582761c0))
* **shell:** accept omitted optional arguments on every Tauri command ([#1125](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1125)) ([aae0593](https://github.com/Atharva-Kanherkar/bridge-harness/commit/aae0593a249b2600bef88b37a9d4a63d97d8fe04))
* **sidebar:** rename the Running group to Active and swap spinners for live dots ([#1099](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1099)) ([8b780b5](https://github.com/Atharva-Kanherkar/bridge-harness/commit/8b780b5d03d178b1870fc8683c2a79a7fa2b107c))
* **storage:** make deleting work, honest sizes, calmer page, resizable assistant ([#961](https://github.com/Atharva-Kanherkar/bridge-harness/issues/961)) ([a713bba](https://github.com/Atharva-Kanherkar/bridge-harness/commit/a713bba713bca4a560fbc532742fa9014fcda48c))
* **subagents:** keep harness background agents alive, and add fast, ultrafast and ultracode ([#1068](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1068)) ([e8dd547](https://github.com/Atharva-Kanherkar/bridge-harness/commit/e8dd54765e0b16f9d27f84bfda3830be222ccafc))
* **subagents:** never track a chat's own root thread as its child ([#960](https://github.com/Atharva-Kanherkar/bridge-harness/issues/960)) ([654717b](https://github.com/Atharva-Kanherkar/bridge-harness/commit/654717b6548c447135b93826eae88d2e256e71e3))
* **terminal:** let chat Run reach the dock shell in the embedded host ([#1137](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1137)) ([92501f5](https://github.com/Atharva-Kanherkar/bridge-harness/commit/92501f5dc70489ef542136afeb9ec73d3e0a40ab))
* **terminal:** render TUIs correctly in the dock and Agent Fleet, and stop losing input ([#1112](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1112)) ([34b9a9f](https://github.com/Atharva-Kanherkar/bridge-harness/commit/34b9a9f35744f2e03a3322ac2e6f6975176fead0))
* **trails:** let the Trails list and trail page scroll, with scroll motion ([#1142](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1142)) ([2c82818](https://github.com/Atharva-Kanherkar/bridge-harness/commit/2c8281884d3e7de34ac11c0348ebdbba005cbcc8))
* **trails:** put agent-written notes on their trail and let people add anything ([#1008](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1008)) ([671e4c6](https://github.com/Atharva-Kanherkar/bridge-harness/commit/671e4c696c79117feb3917d50cca2c65be3b94d9))
* **transcript:** draw a question once, without its tool call ([#1101](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1101)) ([1e15af8](https://github.com/Atharva-Kanherkar/bridge-harness/commit/1e15af8a9a035dda33196f287984404c62eff742))
* treat non-agent hash references as chat text ([#882](https://github.com/Atharva-Kanherkar/bridge-harness/issues/882)) ([2a76028](https://github.com/Atharva-Kanherkar/bridge-harness/commit/2a76028ef1c2e02cab65e2086d3f9c57f8217779))
* **ui:** clear the React hook lint findings and empty the lint baseline ([#1191](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1191)) ([4146ffe](https://github.com/Atharva-Kanherkar/bridge-harness/commit/4146ffe87cede2ed2e02dd08c6504fa707779989))
* **ui:** drop verbose helper copy and the partial-total pill ([#1100](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1100)) ([0b75f08](https://github.com/Atharva-Kanherkar/bridge-harness/commit/0b75f08002092dc02d9e9c001cb8cbe3329f4dbf))
* **ui:** give the new-chat context row room and line up the composer's edges ([#1178](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1178)) ([2f17178](https://github.com/Atharva-Kanherkar/bridge-harness/commit/2f1717868021cc2d27bd1478d52933db6bba2c1f))
* **ui:** replace spinners and loading copy with one skeleton state ([#1162](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1162)) ([35cf95b](https://github.com/Atharva-Kanherkar/bridge-harness/commit/35cf95b6c5f49ca681d729c8edc1243f6bb911be))
* **usage:** hide stale totals while history loads ([#902](https://github.com/Atharva-Kanherkar/bridge-harness/issues/902)) ([9af2a64](https://github.com/Atharva-Kanherkar/bridge-harness/commit/9af2a64f4b1f40588b20ed9646ce07f1e4c88ea9))
* **usage:** repair Claude ledger rows stored as session running totals ([#1098](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1098)) ([69b8d7b](https://github.com/Atharva-Kanherkar/bridge-harness/commit/69b8d7bbaa0cf83542b27f76fb48e4e730490fc1))
* **usage:** show figures at once and make the history scan about 10x faster ([#941](https://github.com/Atharva-Kanherkar/bridge-harness/issues/941)) ([9428eff](https://github.com/Atharva-Kanherkar/bridge-harness/commit/9428eff908804b726a17fa7953bd1aa3a38eb6d5))
* **usage:** show OpenCode Go limits from OpenCode 2.x sign-in, with Refresh in the card ([#870](https://github.com/Atharva-Kanherkar/bridge-harness/issues/870)) ([8309da8](https://github.com/Atharva-Kanherkar/bridge-harness/commit/8309da8f2d4a5fd50152554c67ecf469b03c2810))
* **voice:** preserve takes and streamline dictation UI ([#830](https://github.com/Atharva-Kanherkar/bridge-harness/issues/830)) ([bdabca0](https://github.com/Atharva-Kanherkar/bridge-harness/commit/bdabca065a162e871527a1c2d8c49f01d9f50cd5))
* **workflows:** finish phase one runtime follow-ups ([#975](https://github.com/Atharva-Kanherkar/bridge-harness/issues/975)) ([6c27cf8](https://github.com/Atharva-Kanherkar/bridge-harness/commit/6c27cf8410830ed8d0d4c2d8873fb3ae735878a1))
* **workflows:** reach connectors, close built-in escapes, stop and recover runs ([#1128](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1128)) ([90e6834](https://github.com/Atharva-Kanherkar/bridge-harness/commit/90e68346a97e538dd35a055e570239b92844a9a1))


### Performance Improvements

* **chat:** open long chats instantly ([#998](https://github.com/Atharva-Kanherkar/bridge-harness/issues/998)) ([aac1452](https://github.com/Atharva-Kanherkar/bridge-harness/commit/aac1452472136825125c1f817f3da28190f90227))
* **claude:** stop paying for connector discovery a chat launch discards ([#1014](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1014)) ([c4607b2](https://github.com/Atharva-Kanherkar/bridge-harness/commit/c4607b29bedacc259e52d34f3d4ebb4e2cf20b83))
* **core:** keep Bridge responsive with ten or more chats running ([#1130](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1130)) ([29369f4](https://github.com/Atharva-Kanherkar/bridge-harness/commit/29369f4406a258c88bc6000bb78403bac7f2f303))
* **forest:** move repository stamps outside the database lock ([#1169](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1169)) ([e00f60e](https://github.com/Atharva-Kanherkar/bridge-harness/commit/e00f60ebee39783b40043659624830e35fa4a1a4))
* **notes:** open the Notes list in 59 ms instead of 1.5 s ([#994](https://github.com/Atharva-Kanherkar/bridge-harness/issues/994)) ([7faaafe](https://github.com/Atharva-Kanherkar/bridge-harness/commit/7faaafe76f17c16a15c5658d3e602541288450e8))
* stop per-second and per-commit stalls and trim memory across store, daemon and chat ([#1167](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1167)) ([044c905](https://github.com/Atharva-Kanherkar/bridge-harness/commit/044c905c0590eb88399c7181adb9ba43dd33b210))
* **transcript:** open long chats on their newest rows ([#864](https://github.com/Atharva-Kanherkar/bridge-harness/issues/864)) ([9477e8f](https://github.com/Atharva-Kanherkar/bridge-harness/commit/9477e8fc321b14d25b7555444865c26bedfb338d))

## [0.9.0](https://github.com/Atharva-Kanherkar/bridge-harness/compare/v0.8.0...v0.9.0) (2026-10-05)


### Features

* add supervised composer dictation ([#683](https://github.com/Atharva-Kanherkar/bridge-harness/issues/683)) ([037627b](https://github.com/Atharva-Kanherkar/bridge-harness/commit/037627b357b7b93e2f60d9396b6f505a52d96c19))
* **bridged:** opt-in WebSocket listener and deployment credential policy ([#815](https://github.com/Atharva-Kanherkar/bridge-harness/issues/815)) ([aa0f663](https://github.com/Atharva-Kanherkar/bridge-harness/commit/aa0f6631da6d8b7fbe531fd55f384c6109d628c4))
* **delegation:** opt out of informational worker notices ([#818](https://github.com/Atharva-Kanherkar/bridge-harness/issues/818)) ([db0a778](https://github.com/Atharva-Kanherkar/bridge-harness/commit/db0a7781d218f04d89f55df4dc8cbf59725c2594))
* **memory:** real recall analytics, run log, and Insights tab ([#812](https://github.com/Atharva-Kanherkar/bridge-harness/issues/812)) ([3520c79](https://github.com/Atharva-Kanherkar/bridge-harness/commit/3520c799a53502b5c1ca5e2e3008ba6891312d58))
* **new-chat:** choose an agent when starting a chat ([#814](https://github.com/Atharva-Kanherkar/bridge-harness/issues/814)) ([62006af](https://github.com/Atharva-Kanherkar/bridge-harness/commit/62006af5b6d32e0ff485797d733606e004658b60))
* **storage:** dock a standing chat beside the storage page ([#811](https://github.com/Atharva-Kanherkar/bridge-harness/issues/811)) ([9e188ef](https://github.com/Atharva-Kanherkar/bridge-harness/commit/9e188efa49062269e2ad75ac0edd17fad0a55f7b))
* **storage:** give the storage agent its own system prompt, plan cards, and a grid layout ([#813](https://github.com/Atharva-Kanherkar/bridge-harness/issues/813)) ([e0a52a1](https://github.com/Atharva-Kanherkar/bridge-harness/commit/e0a52a12abbf7186df7fb1aee2f228cec140a170))


### Bug Fixes

* **dictation:** survive model load, honor early release, free Enter, retry back-to-back takes ([#816](https://github.com/Atharva-Kanherkar/bridge-harness/issues/816)) ([16c4ddc](https://github.com/Atharva-Kanherkar/bridge-harness/commit/16c4ddc749e2b4679aa42c8ba2de8346015168ba))
* **memory:** show a loading skeleton instead of a blank pin list ([#810](https://github.com/Atharva-Kanherkar/bridge-harness/issues/810)) ([cd8a605](https://github.com/Atharva-Kanherkar/bridge-harness/commit/cd8a605ed6c6b7f978fc046b8440aabcb632d39e))
* **updater:** rotate the updater public key ([#819](https://github.com/Atharva-Kanherkar/bridge-harness/issues/819)) ([e3a4337](https://github.com/Atharva-Kanherkar/bridge-harness/commit/e3a433789f1dc5c7c17e50f7dfeec2aecc807fc5))
* **voice:** keep dictation available across turns and on new chats ([#808](https://github.com/Atharva-Kanherkar/bridge-harness/issues/808)) ([e2a52e8](https://github.com/Atharva-Kanherkar/bridge-harness/commit/e2a52e8186343a6a2b7b7cba126772cd8d87fa25))

## [0.8.0](https://github.com/Atharva-Kanherkar/bridge-harness/compare/v0.7.0...v0.8.0) (2026-10-04)


### Features

* **memory:** calmer memory surface and a one-time recall note in chat ([#802](https://github.com/Atharva-Kanherkar/bridge-harness/issues/802)) ([8f8f990](https://github.com/Atharva-Kanherkar/bridge-harness/commit/8f8f990a6b71a042d3ed2e701fd4138f58930283))
* **storage:** manage disk space across the whole Mac ([#804](https://github.com/Atharva-Kanherkar/bridge-harness/issues/804)) ([3811a99](https://github.com/Atharva-Kanherkar/bridge-harness/commit/3811a9908b68353167c837fbd5c15981740c783d))


### Bug Fixes

* refresh Bridge B icons across app and web ([#803](https://github.com/Atharva-Kanherkar/bridge-harness/issues/803)) ([25a50c8](https://github.com/Atharva-Kanherkar/bridge-harness/commit/25a50c848048164fee6b995f15733e1b78df704c))

## [0.7.0](https://github.com/Atharva-Kanherkar/bridge-harness/compare/v0.6.0...v0.7.0) (2026-10-03)


### Features

* add Steer/Queue settings and move archived chats into Settings ([#790](https://github.com/Atharva-Kanherkar/bridge-harness/issues/790)) ([7e65af7](https://github.com/Atharva-Kanherkar/bridge-harness/commit/7e65af7237f703b8703874314de3c12d06d4ca03))
* **browser:** one Browser pane for your pages and the agent's throwaway clone ([#782](https://github.com/Atharva-Kanherkar/bridge-harness/issues/782)) ([6a3b050](https://github.com/Atharva-Kanherkar/bridge-harness/commit/6a3b0502d95a72624cb8da5788d941d1cd379d62))
* **gitplace:** revamp the GitHub surface's layout and colour ([#796](https://github.com/Atharva-Kanherkar/bridge-harness/issues/796)) ([6ca694b](https://github.com/Atharva-Kanherkar/bridge-harness/commit/6ca694bdb1c591aa925f72f82d91856a4ac56f35))
* **landing:** lead the feature scroll with what only Bridge does ([#779](https://github.com/Atharva-Kanherkar/bridge-harness/issues/779)) ([9775d2e](https://github.com/Atharva-Kanherkar/bridge-harness/commit/9775d2e334cc67a6cb7229b66bd59d9687d6eb3f))
* simplify settings and first-run agent setup ([#788](https://github.com/Atharva-Kanherkar/bridge-harness/issues/788)) ([6fafd8b](https://github.com/Atharva-Kanherkar/bridge-harness/commit/6fafd8bc195ce423c0af5b079c05ddf6ffaeae2e))
* **transcript:** pulse the worker status placeholders ([#798](https://github.com/Atharva-Kanherkar/bridge-harness/issues/798)) ([1026e87](https://github.com/Atharva-Kanherkar/bridge-harness/commit/1026e87612e6ac139e0b63fd0a9839b1b5985cc8))
* **usage:** drop Classic layout, layout picker is a dropdown ([#801](https://github.com/Atharva-Kanherkar/bridge-harness/issues/801)) ([3ade29c](https://github.com/Atharva-Kanherkar/bridge-harness/commit/3ade29c7edb46b4a29090089d243de32f1908fb6))


### Bug Fixes

* **auth:** confirm provider sign-in and offer the retry ([#793](https://github.com/Atharva-Kanherkar/bridge-harness/issues/793)) ([66322a6](https://github.com/Atharva-Kanherkar/bridge-harness/commit/66322a61acc55893e0777cbfa994264dfe3a1d68))
* **browser:** correct Chrome automation flags and sign-in recovery ([#792](https://github.com/Atharva-Kanherkar/bridge-harness/issues/792)) ([0b4d9f1](https://github.com/Atharva-Kanherkar/bridge-harness/commit/0b4d9f1879cdef02644b1fc478f093ad0cb8813f))
* **browser:** preserve signed-in sessions across approved domains ([#784](https://github.com/Atharva-Kanherkar/bridge-harness/issues/784)) ([2a9b050](https://github.com/Atharva-Kanherkar/bridge-harness/commit/2a9b050dc4e73ffca958e1a54f9bcdfe3ea95a5f))
* **chat:** per-chat drafts, safe send recovery, repeat bubbles, and old memory pins ([#800](https://github.com/Atharva-Kanherkar/bridge-harness/issues/800)) ([22b4c79](https://github.com/Atharva-Kanherkar/bridge-harness/commit/22b4c79bffd614bc16b4aa137190a39aa10d2ba0))
* **chat:** say when the harness is compacting its context ([#797](https://github.com/Atharva-Kanherkar/bridge-harness/issues/797)) ([b0099b1](https://github.com/Atharva-Kanherkar/bridge-harness/commit/b0099b133d1aa4cc532fdc4f9bfbf66e454c4a78))
* prevent dev update installs from replacing Cargo builds ([#745](https://github.com/Atharva-Kanherkar/bridge-harness/issues/745)) ([a4718ea](https://github.com/Atharva-Kanherkar/bridge-harness/commit/a4718ea8ebef7bcdda8135fbbc721045e4b7d457))
* restore automatic chat PR cards and live check updates ([#777](https://github.com/Atharva-Kanherkar/bridge-harness/issues/777)) ([8aedcb7](https://github.com/Atharva-Kanherkar/bridge-harness/commit/8aedcb71f2c468b5f9302dea978fcdbe33188730))
* restore composer skill discovery and AI suggestions ([#794](https://github.com/Atharva-Kanherkar/bridge-harness/issues/794)) ([fb5bb73](https://github.com/Atharva-Kanherkar/bridge-harness/commit/fb5bb73b3d5538667f5645fa0a5f47d61742739c))
* **sessions:** steering a chat stops the running turn and runs the message next ([#778](https://github.com/Atharva-Kanherkar/bridge-harness/issues/778)) ([ecca02d](https://github.com/Atharva-Kanherkar/bridge-harness/commit/ecca02db3e11926dfa0cf4713f2ec7e078046ec0))
* stop sessions automatically when archiving chats ([#789](https://github.com/Atharva-Kanherkar/bridge-harness/issues/789)) ([86f235a](https://github.com/Atharva-Kanherkar/bridge-harness/commit/86f235aed30038e93988bd60970b9f2f967279ad))

## [0.6.0](https://github.com/Atharva-Kanherkar/bridge-harness/compare/v0.5.10...v0.6.0) (2026-10-02)


### Features

* add a Direct chat mode that skips the orchestrator prompt ([#733](https://github.com/Atharva-Kanherkar/bridge-harness/issues/733)) ([1c12c76](https://github.com/Atharva-Kanherkar/bridge-harness/commit/1c12c7653c9ab2f3ee672dd61ac015f78644aa02))
* add beta nightly update channel and settings controls ([#738](https://github.com/Atharva-Kanherkar/bridge-harness/issues/738)) ([e3ad782](https://github.com/Atharva-Kanherkar/bridge-harness/commit/e3ad782888b78860e933e2fe124cf93021d5d220))
* add cross-chat conversation search ([#766](https://github.com/Atharva-Kanherkar/bridge-harness/issues/766)) ([670fbd2](https://github.com/Atharva-Kanherkar/bridge-harness/commit/670fbd204741285f3fac317243e7945e0d858be4))
* add native browser tabs and selected element editing ([9246c97](https://github.com/Atharva-Kanherkar/bridge-harness/commit/9246c9788887737b77d8280f56e0eb3af1bb62a4))
* **agents:** an Agents pane for workers and harness subagents, and pointer lines in the chat ([1c21fcf](https://github.com/Atharva-Kanherkar/bridge-harness/commit/1c21fcf0c72b65cba824ec89ed019c5cb7f4923a))
* **agents:** give workers and subagents a home in the dock ([281d15a](https://github.com/Atharva-Kanherkar/bridge-harness/commit/281d15aab6f6da2f9447da967be27b05e54e801a))
* **agents:** keep a chat busy while its agents run ([eaba9f7](https://github.com/Atharva-Kanherkar/bridge-harness/commit/eaba9f78acb5f321e01dc1c0c460a525adb5e105))
* **agents:** open the Agents pane on spawn and show the agent as a chat ([#730](https://github.com/Atharva-Kanherkar/bridge-harness/issues/730)) ([0741c1a](https://github.com/Atharva-Kanherkar/bridge-harness/commit/0741c1a64e555cf198f52128bf4d8920cd73894e))
* **agents:** show only a chat's running agents in the dock ([933a492](https://github.com/Atharva-Kanherkar/bridge-harness/commit/933a492683027595f8913a5397affef81566d84f))
* **agents:** show only a chat's running agents in the dock ([dfbd6c2](https://github.com/Atharva-Kanherkar/bridge-harness/commit/dfbd6c2e536ead03be57fbc15b96a654337fc8dc))
* **attribution:** hide-ai-attribution toggle in Gitplace ([#741](https://github.com/Atharva-Kanherkar/bridge-harness/issues/741)) ([2941bb8](https://github.com/Atharva-Kanherkar/bridge-harness/commit/2941bb8f82b8af174ba75feb400e588610b83276))
* **browser:** add native tabs and selected component edits ([617500d](https://github.com/Atharva-Kanherkar/bridge-harness/commit/617500d8fbf6a9daf8a8f1ffc8e974f177dd205a))
* **browser:** clone dock surface and settings ([d2f30c0](https://github.com/Atharva-Kanherkar/bridge-harness/commit/d2f30c01c6dc5791ffa9cf8d7102978c25d9bf25))
* **browser:** clone leak guard and egress proxy ([ddf33e0](https://github.com/Atharva-Kanherkar/bridge-harness/commit/ddf33e06454500762079b9cbc48f78b8f5cdfeec))
* **browser:** compose clones into the end-to-end flow ([3cac62a](https://github.com/Atharva-Kanherkar/bridge-harness/commit/3cac62ab0caf32849b502e34fd2224a47b424112))
* **browser:** full agent control of clones, vision on by default, UI revamp ([#767](https://github.com/Atharva-Kanherkar/bridge-harness/issues/767)) ([d24bdfb](https://github.com/Atharva-Kanherkar/bridge-harness/commit/d24bdfb6dea71d98cb93d4cf4453a9b847fb9dd2))
* **browser:** sign-in import and the agent clone tool ([e365363](https://github.com/Atharva-Kanherkar/bridge-harness/commit/e365363062856a25299f70c13471e06f2b27b213))
* **browser:** takeover, input forwarding, and the enforced lease ([f9191f6](https://github.com/Atharva-Kanherkar/bridge-harness/commit/f9191f6355beca15d03a2178bb9bbbbb5c521c77))
* **browser:** the agent-asks lifecycle, approval, and concurrency ([a7782a3](https://github.com/Atharva-Kanherkar/bridge-harness/commit/a7782a3200ff6530ddaf27a8d40a66fb009da60a))
* **browser:** throwaway clone process and lifecycle ([ef741a4](https://github.com/Atharva-Kanherkar/bridge-harness/commit/ef741a4dd7fe20d2a7994d9a4611b2b9468b436a))
* **browser:** wire the clone trigger through to the UI ([a42be72](https://github.com/Atharva-Kanherkar/bridge-harness/commit/a42be7260062a7a97858f1bb1d5de0a4d814f135))
* **chat:** thinking row, reply copy, logo timeline, Gitplace tab, and UX cleanup ([#729](https://github.com/Atharva-Kanherkar/bridge-harness/issues/729)) ([ddabdcd](https://github.com/Atharva-Kanherkar/bridge-harness/commit/ddabdcd8d51308ae609c85e89cd4bd9abb1729b1))
* **codex:** pin Codex 0.159.2 and add GPT-6.1 Sol ([#763](https://github.com/Atharva-Kanherkar/bridge-harness/issues/763)) ([122aea1](https://github.com/Atharva-Kanherkar/bridge-harness/commit/122aea1df4ab220698ad1bec10e3b1009708c925))
* **context:** open the Context lens as a modal ([#774](https://github.com/Atharva-Kanherkar/bridge-harness/issues/774)) ([f41d42f](https://github.com/Atharva-Kanherkar/bridge-harness/commit/f41d42f6eb0cd60f0132793f1325ff0be6dcb6ba))
* in-chat context lens for every model and agent window ([#769](https://github.com/Atharva-Kanherkar/bridge-harness/issues/769)) ([7dbab1d](https://github.com/Atharva-Kanherkar/bridge-harness/commit/7dbab1d1e5e6394a3ad435e6aae28b780d83f7b2))
* live pull request and CI status card in the originating chat ([#743](https://github.com/Atharva-Kanherkar/bridge-harness/issues/743)) ([8ce3f4e](https://github.com/Atharva-Kanherkar/bridge-harness/commit/8ce3f4e79e8cc9c36a9a3f99180939fa9ce76824))
* make edit activity expansion optional ([#747](https://github.com/Atharva-Kanherkar/bridge-harness/issues/747)) ([f677a8c](https://github.com/Atharva-Kanherkar/bridge-harness/commit/f677a8c3c9f2c38d1bf50cebc9a6bbb8426e7fe5))
* **mission-control:** make each tile say which project and task it is ([baaa586](https://github.com/Atharva-Kanherkar/bridge-harness/commit/baaa586e47dd7a552182907de9f8a82c1cd3fbfc))
* **mission-control:** make each tile say which project and task it is ([359a642](https://github.com/Atharva-Kanherkar/bridge-harness/commit/359a642d55e352257dc486a73f0e7b2a7d9c7ae7))
* **mission-control:** new chat button ([#772](https://github.com/Atharva-Kanherkar/bridge-harness/issues/772)) ([4d57917](https://github.com/Atharva-Kanherkar/bridge-harness/commit/4d579173afac5e0d430c073989db1c9f53a6dbf6))
* offer Codex CLI update when outdated ([4f063ab](https://github.com/Atharva-Kanherkar/bridge-harness/commit/4f063abd62ac658dd28f79a36c244a6f1f56d709))
* **plugins:** category views, featured defaults, and resolved app names ([#739](https://github.com/Atharva-Kanherkar/bridge-harness/issues/739)) ([f7d64b1](https://github.com/Atharva-Kanherkar/bridge-harness/commit/f7d64b1ce4aa20537fce96fc0c3ab51284e7a99b))
* **router:** let the orchestrator pin a harness or model with the tier as fallback ([5a7a9f4](https://github.com/Atharva-Kanherkar/bridge-harness/commit/5a7a9f45a805ba60707776aa6886e88651171f27))
* **router:** let the orchestrator pin a harness or model with the tier as fallback ([f87c50a](https://github.com/Atharva-Kanherkar/bridge-harness/commit/f87c50a43035a9ed2b7eb7ae9c1e4aacb161621f))
* **sessions:** fork a conversation into an independent session ([23502b0](https://github.com/Atharva-Kanherkar/bridge-harness/commit/23502b0da4a2b980bbca0372fb118d7675a890e4))
* **sessions:** fork a conversation into an independent session ([#660](https://github.com/Atharva-Kanherkar/bridge-harness/issues/660)) ([23502b0](https://github.com/Atharva-Kanherkar/bridge-harness/commit/23502b0da4a2b980bbca0372fb118d7675a890e4))
* **sessions:** resolve_reference — portable chat ids as composer chips ([91ecef4](https://github.com/Atharva-Kanherkar/bridge-harness/commit/91ecef4af8874752bbea2f0a6cbc2652c69a80f9))
* **sessions:** resolve_reference — portable chat ids as composer chips ([#666](https://github.com/Atharva-Kanherkar/bridge-harness/issues/666)) ([91ecef4](https://github.com/Atharva-Kanherkar/bridge-harness/commit/91ecef4af8874752bbea2f0a6cbc2652c69a80f9))
* **sidebar:** chat row actions menu with copy id, mention, fork; honest reference chips ([971394f](https://github.com/Atharva-Kanherkar/bridge-harness/commit/971394f3611dfee60eb94efbe9c1ec50393fad61))
* **transcript:** inspect harness-spawned subagents inline ([af01c55](https://github.com/Atharva-Kanherkar/bridge-harness/commit/af01c55bb75b87a4f746583fe5a9578f957f258b))
* **transcript:** parse harness-spawned subagent facet (issue [#667](https://github.com/Atharva-Kanherkar/bridge-harness/issues/667)) ([cfd5102](https://github.com/Atharva-Kanherkar/bridge-harness/commit/cfd51026e8a1ad5d1e108b966cac375e4078da12))
* **transcript:** polish subagent inspection card with markdown prompt, copy buttons and status header ([ab9aad5](https://github.com/Atharva-Kanherkar/bridge-harness/commit/ab9aad518ee41191ce7d3de30b189a248a8e51cc))
* **transcript:** show harness subagent prompt and result inline (issue [#667](https://github.com/Atharva-Kanherkar/bridge-harness/issues/667)) ([5ebad07](https://github.com/Atharva-Kanherkar/bridge-harness/commit/5ebad0772fb67611c03da939f7f8b432d2fe2e69))
* **transcript:** thinking toggle, agent-directed failures, and honest live state ([#737](https://github.com/Atharva-Kanherkar/bridge-harness/issues/737)) ([ed5db04](https://github.com/Atharva-Kanherkar/bridge-harness/commit/ed5db0407dffb2fa4e262b837058a1dd4c1fb540))
* **turns:** attach a referenced chat's stored history when its id is in the turn ([09fc067](https://github.com/Atharva-Kanherkar/bridge-harness/commit/09fc067f73f2c24e1e29f2dbcdda79fb235dcaec))
* **ui:** fork a conversation from the transcript with rewind and sidebar breadcrumbs ([f3b2973](https://github.com/Atharva-Kanherkar/bridge-harness/commit/f3b29730fe8bfb9007e9efae532c977a1d47457e))
* **ui:** fork a conversation from the transcript with rewind and sidebar breadcrumbs ([#663](https://github.com/Atharva-Kanherkar/bridge-harness/issues/663)) ([f3b2973](https://github.com/Atharva-Kanherkar/bridge-harness/commit/f3b29730fe8bfb9007e9efae532c977a1d47457e))
* **usage:** add switchable Ledger, Strips, Flow, Mosaic, and Calendar layouts ([3f7bd53](https://github.com/Atharva-Kanherkar/bridge-harness/commit/3f7bd536ea9bca53bede271378aa0f3e3e745636))
* **usage:** step 1 — define banked reset protocol ([1b50d42](https://github.com/Atharva-Kanherkar/bridge-harness/commit/1b50d42b58d73580a84b3043ff795927fcb6580b))
* **usage:** step 2 — collect and redeem banked resets ([191b315](https://github.com/Atharva-Kanherkar/bridge-harness/commit/191b315aaa077b6155650c340ed81a8bd15fa38f))
* **usage:** step 3 — surface and redeem banked resets in the UI ([66b536f](https://github.com/Atharva-Kanherkar/bridge-harness/commit/66b536f991837d29862859394f5c6a041bab64c5))
* **usage:** surface and redeem banked rate-limit resets ([8d4cdc1](https://github.com/Atharva-Kanherkar/bridge-harness/commit/8d4cdc1952dbd81398c173e93fc90f0c234acf2d))
* **usage:** switchable Ledger, Strips, Flow, Mosaic, and Calendar layouts ([48877bf](https://github.com/Atharva-Kanherkar/bridge-harness/commit/48877bf5294202caf1ce1ad2f0ad5a2bd0b0abb8))


### Bug Fixes

* add spacing after diff gutter ([4116d8a](https://github.com/Atharva-Kanherkar/bridge-harness/commit/4116d8aa03ed123a1c689396ffdb52aedab75d66))
* **agents:** backfill worker steps, count only edited files, label pin per row ([2ce0944](https://github.com/Atharva-Kanherkar/bridge-harness/commit/2ce09440980dc1b7beae27cf0bb0f131845ae0f6))
* **agents:** keep a pinned agent in the tray when you switch chats ([677cf3b](https://github.com/Atharva-Kanherkar/bridge-harness/commit/677cf3b5b7c6716d0379e997ea776a29f95f47c0))
* **agents:** offer an update when a managed runtime falls behind its pin ([00ac2c9](https://github.com/Atharva-Kanherkar/bridge-harness/commit/00ac2c9855d0c9013a43c5664057019b735308ff))
* **agents:** offer an update when a managed runtime falls behind its pin ([7377b45](https://github.com/Atharva-Kanherkar/bridge-harness/commit/7377b457fdf82b6fcbd256022ed4f36c99d3e7e2))
* **agents:** one row per subagent, focused by the id the pane minted ([a1b956e](https://github.com/Atharva-Kanherkar/bridge-harness/commit/a1b956e4c93b2e3324bb5762e8eb51f3cea2da18))
* allow direct worker model selection ([#717](https://github.com/Atharva-Kanherkar/bridge-harness/issues/717)) ([a29416a](https://github.com/Atharva-Kanherkar/bridge-harness/commit/a29416aac47c7b1aad18cac1e2527cbde8bf5010))
* auto-dismiss and contain app alerts ([6799388](https://github.com/Atharva-Kanherkar/bridge-harness/commit/6799388bb94a775908bf3c6c1466846e1da5857a))
* cancel browser inspection before reopening tabs ([6807492](https://github.com/Atharva-Kanherkar/bridge-harness/commit/680749272680593d5860d77d633a49179b1357a9))
* **chat-search:** stop probing every harness on each keystroke ([#771](https://github.com/Atharva-Kanherkar/bridge-harness/issues/771)) ([0a9b0f7](https://github.com/Atharva-Kanherkar/bridge-harness/commit/0a9b0f74fddff3d470bbf5d608999d136e2a9ffc))
* **chat:** remove composer band, stop doubled replies, smooth streaming ([#731](https://github.com/Atharva-Kanherkar/bridge-harness/issues/731)) ([dc4cc9d](https://github.com/Atharva-Kanherkar/bridge-harness/commit/dc4cc9d4c0e7cb397cab7495f858cbbf3d6fe2aa))
* **ci:** use underscore input names for actions/first-interaction v3 ([#735](https://github.com/Atharva-Kanherkar/bridge-harness/issues/735)) ([0b3176c](https://github.com/Atharva-Kanherkar/bridge-harness/commit/0b3176cdead43c7b434575ff1afde13ba59fbf36))
* clarify Agent Fleet move-to-tab icon ([416f4d0](https://github.com/Atharva-Kanherkar/bridge-harness/commit/416f4d049834dd40d5214b816650535c9ffc3949))
* clarify Agent Fleet move-to-tab icon ([f6efb3e](https://github.com/Atharva-Kanherkar/bridge-harness/commit/f6efb3e4b1818bb30611a74b50d51901ae592062))
* **codex:** advertise thinking effort levels in model picker ([#724](https://github.com/Atharva-Kanherkar/bridge-harness/issues/724)) ([8238301](https://github.com/Atharva-Kanherkar/bridge-harness/commit/8238301f9baf83b0d1c373017f3c517e6688396f))
* **codex:** chats stuck on thinking after the browser clone PRs ([#770](https://github.com/Atharva-Kanherkar/bridge-harness/issues/770)) ([b125ee3](https://github.com/Atharva-Kanherkar/bridge-harness/commit/b125ee33468180782fd099bc1033ee2eac75b6bd))
* **composer:** remove a reference without reflowing the draft; no sidebar mention on the Welcome screen ([c7dea7a](https://github.com/Atharva-Kanherkar/bridge-harness/commit/c7dea7ab82f32e26746f0db337152eaec21d5036))
* **context:** count only loaded MCP tools, add a per-window Compact button ([#775](https://github.com/Atharva-Kanherkar/bridge-harness/issues/775)) ([13aea54](https://github.com/Atharva-Kanherkar/bridge-harness/commit/13aea54590984ffd8fdd35d169cd659af4cb9626))
* correct session usage, cache reuse, and transcript recovery ([ceb314f](https://github.com/Atharva-Kanherkar/bridge-harness/commit/ceb314f2ca80e40735f03597173c8aa5f36503db))
* **desktop:** keep zoom live where the composer and terminal have focus ([494e3f9](https://github.com/Atharva-Kanherkar/bridge-harness/commit/494e3f91ac9e8088c5b3037f651ec8ae71b8c368))
* **desktop:** serialize zoom so overlapping commands cannot collapse ([e708daa](https://github.com/Atharva-Kanherkar/bridge-harness/commit/e708daa82d50d335516b97c463f0835fdd88c30f))
* **desktop:** step zoom by a tenth, and make a pinch one step ([6ed7cab](https://github.com/Atharva-Kanherkar/bridge-harness/commit/6ed7caba522575bc9cc842a90e9de5c707138947))
* **desktop:** step zoom by a tenth, and make a pinch one step ([7feec28](https://github.com/Atharva-Kanherkar/bridge-harness/commit/7feec2848d12307b85c06930f8b7048328ecd314)), closes [#715](https://github.com/Atharva-Kanherkar/bridge-harness/issues/715)
* **github:** sign in to gh from the GitHub pane ([#746](https://github.com/Atharva-Kanherkar/bridge-harness/issues/746)) ([4b91f13](https://github.com/Atharva-Kanherkar/bridge-harness/commit/4b91f1338786f48f78b5ae722d6e9c23258c2742))
* keep native menu card painted during refresh ([e59948a](https://github.com/Atharva-Kanherkar/bridge-harness/commit/e59948a573a7d2e0fd3eab4f5a63483a41febfe0))
* keep native menu card painted during refresh ([66fe95b](https://github.com/Atharva-Kanherkar/bridge-harness/commit/66fe95b2d30662553af8de21d0d748749f527a04))
* **markdown:** render Mermaid diagrams inline ([a1c00b1](https://github.com/Atharva-Kanherkar/bridge-harness/commit/a1c00b1073107a8acb1ac22c5c04e92660814af9))
* **markdown:** render Mermaid diagrams inline ([5aab58f](https://github.com/Atharva-Kanherkar/bridge-harness/commit/5aab58f55e9aa5fe23a51bcd41d23ff4c6d08d59))
* **mission-control:** narrow "needs you" and make link names one parser ([d93bf71](https://github.com/Atharva-Kanherkar/bridge-harness/commit/d93bf71bff5458d81201e477cbbb0bdcfcd173c2))
* open Memory from memory-used chip ([0e599d0](https://github.com/Atharva-Kanherkar/bridge-harness/commit/0e599d06b3f0dbc9ad683d7e93b793a5f18715c5))
* **plugins:** bound catalog CLI calls and surface load failure ([#748](https://github.com/Atharva-Kanherkar/bridge-harness/issues/748)) ([491b5d4](https://github.com/Atharva-Kanherkar/bridge-harness/commit/491b5d4f8e29f74ca7efd79219393b6da2d5d9fa))
* **policy:** let Full access authorize worker write scope ([#734](https://github.com/Atharva-Kanherkar/bridge-harness/issues/734)) ([fffa6fa](https://github.com/Atharva-Kanherkar/bridge-harness/commit/fffa6fa689c75c23a1b83a001eba214eb0bd30c0))
* prevent Codex updater from hanging ([#725](https://github.com/Atharva-Kanherkar/bridge-harness/issues/725)) ([65d4e81](https://github.com/Atharva-Kanherkar/bridge-harness/commit/65d4e81f5cfb8b07e106422e70f8e3beeb651f13))
* reject unknown harness shortcuts ([#678](https://github.com/Atharva-Kanherkar/bridge-harness/issues/678)) ([2b0726c](https://github.com/Atharva-Kanherkar/bridge-harness/commit/2b0726c7a98c4c2b30991fe186162c2367af71eb))
* **router:** drop unservable pins atomically and respect disabled harnesses ([edc2fb1](https://github.com/Atharva-Kanherkar/bridge-harness/commit/edc2fb1eb2214c4fb570f8a1159b2c84035cf4a5))
* **sessions:** keep a fork out of the agent tree ([3fbbf13](https://github.com/Atharva-Kanherkar/bridge-harness/commit/3fbbf133240295022a0696a3b350cac9101aa3b1))
* **sessions:** make resolve_reference survive the wire ([120ff9a](https://github.com/Atharva-Kanherkar/bridge-harness/commit/120ff9a6c25dba4e794c7ec08f76e4901470df6f))
* **sessions:** pasted chat ids carry their history to the agent; chat row actions menu with fork ([f277720](https://github.com/Atharva-Kanherkar/bridge-harness/commit/f277720aeaa00d3cef6b82366036a498172563a4))
* **sidebar:** stop search from mounting every chat ([#776](https://github.com/Atharva-Kanherkar/bridge-harness/issues/776)) ([67a6a4a](https://github.com/Atharva-Kanherkar/bridge-harness/commit/67a6a4a7ac02442fc8e73dda21a5c7f65ece47a4))
* stabilize syntax highlighting tests ([92c424c](https://github.com/Atharva-Kanherkar/bridge-harness/commit/92c424c0fa6720778e60f78101542391ced51dcc))
* stop a broken stderr pipe from poisoning workspace locks ([fdbf0f4](https://github.com/Atharva-Kanherkar/bridge-harness/commit/fdbf0f4f4a7b4cf7466209d9335a2bb903e307af))
* stop a broken stderr pipe from poisoning workspace locks ([5327b57](https://github.com/Atharva-Kanherkar/bridge-harness/commit/5327b577559513c21bdce120ab12f44f986bb602))
* toast Agent Fleet terminal launch errors ([a5e013b](https://github.com/Atharva-Kanherkar/bridge-harness/commit/a5e013b4f4deb7b7de1b7918b291ad483700b0e2))
* toast Agent Fleet terminal launch errors ([6fc50b3](https://github.com/Atharva-Kanherkar/bridge-harness/commit/6fc50b30ae11b171199cebf7637038fed97dba48))
* **transcript:** address subagent review findings — Agent tool, arguments bag, agentsStates lifecycle ([47a902a](https://github.com/Atharva-Kanherkar/bridge-harness/commit/47a902a8706ef68cb7e40b85013c25e1ef5b6301))
* **transcript:** replay dropped live frames so finished replies always render ([#744](https://github.com/Atharva-Kanherkar/bridge-harness/issues/744)) ([00316c9](https://github.com/Atharva-Kanherkar/bridge-harness/commit/00316c958a20823afea06162ed9d5bf798d3692d))
* **transcript:** show mid-turn assistant messages live ([#732](https://github.com/Atharva-Kanherkar/bridge-harness/issues/732)) ([b05384b](https://github.com/Atharva-Kanherkar/bridge-harness/commit/b05384b204c02ea4f249429b884d18ccdeb11624))
* **ui:** make a fork reachable in the rail and keep its status line ([13aafea](https://github.com/Atharva-Kanherkar/bridge-harness/commit/13aafeaac41cccd5f25d202af7adc6e45165760d))
* **usage:** keep calendar totals on visible cells and reset stale selections ([a56fbd5](https://github.com/Atharva-Kanherkar/bridge-harness/commit/a56fbd595b6baa3df21364a1a1aa75502b992754))
* **usage:** price models newer than a rate refresh and merge context variants ([f6c4d80](https://github.com/Atharva-Kanherkar/bridge-harness/commit/f6c4d80e3d1c35f959e749f9f3f56f288d5ff386))
* **usage:** price Opus 5.5 after a rate refresh and merge [1m] rows ([49fb81c](https://github.com/Atharva-Kanherkar/bridge-harness/commit/49fb81cedf5db9abf89d956c994668cdc907df67))
* **usage:** untangle flow labels, calm the headline, and fold model prices ([74ab49c](https://github.com/Atharva-Kanherkar/bridge-harness/commit/74ab49c1ce0067432fd4bca6784de03ba9f25cc0))

## [0.5.10] - 2026-09-21

- Notify on macOS when a session needs attention. The glass toast does not reset while unrelated UI renders, and a refused connector poll stays queued. The inbox can include mentions that are already read.
- Bring the usage dot back to the chat and headline the tightest window. Usage refresh stays isolated per account.
- Mission Control can close a chat and hide worker sessions. The live badge counts active pinned workers.
- Offer Connect when GitHub has no resolved repository, and ask where a pasted GitHub link should open.
- Propose memory extraction on the chat's own model, and promote safe memories automatically.
- Bound snapshot frames so a full payload, including non-entry overhead, stays inside the string cap.
- Show Codex and OpenCode file changes as named rows and keep patch newlines.
- Stop reloading the session forest on every Claude frame, and time tools from execution.
- Keep OpenCode turns attached to the session tree, reconnect the event stream with backoff, and label subagent rows. A stall watchdog treats provider heartbeats as liveness rather than progress.
- Review a pull request with a configurable reviewer. OpenCode reviews run from an isolated worktree.
- Put the sidebar archive action before the harness mark.

## [0.5.9] - 2026-09-16

- Include migration 58 in the release so databases created by the current Bridge build open successfully.
- Publish a new app version after the schema change so macOS does not retain or select the older 0.5.8 DMG.

## [0.5.8] - 2026-09-12

- Encode the existing updater public key as the minisign text box Tauri requires. The 0.5.7 build was rejected before publication; no signing key was replaced.

- Persist worker-result notifications with their canonical result and deliver the complete report at the parent's next turn boundary. Busy or disconnected parents no longer lose a one-shot notification.
- Recover completed worker reports through peek, keep pending deliveries across restarts, and preserve parent runtimes while work or result delivery is pending.
- Distinguish request throttling from usage exhaustion and API-key failures from subscription sign-in failures.
- Preserve the originating provider on historical errors and render each live/durable failure once without hiding later identical retries.
- External terminal credential changes still require provider-process refresh; this release does not claim to fix stale credentials in a running provider.

## [0.5.6] - 2026-09-11

- Send pasted and uploaded images to Codex, OpenCode, and image-capable Cursor sessions.
- Offer agent installation and provider sign-in during setup, with login recovery that preserves the draft.
- Remove the sidebar reload control.
- Preserve the Doto B artwork through release builds.
- Build Linux Debian, AppImage, and Arch package candidates alongside the signed macOS DMG workflow.

- Show only connected integration activity from the past 24 hours on Work, using source timestamps rather than cache refresh times.
- Remove local checks, approvals, workspace drift, and task-tracker controls from the board. Open it from Settings → Work briefing → Open Work.
- Recognize native GitHub evidence fields and Slack workspace links; keep the browser preview empty instead of showing fabricated work.

## [0.5.5] - 2026-09-08

- Adopt the Span app icon: a mint deck over two off-white supports on a dark tile.
- Regenerate all platform icon exports and check the mint deck and off-white supports at every macOS scale before release.

## [0.5.4] - 2026-09-08

- Integrate main through `65b00ad`, retaining the signed-release, startup, native window, icon, and daemon ownership fixes from 0.5.2 and 0.5.3.
- Show immediate send feedback and improve streaming delivery, transcript updates, and Claude thinking-block completion.
- Bound failed worker-result repair loops and give reused workers a fresh repair budget.
- Improve model-switch handoffs, compaction ownership, provider resume, and context checkpoints.
- Include the updated macOS navigation, settings, model controls, and dark appearance.
- Settle detached model-switch summaries alongside tracked providers during clean shutdown.

## [0.5.3] - 2026-09-08

- Settle provider-process ownership during normal app shutdown so completed or idle chats are not falsely marked failed on the next launch.

## [0.5.2] - 2026-09-08

- Regenerate app icons from the source SVG and verify the visible mark before packaging.
- Report native startup errors in a dialog instead of panicking through macOS launch callbacks; retain bounded diagnostic logs with the executable path and version.
- Let macOS manage its titlebar controls. Use public AppKit material/layer APIs and defer geometry updates outside window callbacks.
- Update Bridge's owned material during resize without repeatedly retaining and releasing Wry's parent view, which reproduced a native deallocation failure.
- Keep other Bridge builds and their active daemons running when a conflicting copy opens.
- Reject overlong local socket paths before runtime startup, with an actionable data-directory error.
- Refresh recommended model profiles when live model discovery replaces their initial aliases.
- Validate signing, entitlements, bundled sidecar dependencies, and notarization before producing a public DMG.

## [0.5.1] - 2026-09-04

### Fixed

- Notarized macOS builds no longer abort on first launch. Hardened Runtime was enabled with an empty entitlements blob, so WKWebView could not JIT and Rust aborted on an uncatchable Objective-C exception in the event loop.
- Window chrome (traffic lights, wallpaper tint, corner radius) now catches Objective-C exceptions instead of aborting the process. On macOS 26 those AppKit calls can throw through tao's run-loop observer, which Rust cannot unwind.

## [0.5.0] - 2026-09-04

First public macOS disk image. Install by opening the DMG and dragging Bridge into Applications.

### Added

- Signed, notarized `.dmg` for macOS 12+ (Apple Silicon host builds).
- Native control room for Codex, Claude Code, OpenCode, and other structured harnesses — messages, reasoning, plans, tools, approvals, and diffs in the desktop UI instead of a provider TUI.
- Git worktree isolation for task workspaces and policy-authorized workers.
- Durable session forest: rewind or fork conversation history without pretending files or provider state were rewound.
- Supervised orchestration with capability tiers, write scopes, budgets, retries, and approvals.
- Checkpointing and resume (hot, provider-native, checkpoint-restored, or fresh).
- Authenticated browser bridge for a user-approved Chrome or Safari tab.
- In-app health, usage, memory, and GitHub surfaces.

### Notes for this cut

- Claude sessions still need Node.js 18+ on the machine. Codex, Claude Code, and OpenCode CLIs stay optional on `PATH`.
- Auto-update and App Store distribution are not in this release.
- The project remains all rights reserved unless a later license says otherwise.
