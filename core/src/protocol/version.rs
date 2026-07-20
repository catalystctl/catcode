pub const PROTOCOL_VERSION: u32 = 2;

pub const CAPABILITIES: &[&str] = &[
    "protocol_v2",
    "run_ids",
    "session_ids",
    "event_sequence",
    "stale_event_rejection",
    "worktree",
    "checkpoint",
    "file_change",
    "audit",
    "routing",
    "allow_pattern",
    "cost_update",
];
