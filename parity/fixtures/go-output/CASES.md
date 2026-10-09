# Cases (number = file name in expected/: N.out is stdout + "exit=N", N.err is stderr)

Run with stdout and stderr piped (not a terminal), NO_COLOR=1, a fresh MIXLAYER_HOME with one profile saved by `profile save p --json --description d`,
MIXLAYER_BASE_URL and MIXLAYER_PLATFORM_URL pointing at a fake API. The fake API answers every GET with
`{"object":"list","data":[{"id":"fake/model","object":"model"}]}` and every POST with a chat completion
(id "x", model "fake/model", content "[]", usage 1/1/2). DB means a custom --db path in that MIXLAYER_HOME.
Timestamps, temp paths and the chat id are normalized.

1. `mixlayer models list`
2. `mixlayer models list --json`
3. `mixlayer models list --csv`
4. `mixlayer models list --plain`
5. `mixlayer models list --quiet`
6. `mixlayer models list --select id`
7. `mixlayer models list --compact`
8. `mixlayer models get fake/model`
9. `mixlayer models get fake/model --csv`
10. `mixlayer models get fake/model --quiet`
11. `mixlayer models query fake`
12. `mixlayer models query fake --refresh`
13. `mixlayer models query fake --db DB`
14. `mixlayer models query fake --refresh --db DB`
15. `mixlayer models list --live-prices`
16. `mixlayer models list --live-prices --json`
17. `mixlayer chat --model fake/model --messages '[{"role":"user","content":"hi"}]'`
18. `mixlayer chat --model fake/model --messages '[{"role":"user","content":"hi"}]' --quiet`
19. `mixlayer chat --model fake/model --messages '[{"role":"user","content":"hi"}]' --csv`
20. `mixlayer profile list --csv`
21. `mixlayer profile list --plain`
22. `mixlayer profile list --quiet`
