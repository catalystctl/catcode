//! Environment preflight for Microsandbox.
//!
//! Determines whether the host can run a local microVM, with exact, actionable
//! remediation text per platform. Platform detection is injectable via
//! [`PlatformProbe`] so unit tests assert the remediation text without KVM /
//! Apple Silicon / WHP. The real probe ([`RealProbe`]) reads the host directly.
use super::error::{
    error_codes, CheckStatus, SandboxPreflightCheck, SandboxPreflightReport, SandboxSetupAction,
};

/// Injectable host-facts probe. Tests supply a fake; production uses [`RealProbe`].
pub trait PlatformProbe: Send + Sync {
    fn os(&self) -> &'static str;
    fn arch(&self) -> &'static str;
    /// Linux: `/dev/kvm` state. Non-Linux: [`KvmState::NotApplicable`].
    fn kvm(&self) -> KvmState;
    /// Linux: CPU virtualization capability (VT-x / AMD-V via cpu flags).
    fn cpu_virt(&self) -> VirtCapability;
    /// Whether the process appears to be inside a VM without nested virt.
    fn nested_virt_unavailable(&self) -> bool;
    /// macOS: Apple Silicon (aarch64) vs Intel.
    fn apple_silicon(&self) -> bool;
    /// Windows: Windows Hypervisor Platform state.
    fn whp(&self) -> WhpState;
    /// Windows: a restart is pending for a WHP enable.
    fn restart_pending(&self) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KvmState {
    NotApplicable,
    Ready,
    Missing,
    ModulesMissing { intel: bool },
    PermissionDenied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VirtCapability {
    NotApplicable,
    Present,
    Absent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WhpState {
    NotApplicable,
    Enabled,
    Disabled,
    RestartPending,
    Unknown,
}

/// The real probe — reads the host. Cheap, synchronous, never panics.
pub struct RealProbe;

impl PlatformProbe for RealProbe {
    fn os(&self) -> &'static str {
        std::env::consts::OS
    }
    fn arch(&self) -> &'static str {
        std::env::consts::ARCH
    }
    fn kvm(&self) -> KvmState {
        #[cfg(target_os = "linux")]
        {
            return linux_kvm_state();
        }
        #[cfg(not(target_os = "linux"))]
        {
            KvmState::NotApplicable
        }
    }
    fn cpu_virt(&self) -> VirtCapability {
        #[cfg(target_os = "linux")]
        {
            return linux_cpu_virt();
        }
        #[cfg(not(target_os = "linux"))]
        {
            VirtCapability::NotApplicable
        }
    }
    fn nested_virt_unavailable(&self) -> bool {
        // Heuristic: inside a hypervisor without nested virt exposes no KVM.
        #[cfg(target_os = "linux")]
        {
            matches!(
                self.kvm(),
                KvmState::Missing | KvmState::ModulesMissing { .. }
            ) && self.cpu_virt() == VirtCapability::Present
                && likely_in_vm()
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }
    fn apple_silicon(&self) -> bool {
        cfg!(target_arch = "aarch64") && cfg!(target_os = "macos")
    }
    fn whp(&self) -> WhpState {
        #[cfg(target_os = "windows")]
        {
            return windows_whp_state();
        }
        #[cfg(not(target_os = "windows"))]
        {
            WhpState::NotApplicable
        }
    }
    fn restart_pending(&self) -> bool {
        // The WHP restart-pending check is folded into whp() on Windows.
        false
    }
}

/// Injectable probe for tests — mirrors [`RealProbe`] but with fixed fields so
/// preflight logic can be exercised without real virtualization. Construct via
/// [`FakeProbe::linux_ready`] etc. or by setting fields directly.
#[derive(Clone, Debug)]
pub struct FakeProbe {
    pub os: &'static str,
    pub arch: &'static str,
    pub kvm: KvmState,
    pub cpu_virt: VirtCapability,
    pub nested_unavailable: bool,
    pub apple_silicon: bool,
    pub whp: WhpState,
}

impl FakeProbe {
    pub fn linux_ready() -> Self {
        Self {
            os: "linux",
            arch: "x86_64",
            kvm: KvmState::Ready,
            cpu_virt: VirtCapability::Present,
            nested_unavailable: false,
            apple_silicon: false,
            whp: WhpState::NotApplicable,
        }
    }
    pub fn macos_arm64_ready() -> Self {
        Self {
            os: "macos",
            arch: "aarch64",
            kvm: KvmState::NotApplicable,
            cpu_virt: VirtCapability::NotApplicable,
            nested_unavailable: false,
            apple_silicon: true,
            whp: WhpState::NotApplicable,
        }
    }
}

impl PlatformProbe for FakeProbe {
    fn os(&self) -> &'static str {
        self.os
    }
    fn arch(&self) -> &'static str {
        self.arch
    }
    fn kvm(&self) -> KvmState {
        self.kvm
    }
    fn cpu_virt(&self) -> VirtCapability {
        self.cpu_virt
    }
    fn nested_virt_unavailable(&self) -> bool {
        self.nested_unavailable
    }
    fn apple_silicon(&self) -> bool {
        self.apple_silicon
    }
    fn whp(&self) -> WhpState {
        self.whp
    }
    fn restart_pending(&self) -> bool {
        matches!(self.whp, WhpState::RestartPending)
    }
}

#[cfg(target_os = "linux")]
fn linux_kvm_state() -> KvmState {
    use std::os::unix::fs::MetadataExt;
    let path = std::path::Path::new("/dev/kvm");
    match std::fs::metadata(path) {
        Ok(md) => {
            let mode = md.mode();
            // Require read+write for the current user.
            let euid = unsafe { libc::geteuid() };
            let owner = (mode & 0o0700) >> 6;
            let group = (mode & 0o070) >> 3;
            let other = mode & 0o07;
            let my_bits = if euid == md.uid() {
                owner
            } else {
                // group/other: approximate (we don't resolve group membership here).
                group | other
            };
            if my_bits & 0o6 == 0o6 {
                KvmState::Ready
            } else {
                KvmState::PermissionDenied
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // /dev/kvm absent — either modules not loaded or virt disabled.
            if linux_cpu_virt() == VirtCapability::Present {
                KvmState::ModulesMissing {
                    intel: cpu_is_intel(),
                }
            } else {
                KvmState::Missing
            }
        }
        Err(_) => KvmState::Missing,
    }
}

#[cfg(target_os = "linux")]
fn linux_cpu_virt() -> VirtCapability {
    let flags = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let has_vmx = flags.contains("vmx");
    let has_svm = flags.contains("svm");
    if has_vmx || has_svm {
        VirtCapability::Present
    } else {
        VirtCapability::Absent
    }
}

#[cfg(target_os = "linux")]
fn cpu_is_intel() -> bool {
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    cpuinfo.contains("GenuineIntel")
}

#[cfg(target_os = "linux")]
fn likely_in_vm() -> bool {
    // /sys/hypervisor/type or DMI product name containing "KVM"/"VMware"/"Hyper"/"QEMU".
    if std::fs::read_to_string("/sys/hypervisor/type")
        .map(|s| s.trim().eq_ignore_ascii_case("kvm") || s.trim().eq_ignore_ascii_case("xen"))
        .unwrap_or(false)
    {
        return true;
    }
    if let Ok(p) = std::fs::read_to_string("/sys/class/dmi/id/product_name") {
        let p = p.to_ascii_lowercase();
        if p.contains("kvm") || p.contains("vmware") || p.contains("hyper-v") || p.contains("qemu")
        {
            return true;
        }
    }
    false
}

#[cfg(target_os = "windows")]
fn windows_whp_state() -> WhpState {
    // Best-effort: check the optional feature state via PowerShell. This runs
    // `powershell -Command Get-WindowsOptionalFeature`; on failure → Unknown.
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg("Get-WindowsOptionalFeature -Online -FeatureName HypervisorPlatform | Select-Object -ExpandProperty State")
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout)
                .trim()
                .to_ascii_lowercase();
            if s.contains("enabled") {
                WhpState::Enabled
            } else if s.contains("restart") || s.contains("pending") {
                WhpState::RestartPending
            } else {
                WhpState::Disabled
            }
        }
        _ => WhpState::Unknown,
    }
}

/// Run preflight against a probe + config. Produces the full report. The
/// runtime/image readiness checks are added by the manager (which has the SDK);
/// this function covers platform + virtualization support only.
pub fn run_platform_preflight(
    probe: &dyn PlatformProbe,
    requested: bool,
) -> SandboxPreflightReport {
    let mut checks = Vec::new();
    let mut actions = Vec::new();
    let os = probe.os();
    let arch = probe.arch();
    let mut supported = true;

    // --- platform / architecture ---
    let (plat_ok, plat_detail) = match os {
        "linux" => (true, "Linux".to_string()),
        "macos" => (true, "macOS".to_string()),
        "windows" => (true, "Windows".to_string()),
        other => (false, format!("unsupported OS: {other}")),
    };
    if !plat_ok {
        supported = false;
        checks.push(fail(
            error_codes::UNSUPPORTED_PLATFORM,
            "Platform",
            plat_detail,
        ));
        actions.push(action(
            "Use a supported platform",
            "Microsandbox runs on Linux, Apple-Silicon macOS, or Windows. This build does not support the current OS.",
            None,
            false,
            false,
        ));
    } else {
        checks.push(info("Platform", plat_detail));
    }

    let arch_ok = matches!(arch, "x86_64" | "aarch64");
    if !arch_ok {
        supported = false;
        checks.push(fail(
            error_codes::UNSUPPORTED_ARCHITECTURE,
            "Architecture",
            format!("unsupported architecture: {arch}"),
        ));
        actions.push(action(
            "Use a supported architecture",
            "Microsandbox supports x86_64 and aarch64 (Apple Silicon).",
            None,
            false,
            false,
        ));
    } else {
        checks.push(info("Architecture", arch.to_string()));
    }

    // --- platform-specific virtualization ---
    match os {
        "linux" => linux_checks(probe, &mut checks, &mut actions),
        "macos" => macos_checks(probe, &mut checks, &mut actions, &mut supported),
        "windows" => windows_checks(probe, &mut checks, &mut actions),
        _ => {}
    }

    let ready = supported && checks.iter().all(|c| c.status != CheckStatus::Fail);
    SandboxPreflightReport {
        requested,
        supported,
        ready,
        platform: os.to_string(),
        architecture: arch.to_string(),
        checks,
        actions,
    }
}

fn linux_checks(
    probe: &dyn PlatformProbe,
    checks: &mut Vec<SandboxPreflightCheck>,
    actions: &mut Vec<SandboxSetupAction>,
) {
    let virt = probe.cpu_virt();
    match virt {
        VirtCapability::Present => {
            checks.push(pass("CPU virtualization", "VT-x / AMD-V available"))
        }
        VirtCapability::Absent => {
            checks.push(fail(
                error_codes::VIRTUALIZATION_DISABLED,
                "CPU virtualization",
                "not detected",
            ));
            actions.push(action(
                "Enable hardware virtualization in BIOS/UEFI",
                "Hardware virtualization may be disabled. Enable Intel VT-x or AMD-V in BIOS/UEFI, then boot Linux again.",
                None,
                true,
                true,
            ));
        }
        VirtCapability::NotApplicable => {}
    }
    match probe.kvm() {
        KvmState::Ready => checks.push(pass("KVM device", "/dev/kvm read/write")),
        KvmState::Missing => {
            checks.push(fail(
                error_codes::KVM_DEVICE_MISSING,
                "KVM device",
                "/dev/kvm not found",
            ));
            actions.push(action(
                "Enable hardware virtualization",
                "Hardware virtualization may be disabled. Enable Intel VT-x or AMD-V in BIOS/UEFI, then boot Linux again.",
                None,
                true,
                true,
            ));
        }
        KvmState::ModulesMissing { intel } => {
            let modprobe = if intel {
                "sudo modprobe kvm && sudo modprobe kvm_intel"
            } else {
                "sudo modprobe kvm && sudo modprobe kvm_amd"
            };
            checks.push(fail(
                error_codes::KVM_MODULES_MISSING,
                "KVM modules",
                "kvm kernel modules not loaded",
            ));
            actions.push(action(
                "Load the KVM kernel modules",
                "The CPU supports virtualization but the KVM modules are not loaded.",
                Some(modprobe.to_string()),
                true,
                false,
            ));
        }
        KvmState::PermissionDenied => {
            checks.push(fail(
                error_codes::KVM_PERMISSION_DENIED,
                "KVM device",
                "/dev/kvm not readable/writable by this user",
            ));
            actions.push(action(
                "Add your user to the kvm group",
                "Your user lacks read/write access to /dev/kvm. After this you must sign out and back in (or reboot) for the group change to take effect.",
                Some("sudo usermod -aG kvm \"$USER\"".to_string()),
                true,
                false,
            ));
            actions.push(action(
                "Verify KVM readiness",
                "After re-logging in, confirm KVM is usable:",
                Some("test -r /dev/kvm && test -w /dev/kvm && echo \"KVM is ready\"".to_string()),
                false,
                false,
            ));
        }
        KvmState::NotApplicable => {}
    }
    if probe.nested_virt_unavailable() {
        checks.push(fail(
            error_codes::NESTED_VIRTUALIZATION_UNAVAILABLE,
            "Nested virtualization",
            "running inside a VM without nested virtualization enabled",
        ));
        actions.push(action(
            "Enable nested virtualization in the outer hypervisor",
            "CatCode is running inside a virtual machine, and nested virtualization is not enabled. Enable it in the outer hypervisor (e.g. 'Enable nested virtualization' on the host hypervisor), then restart the VM.",
            None,
            true,
            true,
        ));
    }
}

fn macos_checks(
    probe: &dyn PlatformProbe,
    checks: &mut Vec<SandboxPreflightCheck>,
    actions: &mut Vec<SandboxSetupAction>,
    supported: &mut bool,
) {
    if probe.apple_silicon() {
        checks.push(pass("Apple Silicon", "aarch64 (Apple Silicon)"));
        // The built-in hypervisor framework requires no external package.
        checks.push(info(
            "Hypervisor",
            "Apple Virtualization framework (no external package required)",
        ));
    } else {
        *supported = false;
        checks.push(fail(
            error_codes::INTEL_MACOS_UNSUPPORTED,
            "Apple Silicon",
            "Intel macOS is not supported",
        ));
        actions.push(action(
            "Use an Apple-Silicon Mac",
            "Microsandbox requires Apple Silicon on macOS. This Intel Mac cannot run the local Microsandbox backend. No external package fixes an unsupported Intel Mac.",
            None,
            false,
            false,
        ));
    }
}

fn windows_checks(
    probe: &dyn PlatformProbe,
    checks: &mut Vec<SandboxPreflightCheck>,
    actions: &mut Vec<SandboxSetupAction>,
) {
    match probe.whp() {
        WhpState::Enabled => checks.push(pass("Windows Hypervisor Platform", "enabled")),
        WhpState::Disabled => {
            checks.push(fail(
                error_codes::WHP_DISABLED,
                "Windows Hypervisor Platform",
                "disabled",
            ));
            actions.push(action(
                "Enable Windows Hypervisor Platform",
                "Run this in an Administrator PowerShell. A restart may be required, and hardware virtualization must also be enabled in BIOS/UEFI.",
                Some("Enable-WindowsOptionalFeature -Online -FeatureName HypervisorPlatform -All".to_string()),
                true,
                true,
            ));
            actions.push(action(
                "Check WHP status",
                "After enabling, verify the feature state:",
                Some(
                    "Get-WindowsOptionalFeature -Online -FeatureName HypervisorPlatform"
                        .to_string(),
                ),
                false,
                false,
            ));
        }
        WhpState::RestartPending => {
            checks.push(fail(
                error_codes::WHP_RESTART_PENDING,
                "Windows Hypervisor Platform",
                "enabled but a restart is pending",
            ));
            actions.push(action(
                "Restart Windows",
                "Windows Hypervisor Platform is enabled but a restart is required before it takes effect.",
                None,
                false,
                true,
            ));
        }
        WhpState::Unknown => {
            checks.push(warn(
                "whp_state",
                "Windows Hypervisor Platform",
                "unable to determine WHP state (run Get-WindowsOptionalFeature manually)",
            ));
            actions.push(action(
                "Check Windows Hypervisor Platform",
                "Verify the feature state manually:",
                Some(
                    "Get-WindowsOptionalFeature -Online -FeatureName HypervisorPlatform"
                        .to_string(),
                ),
                false,
                false,
            ));
        }
        WhpState::NotApplicable => {}
    }
}

// --- small constructors ---
fn pass(title: &str, detail: impl Into<String>) -> SandboxPreflightCheck {
    SandboxPreflightCheck {
        code: title.to_ascii_lowercase().replace(' ', "_"),
        title: title.to_string(),
        status: CheckStatus::Pass,
        detail: detail.into(),
    }
}
fn fail(code: &str, title: &str, detail: impl Into<String>) -> SandboxPreflightCheck {
    SandboxPreflightCheck {
        code: code.to_string(),
        title: title.to_string(),
        status: CheckStatus::Fail,
        detail: detail.into(),
    }
}
fn info(title: &str, detail: impl Into<String>) -> SandboxPreflightCheck {
    SandboxPreflightCheck {
        code: title.to_ascii_lowercase().replace(' ', "_"),
        title: title.to_string(),
        status: CheckStatus::Info,
        detail: detail.into(),
    }
}
fn warn(code: &str, title: &str, detail: impl Into<String>) -> SandboxPreflightCheck {
    SandboxPreflightCheck {
        code: code.to_string(),
        title: title.to_string(),
        status: CheckStatus::Warn,
        detail: detail.into(),
    }
}

/// Public re-export for the [`ExecutionBackend::status`] default and manager.
pub use super::error::SandboxPreflightReport as PreflightReport;

pub(crate) fn action(
    title: &str,
    explanation: &str,
    command: Option<String>,
    admin: bool,
    reboot: bool,
) -> super::error::SandboxSetupAction {
    super::error::SandboxSetupAction {
        title: title.to_string(),
        explanation: explanation.to_string(),
        command,
        requires_admin: admin,
        requires_reboot: reboot,
    }
}

#[cfg(test)]
mod tests;
