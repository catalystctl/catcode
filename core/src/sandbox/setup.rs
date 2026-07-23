//! Runtime + image preparation guidance.
//!
//! The actual SDK install/pull calls live in [`microsandbox_backend`] (feature
//! `microsandbox`). This module documents the user-space-vs-admin split that
//! the setup flow enforces, and is compiled unconditionally so the protocol and
//! docs can reference it.

/// What CatCode may do automatically during sandbox setup (user-space only).
pub const MAY_AUTOMATE: &[&str] = &[
    "Download verified Microsandbox runtime assets into the CatCode cache",
    "Create CatCode cache directories",
    "Pull the configured OCI image",
    "Show download progress and retry failed downloads",
    "Recheck readiness after setup",
];

/// What CatCode must NOT do automatically (requires explicit user action).
pub const MUST_NOT_AUTOMATE: &[&str] = &[
    "Change BIOS/UEFI settings",
    "Enable Windows features without explicit user action",
    "Run sudo",
    "Modify group membership",
    "Install system packages",
    "Reboot the computer",
];
