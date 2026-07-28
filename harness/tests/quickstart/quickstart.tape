# Records the real quickstart validator run as terminal evidence (same VHS
# pattern as iii-hq/templates). VHS does not propagate the typed command's
# exit code, so the workflow decides pass/fail from result.json afterwards.
Output target/harness-quickstart/quickstart.mp4

Set Shell bash
Set Width 1200
Set Height 760
Set FontSize 20
Set FontFamily "DejaVu Sans Mono"
Set Theme "Catppuccin Mocha"
Set Padding 18
Set Framerate 15
Set PlaybackSpeed 3
Set WaitTimeout 1200s

Hide
Type "clear"
Enter
Show

Type "harness/tests/quickstart/run-ci.sh"
Enter
# The validator always ends on one of these lines (success log or the
# cleanup-trap failure message), then the prompt returns once cleanup is done.
Wait+Screen /ALL QUICKSTART ASSERTIONS PASSED|quickstart validator failed/
Wait@60s
Sleep 2s
