# Records the real quickstart validator operations. A unique shell prompt makes
# completion deterministic without matching output that may leave the viewport.
Output target/harness-quickstart/quickstart.mp4

Set Shell bash
Set Width 1200
Set Height 760
Set FontSize 20
Set FontFamily "DejaVu Sans Mono"
Set Theme "Catppuccin Mocha"
Set Padding 18
Set Framerate 15
Set PlaybackSpeed 1.5
Set WaitTimeout 1200s

Hide
Type "export PS1='__HARNESS_QUICKSTART_DONE__> '"
Enter
Type "clear"
Enter
Show

Type "harness/tests/quickstart/run-ci.sh"
Enter
Wait /__HARNESS_QUICKSTART_DONE__>/
Sleep 2s
