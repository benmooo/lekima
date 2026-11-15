# Sprint 6 — TUI correctness and accessibility

Status: **Complete** · Release pending

The plan of record is [`../tui-backlog.md`](../tui-backlog.md). This sprint took
every backlog item, including TUI-21 and the terminal-text hardening:

- **Responsiveness:** width-budgeted HUD, adaptive track columns, stacked
  lyrics at mid widths, list position cues, a help overlay that fits 80×24 and
  scrolls on smaller terminals.
- **Correctness:** the settings `TRANSLATION` row toggles translation, inactive
  selections stay visible, queue deletion asks for `y/N`, CJK/emoji widths are
  counted in cells, and ASCII mode no longer leaks Unicode glyphs.
- **Interaction:** count prefixes, `Ctrl-w 1..4` pane keys, a prompt editor with
  word delete and history, `LOADING`/`BUFFERING`/`MUTE` labels, buffered-range
  shading on the progress rail, a one-line notice chip, and a shared keymap
  table behind the help overlay.
- **Accessibility:** a `light` palette, `LEKIMA_THEME=light|dark|auto|<name>`,
  readable numeric text, and palette semantics that reserve `hot` for danger.
- **Queue UX:** queue mode keeps transport controls, follows the playing entry
  until the user moves, dims already-played rows, shows the remaining time,
  confirms deletions and keeps the queue open after `Enter`.
- **Gapless reliability:** preloads are idempotent and validated against the
  queue; boundaries carry track ids, stale boundaries stop and continue with
  the queue's next entry, FM ignores repeat wrapping, and loads/stops clear the
  boundary flag so audio and lyrics stay in sync. FM prefetch publishes queue
  events so the queue view receives appended batches.
- **Terminal hygiene:** `lekima_domain::sanitize_line` strips C0/C1, ANSI/OSC,
  bidi and zero-width controls; the NCM mapping layer, TUI and human CLI output
  apply it.

Evidence: `cargo test --workspace` (141 passed), `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo fmt --all --check`, and the
`lekima-tui` preview example at 60×20, 80×24 and 100×30.

Remaining work: none in this backlog. Buffered ranges are approximate for
variable-bitrate streams because the daemon maps bytes to time proportionally.
