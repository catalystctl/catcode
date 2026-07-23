use super::*;
use crate::sandbox::error::CheckStatus;

fn check_codes(r: &SandboxPreflightReport) -> Vec<String> {
    r.checks.iter().map(|c| c.code.clone()).collect()
}

#[test]
fn linux_ready_when_kvm_present_and_rw() {
    let r = run_platform_preflight(&FakeProbe::linux_ready(), true);
    assert!(r.supported, "supported: {:?}", r.checks);
    assert!(r.ready, "ready: {:?}", r.checks);
    assert_eq!(r.platform, "linux");
}

#[test]
fn linux_kvm_missing_reports_hardware_virtualization_guidance() {
    let mut p = FakeProbe::linux_ready();
    p.kvm = KvmState::Missing;
    let r = run_platform_preflight(&p, true);
    assert!(!r.ready);
    // The exact remediation guidance lives in the setup actions.
    let actions = r
        .actions
        .iter()
        .map(|a| format!("{}|{}|{:?}", a.title, a.explanation, a.command))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        actions.contains("BIOS"),
        "expected BIOS guidance for missing KVM, got: {actions}"
    );
    assert!(
        actions.contains("Intel VT-x") || actions.contains("AMD-V"),
        "expected VT-x/AMD-V guidance, got: {actions}"
    );
}

#[test]
fn linux_kvm_permission_denied_reports_usermod_and_relogin() {
    let mut p = FakeProbe::linux_ready();
    p.kvm = KvmState::PermissionDenied;
    let r = run_platform_preflight(&p, true);
    assert!(!r.ready);
    let blob = r
        .actions
        .iter()
        .map(|a| format!("{}|{}|{:?}", a.title, a.explanation, a.command))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        blob.contains("usermod") || blob.contains("kvm"),
        "expected kvm group guidance, got: {blob}"
    );
}

#[test]
fn linux_kvm_modules_missing_offers_modprobe() {
    let mut p = FakeProbe::linux_ready();
    p.kvm = KvmState::ModulesMissing { intel: true };
    let r = run_platform_preflight(&p, true);
    assert!(!r.ready);
    let blob = r
        .actions
        .iter()
        .map(|a| a.command.clone().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        blob.contains("kvm_intel"),
        "expected kvm_intel modprobe guidance, got: {blob}"
    );
}

#[test]
fn linux_nested_virt_unavailable_is_unsupported() {
    let mut p = FakeProbe::linux_ready();
    p.nested_unavailable = true;
    let r = run_platform_preflight(&p, true);
    assert!(!r.ready);
    let codes = check_codes(&r);
    assert!(
        codes.iter().any(|c| c.contains("nested")),
        "expected nested-virt check, got: {codes:?}"
    );
}

#[test]
fn linux_no_cpu_virtualization_is_unsupported() {
    let mut p = FakeProbe::linux_ready();
    p.cpu_virt = VirtCapability::Absent;
    let r = run_platform_preflight(&p, true);
    assert!(!r.ready);
}

#[test]
fn macos_apple_silicon_ready() {
    let r = run_platform_preflight(&FakeProbe::macos_arm64_ready(), true);
    assert!(r.supported, "supported: {:?}", r.checks);
    assert!(r.ready, "ready: {:?}", r.checks);
    assert_eq!(r.platform, "macos");
    assert_eq!(r.architecture, "aarch64");
}

#[test]
fn macos_intel_unsupported_with_clear_message() {
    let mut p = FakeProbe::macos_arm64_ready();
    p.arch = "x86_64";
    p.apple_silicon = false;
    let r = run_platform_preflight(&p, true);
    assert!(!r.supported);
    assert!(!r.ready);
    let actions = r
        .actions
        .iter()
        .map(|a| a.explanation.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        actions.contains("Apple Silicon"),
        "expected Apple Silicon message, got: {actions}"
    );
    assert!(
        actions.contains("Intel Mac"),
        "expected explicit Intel-Mac unsupported message, got: {actions}"
    );
    // No "install another package" guidance.
    assert!(
        !r.actions
            .iter()
            .any(|a| a.command.is_some() && a.command.as_deref() == Some("brew install qemu")),
        "must not suggest installing QEMU for an Intel Mac"
    );
}

#[test]
fn windows_whp_disabled_reports_enable_command() {
    let p = FakeProbe {
        os: "windows",
        arch: "x86_64",
        kvm: KvmState::NotApplicable,
        cpu_virt: VirtCapability::Present,
        nested_unavailable: false,
        apple_silicon: false,
        whp: WhpState::Disabled,
    };
    let r = run_platform_preflight(&p, true);
    assert!(!r.ready);
    let blob = r
        .actions
        .iter()
        .map(|a| a.command.clone().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        blob.contains("HypervisorPlatform"),
        "expected WHP enable guidance, got: {blob}"
    );
    let admin = r
        .actions
        .iter()
        .any(|a| a.requires_admin && a.title.contains("Hypervisor"));
    assert!(admin, "expected a requires_admin WHP action");
}

#[test]
fn windows_restart_pending_is_not_ready() {
    let p = FakeProbe {
        os: "windows",
        arch: "x86_64",
        kvm: KvmState::NotApplicable,
        cpu_virt: VirtCapability::Present,
        nested_unavailable: false,
        apple_silicon: false,
        whp: WhpState::RestartPending,
    };
    let r = run_platform_preflight(&p, true);
    assert!(!r.ready);
    let detail = r
        .checks
        .iter()
        .map(|c| c.detail.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        detail.contains("restart"),
        "expected restart guidance, got: {detail}"
    );
}

#[test]
fn unsupported_architecture_is_unsupported() {
    let mut p = FakeProbe::linux_ready();
    p.arch = "mips";
    let r = run_platform_preflight(&p, true);
    assert!(!r.supported);
    assert!(!r.ready);
}

#[test]
fn not_requested_is_not_required() {
    let r = run_platform_preflight(&FakeProbe::linux_ready(), false);
    assert!(!r.requested);
}
