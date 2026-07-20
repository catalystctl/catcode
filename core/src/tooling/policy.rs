use crate::tooling::{metadata, ParallelSafety, ToolKind};

/// Classification is fail-closed: unknown/plugin tools are destructive until
/// their manifest supplies an explicit policy.
pub fn classify(name: &str) -> ToolKind {
    metadata(name)
        .map(|item| item.kind)
        .unwrap_or(ToolKind::Destructive)
}

pub fn is_parallel_wave_tool(name: &str) -> bool {
    metadata(name).is_some_and(|item| item.parallel == ParallelSafety::Safe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_tools_fail_closed_and_only_explicit_safe_tools_parallelize() {
        assert_eq!(classify("unknown"), ToolKind::Destructive);
        assert!(!is_parallel_wave_tool("unknown"));
        assert!(is_parallel_wave_tool("read_file"));
        assert!(!is_parallel_wave_tool("write_file"));
    }
}
