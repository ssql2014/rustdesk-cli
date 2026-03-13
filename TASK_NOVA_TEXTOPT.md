# Task: Research RustDesk Protocol Text Optimizations (Nova — Researcher)

AFTER you finish the current terminal protocol research, pick this up.

## Research Questions:

1. **TerminalData compression** — the `compressed` field in TerminalData: what compression algorithm does RustDesk use? zstd? zlib? How is it toggled?

2. **Multiple terminal sessions** — does RustDesk support multiple simultaneous terminals via terminal_id? How does the server handle terminal_id > 0?

3. **Terminal persistent sessions** — OptionMessage has `terminal_persistent` BoolOption. What does this do? Can we reconnect to an existing terminal after disconnect?

4. **Clipboard protocol flow** — exact sequence of Clipboard/MultiClipboards/Cliprdr messages. Who initiates? Is it push or pull? Format types?

5. **Keystroke batching** — does RustDesk batch KeyEvents or send them individually? Any coalescing on the server side?

6. **TerminalData flow control** — is there backpressure? What happens if the remote command generates massive output (e.g., `cat /dev/urandom`)?

7. **ConnType implications** — does using DEFAULT_CONN vs a hypothetical terminal-only ConnType affect bandwidth? Does the server still try to send video frames?

## Output:
Write findings to RESEARCH.md as a new section (Section 13 or next available): "Terminal Protocol Optimizations"
