# Task: Add text-mode CLI commands

Read ARCHITECTURE_PIVOT.md first for context.

## Changes needed:

### 1. src/session.rs - Add new SessionCommand variants:
- `Shell` - opens interactive terminal session
- `Exec { command: String }` - runs a command, returns output
- `ClipboardGet` - gets remote clipboard text
- `ClipboardSet { text: String }` - sets remote clipboard text

Add dispatch implementations for each (stub responses like existing commands).

### 2. src/main.rs - Add new CLI subcommands:
- `shell` subcommand - no args, opens terminal
- `exec --command <CMD>` subcommand - runs command
- `clipboard get` subcommand - gets clipboard
- `clipboard set --text <TEXT>` subcommand - sets clipboard

Wire them through daemon like existing commands.

### 3. Update tests:
- Add unit tests in session.rs for new commands
- Add CLI integration tests in tests/cli_test.rs

Keep all existing commands. These are additions, not replacements.
