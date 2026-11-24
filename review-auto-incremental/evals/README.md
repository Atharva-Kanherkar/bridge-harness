# Incremental re-review evals (#1122 acceptance)

Each file is one run of `live_incremental_review_on_a_fixture_pr`: the reviewer's final message word for word (`answer`), what Bridge would draft after its repeat filter (`draftedFindings`, `draftedFixed`), usage, and the strict checks. A run passes only if all of `exactlyOneFinding`, `onFirst`, `aboutThePanic`, `droppedNotRaised` and `fixedExactly` hold. `modelRepeatedDropped` records whether the model itself raised the dropped finding before Bridge's filter.

| Run | Reviewer | Pass | Failed checks | Drafted findings | fixed | Seconds | Tokens in / out | Cost (USD) |
|---|---|---|---|---|---|---|---|---|
| 1 | claude | yes | - | `first` panics when n exceeds the slice length (line 15) | f-divide-by-zero | 8 | 4 / 738 | 0.090 |
| 1 | codex | yes | - | Clamp the requested prefix length to the available items (line 16) | f-divide-by-zero | 38 | 185,644 / 838 | not reported |
| 1 | opencode-openai_gpt-5.5 | yes | - | first panics when n exceeds the slice length (line 16) | f-divide-by-zero | 17 | 9,062 / 833 | 0.063 |
| 1 | opencode-opencode-go_glm-5.3 | yes | - | first panics when n exceeds the slice length (line 16) | f-divide-by-zero | 23 | 11,956 / 1,477 | 0.014 |
| 2 | claude | no | exactlyOneFinding, onFirst | `first` panics when n > items.len() (line 16); `average` sum can overflow i32 (line 11) | f-divide-by-zero | 20 | 4 / 1,406 | 0.082 |
| 2 | codex | yes | - | Clamp the requested count to the slice length (line 16) | f-divide-by-zero | 24 | 117,274 / 618 | not reported |
| 2 | opencode-openai_gpt-5.5 | yes | - | first panics when n exceeds the slice length (line 16) | f-divide-by-zero | 28 | 12,622 / 782 | 0.052 |
| 2 | opencode-opencode-go_glm-5.3 | yes | - | first panics when n exceeds the slice length (line 16) | f-divide-by-zero | 27 | 13,622 / 1,897 | 0.016 |
| 3 | claude | yes | - | `first` panics when n > items.len() (line 16) | f-divide-by-zero | 10 | 4 / 1,014 | 0.077 |
| 3 | codex | yes | - | Clamp n to the slice length in first (line 17) | f-divide-by-zero | 23 | 139,606 / 559 | not reported |
| 3 | opencode-openai_gpt-5.5 | no | onFirst | first panics when n exceeds the slice length (line 20) | f-divide-by-zero | 24 | 12,620 / 825 | 0.053 |
| 3 | opencode-opencode-go_glm-5.3 | no | exactlyOneFinding, onFirst | first() panics when n exceeds the slice length (line 16); average() can overflow the i32 sum (line 11) | f-divide-by-zero | 88 | 24,097 / 6,223 | 0.043 |
