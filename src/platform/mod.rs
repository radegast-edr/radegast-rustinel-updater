#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(windows)]
pub mod windows;

pub struct Platform {
    pub os: &'static str,
    pub arch: &'static str,
}

impl Platform {
    pub fn os_name(&self) -> &str { self.os }
    pub fn arch_name(&self) -> &str { self.arch }
    pub fn archive_name(&self) -> String {
        format!("{}-{}.zip", self.os, self.arch)
    }
}

pub fn current() -> Platform {
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "mac",
        "windows" => "windows",
        other => panic!("Unsupported OS: {other}"),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => if os == "mac" { "m5" } else { "arm64" },
        other => panic!("Unsupported architecture: {other}"),
    };
    Platform { os, arch }
}

pub fn default_rustinel_path() -> &'static str {
    #[cfg(target_os = "linux")]
    { "/opt/radegast/rustinel/rustinel" }
    #[cfg(target_os = "macos")]
    { "/Library/Radegast/rustinel/rustinel" }
    #[cfg(windows)]
    { r"C:\Program Files\Radegast\rustinel\rustinel\rustinel.exe" }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    { "/usr/local/bin/rustinel" }
}

pub fn stop_rustinel() -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    return linux::stop_service();
    #[cfg(target_os = "macos")]
    return macos::stop_service();
    #[cfg(windows)]
    return windows::stop_service();
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    return Ok(());
}

pub fn start_rustinel() -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    return linux::start_service();
    #[cfg(target_os = "macos")]
    return macos::start_service();
    #[cfg(windows)]
    return windows::start_service();
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    return Ok(());
}
