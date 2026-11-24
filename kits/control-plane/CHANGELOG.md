# Changelog

## 0.3.0

- **The project controller.** A new `control-plane-controller` profile for the control plane's chat. It reads the control plane and the board, and acts through the board only: creating, updating and moving cards (To do starts a card's run), commenting and recording decisions, each batch after the person confirms. It cannot send to or stop other sessions or answer an agent's question. All projects reads only. Needs control-plane 0.0.2 or later.

## 0.2.0

- **The control plane writer.** The `control-plane-writer` profile writes the summary box (what happened, what is happening, what is left), the diary, notes on live runs and PR wait reasons, and checks every Needs you candidate with a recommendation, all with references.
