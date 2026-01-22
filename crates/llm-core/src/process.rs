//! Process management for Ollama

use anyhow::{Context, Result};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// Manages the Ollama process lifecycle
pub struct OllamaProcess {
    child: Option<Child>,
    host: String,
    port: u16,
    ollama_home: String,
}

impl OllamaProcess {
    /// Create a new process manager
    pub fn new(host: &str, port: u16, ollama_home: &str) -> Self {
        Self {
            child: None,
            host: host.to_string(),
            port,
            ollama_home: ollama_home.to_string(),
        }
    }

    /// Check if the process is running
    pub fn is_running(&mut self) -> bool {
        match &mut self.child {
            Some(child) => child.try_wait().ok().flatten().is_none(),
            None => false,
        }
    }

    /// Start the Ollama server
    pub fn start(&mut self) -> Result<()> {
        if self.is_running() {
            anyhow::bail!("Ollama is already running");
        }

        let ollama_bin = find_ollama_binary()?;

        let child = Command::new(&ollama_bin)
            .arg("serve")
            .env("OLLAMA_HOST", format!("{}:{}", self.host, self.port))
            .env("OLLAMA_HOME", &self.ollama_home)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed to start Ollama")?;

        self.child = Some(child);
        Ok(())
    }

    /// Stop the Ollama server
    pub fn stop(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            // Try graceful shutdown first
            #[cfg(unix)]
            unsafe {
                libc::kill(child.id() as i32, libc::SIGTERM);
            }

            // Wait for graceful shutdown
            let timeout = Duration::from_secs(5);
            let start = std::time::Instant::now();

            loop {
                match child.try_wait() {
                    Ok(Some(_)) => return Ok(()),
                    Ok(None) if start.elapsed() < timeout => {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    _ => break,
                }
            }

            // Force kill if still running
            let _ = child.kill();
            let _ = child.wait();
        }

        Ok(())
    }

    /// Restart the Ollama server
    pub fn restart(&mut self) -> Result<()> {
        self.stop()?;
        std::thread::sleep(Duration::from_millis(500));
        self.start()
    }

    /// Get the process ID if running
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(|c| c.id())
    }

    /// Update the host binding (requires restart)
    pub fn set_host(&mut self, host: &str) {
        self.host = host.to_string();
    }
}

impl Drop for OllamaProcess {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Find the Ollama binary
pub fn find_ollama_binary() -> Result<String> {
    // Check common locations
    let candidates = [
        "/usr/local/bin/ollama",
        "/opt/homebrew/bin/ollama",
        "/usr/bin/ollama",
    ];

    for path in candidates {
        if Path::new(path).exists() {
            return Ok(path.to_string());
        }
    }

    // Try PATH
    if Command::new("which")
        .arg("ollama")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Ok("ollama".to_string());
    }

    anyhow::bail!("Ollama binary not found. Install with: brew install ollama")
}

/// Check if a port is in use
pub fn is_port_in_use(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_err()
}

/// Find what process is using a port (macOS)
#[cfg(target_os = "macos")]
pub fn find_process_using_port(port: u16) -> Result<Option<String>> {
    let output = Command::new("lsof")
        .args(["-i", &format!(":{}", port), "-t"])
        .output()
        .context("Failed to run lsof")?;

    if output.status.success() {
        let pid = String::from_utf8_lossy(&output.stdout).trim().to_string();

        if !pid.is_empty() {
            // Get process name
            let ps_output = Command::new("ps")
                .args(["-p", &pid, "-o", "comm="])
                .output()?;

            let name = String::from_utf8_lossy(&ps_output.stdout)
                .trim()
                .to_string();

            return Ok(Some(format!("{} (PID {})", name, pid)));
        }
    }

    Ok(None)
}

#[cfg(not(target_os = "macos"))]
pub fn find_process_using_port(_port: u16) -> Result<Option<String>> {
    Ok(None)
}

/// Get system memory info
#[cfg(target_os = "macos")]
pub fn get_memory_info() -> Result<MemoryInfo> {
    let output = Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .context("Failed to get memory size")?;

    let total_bytes: u64 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .context("Failed to parse memory size")?;

    // Get memory pressure (simplified - real impl would use vm_stat)
    let output = Command::new("memory_pressure").output();

    let pressure = match output {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            if text.contains("normal") {
                MemoryPressure::Normal
            } else if text.contains("warn") {
                MemoryPressure::Warning
            } else {
                MemoryPressure::Critical
            }
        }
        _ => MemoryPressure::Unknown,
    };

    Ok(MemoryInfo {
        total_gb: total_bytes / (1024 * 1024 * 1024),
        available_gb: 0, // Would need vm_stat parsing
        pressure,
    })
}

#[cfg(not(target_os = "macos"))]
pub fn get_memory_info() -> Result<MemoryInfo> {
    anyhow::bail!("get_memory_info not implemented for this platform")
}

#[derive(Debug, Clone)]
pub struct MemoryInfo {
    pub total_gb: u64,
    pub available_gb: u64,
    pub pressure: MemoryPressure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryPressure {
    Normal,
    Warning,
    Critical,
    Unknown,
}
