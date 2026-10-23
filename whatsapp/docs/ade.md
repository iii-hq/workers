# Configure and test WhatsApp in ADE

Use the worker page to link phones, configure Meta Cloud API, and verify sends, delivery, and incoming messages.

## ADE interface

The running worker injects a **WhatsApp** page and configuration form into ADE.
Open `#/worker/whatsapp` on your ADE host (normally
`http://127.0.0.1:3113/#/worker/whatsapp`). Choose one of two connection methods:
**Linked phone** or **Meta Cloud API**. Each method has its own setup and tests.
**Linked phone** contains session creation, selection and removal, followed by
**Connection**, **Messages**, and **Groups**. Without a session, it shows a single
creation form and the steps to link your phone. Creating a session does not
start or pair it. Open **Connection** to start the selected session, pair with
a QR or phone code, and send an explicit test message to a phone you control.
Reply from that phone to verify receiving.

**Meta Cloud API** first shows the account, webhook, and test setup steps when
disabled or missing credentials. Use **Configure Cloud API** in that view or
**Configure worker** in the page header; phone sessions need no Meta credentials.
Account credentials and webhook credentials are separate groups. Storage and operating limits are under
**Advanced settings**, which opens when one of its fields needs attention.
Environment variable names are under **Configure with environment variables**.
Once configured, Cloud API has the same test flow using the official transport,
with **Check connection** visible above the form and webhook setup available
on demand. For a free-form test, first send a message from the controlled test
phone to the verified business number. Alternatively, **Use an approved template**
opens the template composer directly. Required parameters must be completed
before sending. Use **Test templates or attachments** for the full Cloud composer.

Test results distinguish an accepted send request, a delivery/read receipt for
that exact message ID, and a new incoming message from the same recipient.
Old messages and history sync do not satisfy the receiving check. Results poll
the worker's archive; Cloud delivery and incoming evidence require webhooks.
The message ID can be copied and the test conversation opened for inspection.
The completed test keeps its exact sent text and dispatch time with timezone
beside those results, separately from the next editable draft. Local recipient
and text errors appear beside their fields; submitting invalid input focuses
the first field to correct without sending or querying the archive.
The latest run and validated recipient remain available when changing tools or
connection methods within this page. Stored evidence is scoped to its account
or session; only a new explicit test replaces it. Failed result reads label
the evidence as **Last known test results**, with **Check again** performing
only archive reads. Accepted Cloud composer sends keep their message ID and
selected message type when the new recipient becomes a stored conversation.
Cloud composers prioritize a visible conversation check after uncertainty;
**Send again** is secondary and Ctrl/Cmd+Enter cannot repeat an uncertain request.
Text-parameter template previews reflect the current entered values; Advanced
JSON displays the approved source text without claiming a rendered preview.
The acceptance receipt appears before the next message fields and is revealed
inside the composer when clipped. Passive archive updates do not move focus or
scroll. Editing a later draft or restoring an uncertain send hides the earlier
acceptance receipt. On narrow panes, recovery actions appear below their warning.
A failed result check displays its warning immediately above the evidence
checklist and retains the earlier proof. If an archive read fails before
sending, nothing is sent. Send failures and
timeouts are shown without automatic retries; check the conversation before
trying again when the outcome is unknown. An uncertain Cloud composer send
offers **Check this conversation** and changes the explicit send action to
**Send again**, warning that another submission may duplicate the first.

Within **Linked phone**, **Messages** and **Groups** search stored conversations,
read messages, and inspect participating groups and their members. Group names,
contact titles and member identities wrap on narrow panes, and the full group name is also
available in **About this group**. Offline
archives are labeled as stored history. Session activity and connection help
are collapsed until needed. On narrow panes, the Cloud test stays visible;
**Browse stored conversations** opens the archive separately. Switching connection
methods or phone tools offers **Keep editing**, **Discard draft and switch**,
or **Keep draft and switch** for an unsent message. Cancelling restores keyboard
focus to the retained editor or active tool when a tab initiated navigation,
and to the initiating control for other actions. Kept drafts, selected files and test evidence
exist only in page memory, without browser storage; closing or reloading the
page clears them. The plain-text native editor uses the inherited ADE font
and phone input size, and supports Ctrl/Cmd+Enter without treating IME
composition or repeated key events as send actions. Removing or logging out a native session clears its local
page state. A request in progress must finish before changing context.
With a partially configured Cloud account, **Browse stored conversations** keeps
the archive available for reading, and **Webhook tools** can stop an existing
tunnel. A temporary status error preserves message drafts and recovery controls.
Opening the page never starts a session or marks messages as read. Native
recipient and phone-pairing fields accept international numbers with spaces,
parentheses and hyphens; formatting is removed before validating the existing
7–15 digit range. Explicit WhatsApp JID validation is unchanged.

Use **Remove session** (the trash button next to the session selector) to stop
and delete the selected linked-device session. The confirmation names the
session and explains that saved credentials and its local message archive are
permanently removed. Cancel preserves the session. Removal does not remotely
unlink the device; use WhatsApp's **Linked devices** screen for that. The page
selects another available session, or shows session creation when none remain.
An uncertain error is shown without automatically repeating the removal.
On narrow panes, **Session actions** groups creation, refresh and removal.

Each session keeps a separate set of scoped database-worker tables with normalized text and media
labels from received messages, available WhatsApp history sync, and text/replies
sent through this worker. Media is not downloaded by the inbox. The archive
does not preserve view-once content or media keys and scrubs expired temporary
message bodies on archive reads and writes. `max_inbox_messages` bounds retention (default 10,000 per session);
lowering it publishes the new limit without deleting messages during reload.
The next successful archive write removes the oldest excess messages in that
archive's transaction. A rejected reload preserves all limits and stored rows.
Stored conversations can be read while disconnected. Participating groups and
their current membership require a connected session.

The archive starts when this worker version captures messages; it is not a
complete copy of the phone's history. **Request older messages** requests older
messages from the linked phone using an existing message as its anchor. Results
arrive asynchronously, depend on WhatsApp availability, and remain subject to
the retention limit. An empty conversation has no anchor to request history.

Use the page's settings action to edit the actual entry for this worker
instance. `whatsapp::configuration-id` resolves that ID in the worker's
namespace; the ADE host saves to the configuration service in `default`.
Environment templates are preserved by the form. Configuration changes are
re-fetched from the authoritative service; malformed reloads keep the last
valid settings and appear in `whatsapp::config-status`.

Legacy engines without `configuration::ensure` emit one compatibility warning
per configuration entry and process. Malformed reads never authorize a write;
only an explicit null value or `NOT_FOUND` permits the seed. Existing values,
including environment templates, remain authoritative.

Public operations remain visible in ADE traces. Internal callbacks are hidden
by default, and trace payload capture remains disabled to protect credentials
and message bodies.

Session/media limits and both timeouts apply to subsequent operations. A lower
session limit prevents additional creation without deleting existing accounts.
Changing `database` or `data_dir` requires restart. Changing the database target
does not copy existing keys or archives; use the documented migration process. `max_events`
requires restart because it sizes the native and Cloud event queues and native
session event rings. The page reports pending restart fields. Compose `config_override`
values remain authoritative on startup; remove an override if ADE should own
that setting. `--local-config` ignores central edits and reports local mode.

Keep all application copy, commands, accessibility labels and formatted dates
in English. User messages, group names and descriptions retain their original
content. The layout adapts to the pane width; archive details and group metadata
expand on demand. Pairing appears before recent activity, with optional connection
help at the end. The configuration form groups storage, capacity and time limits
in a single column while ADE owns saving and resetting the draft. Only settings
that require a worker restart carry a restart marker.

The page inherits ADE's typography: Inter for interface text and its existing
monospace face for identifiers and code. It does not request an external font.
Motion is limited to control feedback and respects reduced-motion preferences. Content does not
animate during scrolling or message updates. QR images are generated locally.

Pairing data stays in page memory, expires, and is cleared on session change.
The activity feed is bounded; the inbox is a separate persistent archive. The send
result identifies the protocol message; receipts establish later delivery.
After a send timeout, check activity before sending again.

For source edits, follow the [UI build instructions](development.md#build-and-run).
The development worker embeds the rebuilt assets; no frontend server is needed.
