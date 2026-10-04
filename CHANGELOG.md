# Changelog

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
