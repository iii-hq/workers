@pure @manifest
Feature: --manifest emits the registry payload
  The iii registry pipeline reads `email --manifest` on publish; the
  default config it embeds is what every fresh deployment starts with.
  Subprocess-driven; no engine needed.

  Scenario: --manifest emits valid JSON the registry pipeline can read
    When I run `email --manifest`
    Then the exit status is success
    And  stdout is valid JSON
    And  the manifest field "name" is "email"
    And  the manifest field "version" matches the crate version

  Scenario: the manifest lists every public function id
    When I run `email --manifest`
    Then the manifest lists exactly 8 functions
    And  the manifest lists the function "email::send"
    And  the manifest lists the function "email::accounts::list"
    And  the manifest lists the function "email::list"
    And  the manifest lists the function "email::get"
    And  the manifest lists the function "email::search"
    And  the manifest lists the function "email::flag"
    And  the manifest lists the function "email::move"
    And  the manifest lists the function "email::attachment::get"

  Scenario: the manifest declares the email::new-mail trigger type
    When I run `email --manifest`
    Then the manifest lists the trigger type "email::new-mail"

  Scenario: the manifest embeds the canonical default limits
    When I run `email --manifest`
    Then the manifest default limits.max_attachment_bytes is 26214400
    And  the manifest default limits.max_recipients is 100
    And  the manifest default limits.send_timeout_ms is 30000
    And  the manifest default limits.imap_connect_timeout_ms is 15000

  Scenario: --help describes the worker
    When I run `email --help`
    Then the exit status is success
    And  stdout mentions "SMTP send and real-time IMAP read"
