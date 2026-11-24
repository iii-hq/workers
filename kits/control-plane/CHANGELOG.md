# Changelog

## 0.3.0

- **The project controller.** A new `control-plane-controller` profile for the control plane's chat. It reads the control plane and the board and acts for the person, preferring the board (To do starts a card's run), and confirms every write or side effect with one question first. Its sessions may call any function the engine allows an agent. Needs control-plane 0.0.2 or later.

## 0.2.0

- **The control plane writer.** The `control-plane-writer` profile writes the summary box (what happened, what is happening, what is left), the diary, notes on live runs and PR wait reasons, and checks every Needs you candidate with a recommendation, all with references.
