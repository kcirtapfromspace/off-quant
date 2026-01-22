//! Tailscale integration

use anyhow::{Context, Result};
use serde::Deserialize;
use std::process::Command;

/// Tailscale connection status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TailscaleStatus {
    /// Connected to tailnet
    Connected,
    /// Disconnected
    Disconnected,
    /// Not installed
    NotInstalled,
}

/// Tailscale status information
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TailscaleState {
    pub backend_state: String,
    #[serde(default)]
    pub tailscale_ips: Vec<String>,
    #[serde(default)]
    pub self_: Option<TailscaleSelf>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TailscaleSelf {
    #[serde(rename = "DNSName")]
    pub dns_name: String,
    pub host_name: String,
    #[serde(rename = "TailscaleIPs")]
    pub tailscale_ips: Vec<String>,
    pub online: bool,
}

/// Tailscale client for status and control
#[derive(Debug, Clone)]
pub struct TailscaleClient {
    tailscale_bin: String,
}

impl Default for TailscaleClient {
    fn default() -> Self {
        Self::new()
    }
}

impl TailscaleClient {
    pub fn new() -> Self {
        Self {
            tailscale_bin: "tailscale".to_string(),
        }
    }

    pub fn with_binary(path: impl Into<String>) -> Self {
        Self {
            tailscale_bin: path.into(),
        }
    }

    /// Check if Tailscale is installed
    pub fn is_installed(&self) -> bool {
        Command::new(&self.tailscale_bin)
            .arg("version")
            .output()
            .is_ok()
    }

    /// Get connection status
    pub fn status(&self) -> TailscaleStatus {
        if !self.is_installed() {
            return TailscaleStatus::NotInstalled;
        }

        match self.get_state() {
            Ok(state) if state.backend_state == "Running" => TailscaleStatus::Connected,
            _ => TailscaleStatus::Disconnected,
        }
    }

    /// Get detailed state information
    pub fn get_state(&self) -> Result<TailscaleState> {
        let output = Command::new(&self.tailscale_bin)
            .args(["status", "--json"])
            .output()
            .context("Failed to run tailscale status")?;

        if !output.status.success() {
            anyhow::bail!(
                "tailscale status failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        serde_json::from_slice(&output.stdout).context("Failed to parse tailscale status JSON")
    }

    /// Get IPv4 address on tailnet
    pub fn get_ipv4(&self) -> Result<String> {
        let output = Command::new(&self.tailscale_bin)
            .args(["ip", "-4"])
            .output()
            .context("Failed to run tailscale ip")?;

        if !output.status.success() {
            anyhow::bail!(
                "tailscale ip failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let ip = String::from_utf8_lossy(&output.stdout).trim().to_string();

        if ip.is_empty() {
            anyhow::bail!("No Tailscale IPv4 address found");
        }

        Ok(ip)
    }

    /// Get IPv6 address on tailnet
    pub fn get_ipv6(&self) -> Result<String> {
        let output = Command::new(&self.tailscale_bin)
            .args(["ip", "-6"])
            .output()
            .context("Failed to run tailscale ip -6")?;

        if !output.status.success() {
            anyhow::bail!(
                "tailscale ip -6 failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let ip = String::from_utf8_lossy(&output.stdout).trim().to_string();

        if ip.is_empty() {
            anyhow::bail!("No Tailscale IPv6 address found");
        }

        Ok(ip)
    }

    /// Get the DNS name for this machine
    pub fn get_dns_name(&self) -> Result<String> {
        let state = self.get_state()?;
        state
            .self_
            .map(|s| s.dns_name)
            .ok_or_else(|| anyhow::anyhow!("No DNS name found"))
    }

    /// Connect to tailnet (bring up)
    pub fn connect(&self) -> Result<()> {
        let output = Command::new(&self.tailscale_bin)
            .arg("up")
            .output()
            .context("Failed to run tailscale up")?;

        if !output.status.success() {
            anyhow::bail!(
                "tailscale up failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    /// Disconnect from tailnet (bring down)
    pub fn disconnect(&self) -> Result<()> {
        let output = Command::new(&self.tailscale_bin)
            .arg("down")
            .output()
            .context("Failed to run tailscale down")?;

        if !output.status.success() {
            anyhow::bail!(
                "tailscale down failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    /// Generate a shareable URL for a service
    pub fn service_url(&self, port: u16) -> Result<String> {
        let ip = self.get_ipv4()?;
        Ok(format!("http://{}:{}", ip, port))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_status() {
        let json = r#"{
            "BackendState": "Running",
            "TailscaleIPs": ["100.64.0.1"],
            "Self": {
                "DNSName": "macbook.tail1234.ts.net.",
                "HostName": "macbook",
                "TailscaleIPs": ["100.64.0.1"],
                "Online": true
            }
        }"#;

        let state: TailscaleState = serde_json::from_str(json).unwrap();
        assert_eq!(state.backend_state, "Running");
    }
}
