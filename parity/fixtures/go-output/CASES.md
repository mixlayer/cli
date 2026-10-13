# Cases (number = file name in expected/: N.out is stdout + "exit=N", N.err is stderr)

Run with stdout and stderr piped (not a terminal), NO_COLOR=1, a fresh MIXLAYER_HOME with one profile saved by `profile save p --json --description d`,
MIXLAYER_BASE_URL and MIXLAYER_PLATFORM_URL pointing at a fake API. The fake API answers every GET with
`{"object":"list","data":[{"id":"fake/model","object":"model"}]}` and every POST with a chat completion
(id "x", model "fake/model", content "[]", usage 1/1/2). DB means a custom --db path in that MIXLAYER_HOME.
Timestamps, temp paths and the chat id are normalized.

1. `mxl models list`
2. `mxl models list --json`
3. `mxl models list --csv`
4. `mxl models list --plain`
5. `mxl models list --quiet`
6. `mxl models list --select id`
7. `mxl models list --compact`
8. `mxl models get fake/model`
9. `mxl models get fake/model --csv`
10. `mxl models get fake/model --quiet`
11. `mxl models query fake`
12. `mxl models query fake --refresh`
13. `mxl models query fake --db DB`
14. `mxl models query fake --refresh --db DB`
15. `mxl models list --live-prices`
16. `mxl models list --live-prices --json`
17. `mxl chat --model fake/model --messages '[{"role":"user","content":"hi"}]'`
18. `mxl chat --model fake/model --messages '[{"role":"user","content":"hi"}]' --quiet`
19. `mxl chat --model fake/model --messages '[{"role":"user","content":"hi"}]' --csv`
20. `mxl profile list --csv`
21. `mxl profile list --plain`
22. `mxl profile list --quiet`
