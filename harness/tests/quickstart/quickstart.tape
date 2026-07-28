# Replays the validator's non-interactive CI log as terminal evidence. Keeping
# the real run outside the VHS pseudo-terminal preserves installer behavior and
# the original exit code.
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

Hide
Type "clear"
Enter
Show

Type "harness/tests/quickstart/replay-terminal.sh target/harness-quickstart/terminal.log"
Enter
Sleep 15s
Sleep 2s
