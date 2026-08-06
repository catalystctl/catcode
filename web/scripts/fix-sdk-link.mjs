// postinstall no-op — the hub frontend no longer depends on the local
// `@catalyst-code/coding-agent` SDK (that was only needed for the chat/IDE
// core bridge). Kept so older install scripts that invoke this file still exit 0.
process.exit(0);
