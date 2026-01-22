//! Web fetch tool for retrieving web content

use anyhow::Result;
use async_trait::async_trait;
use scraper::{Html, Selector};
use serde_json::Value;
use std::net::{IpAddr, ToSocketAddrs};
use std::sync::OnceLock;
use std::time::Duration;
use tracing::{debug, instrument, warn};

use crate::tools::{ParameterProperty, ParameterSchema, SecurityLevel, Tool, ToolContext, ToolResult};

/// Shared HTTP client for connection pooling
/// Using OnceLock for lazy initialization with a longer timeout for general use
static SHARED_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn get_shared_client() -> &'static reqwest::Client {
    SHARED_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(Duration::from_secs(90))
            .timeout(Duration::from_secs(30))
            .user_agent("Mozilla/5.0 (compatible; QuantCLI/1.0)")
            .build()
            .expect("Failed to create HTTP client")
    })
}

/// Check if an IP address is in a private/reserved range (SSRF protection)
fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            ipv4.is_loopback()           // 127.0.0.0/8
                || ipv4.is_private()     // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
                || ipv4.is_link_local()  // 169.254.0.0/16
                || ipv4.is_broadcast()   // 255.255.255.255
                || ipv4.is_unspecified() // 0.0.0.0
                || ipv4.octets()[0] == 100 && (ipv4.octets()[1] & 0xC0) == 64  // 100.64.0.0/10 (CGNAT)
        }
        IpAddr::V6(ipv6) => {
            ipv6.is_loopback()           // ::1
                || ipv6.is_unspecified() // ::
                // Check for IPv4-mapped addresses
                || ipv6.to_ipv4_mapped().map(|v4| {
                    v4.is_loopback() || v4.is_private() || v4.is_link_local()
                }).unwrap_or(false)
        }
    }
}

/// Tool for fetching web content
pub struct WebFetchTool;

impl WebFetchTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch content from a URL. Returns the page content as text, converting HTML to readable text."
    }

    fn security_level(&self) -> SecurityLevel {
        SecurityLevel::Moderate
    }

    fn parameters_schema(&self) -> ParameterSchema {
        ParameterSchema::new()
            .with_required("url", ParameterProperty::string("The URL to fetch"))
            .with_property("raw", ParameterProperty::boolean("Return raw HTML instead of extracted text (default: false)"))
            .with_property("selector", ParameterProperty::string("CSS selector to extract specific content"))
    }

    #[instrument(skip(self, args, ctx), fields(url = tracing::field::Empty))]
    async fn execute(&self, args: &Value, ctx: &ToolContext) -> Result<ToolResult> {
        let url = args.get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: url"))?;

        // Record URL in span (truncate for safety)
        tracing::Span::current().record("url", &url.chars().take(100).collect::<String>().as_str());

        let raw = args.get("raw")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let selector = args.get("selector")
            .and_then(|v| v.as_str());

        debug!(raw, selector = ?selector, timeout_secs = ctx.http_timeout_secs, "Fetch parameters");

        // Use shared client for connection pooling
        let client = get_shared_client();

        // Validate URL
        let parsed_url = match url::Url::parse(url) {
            Ok(u) => u,
            Err(e) => {
                warn!(url, error = %e, "Invalid URL");
                return Ok(ToolResult::error(format!("Invalid URL: {}", e)));
            }
        };

        // Only allow HTTP(S)
        if parsed_url.scheme() != "http" && parsed_url.scheme() != "https" {
            return Ok(ToolResult::error("Only HTTP and HTTPS URLs are supported"));
        }

        // P1 Security: SSRF protection - block private/reserved IP ranges
        if let Some(host) = parsed_url.host_str() {
            // Try to resolve hostname to check IP addresses
            let port = parsed_url.port().unwrap_or(if parsed_url.scheme() == "https" { 443 } else { 80 });
            let addr_str = format!("{}:{}", host, port);

            if let Ok(addrs) = addr_str.to_socket_addrs() {
                for addr in addrs {
                    if is_private_ip(&addr.ip()) {
                        warn!(host, ip = %addr.ip(), "SSRF protection blocked private IP");
                        return Ok(ToolResult::error(format!(
                            "SSRF protection: Access to private/reserved IP address {} is blocked",
                            addr.ip()
                        )));
                    }
                }
            }
            // If resolution fails, we'll let the actual fetch handle it
        }

        // Fetch the URL with per-request timeout from context
        debug!("Sending HTTP request");
        let response = match client
            .get(url)
            .timeout(Duration::from_secs(ctx.http_timeout_secs))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "Failed to fetch URL");
                return Ok(ToolResult::error(format!("Failed to fetch URL: {}", e)));
            }
        };

        let status = response.status();
        debug!(status = %status, "HTTP response received");
        if !status.is_success() {
            warn!(status = %status, "HTTP error response");
            return Ok(ToolResult::error(format!("HTTP error: {}", status)));
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();

        let body = match response.text().await {
            Ok(b) => b,
            Err(e) => return Ok(ToolResult::error(format!("Failed to read response: {}", e))),
        };

        // Process based on content type and options
        let output = if raw {
            body
        } else if content_type.contains("text/html") {
            if let Some(sel) = selector {
                extract_with_selector(&body, sel)?
            } else {
                html_to_text(&body)
            }
        } else if content_type.contains("application/json") {
            // Pretty print JSON
            match serde_json::from_str::<Value>(&body) {
                Ok(json) => serde_json::to_string_pretty(&json).unwrap_or(body),
                Err(_) => body,
            }
        } else {
            // Return as-is for other content types
            body
        };

        // Truncate if too long (UTF-8 safe)
        let output = if output.len() > ctx.max_output_len {
            // Find a safe truncation point at a char boundary
            let safe_end = output
                .char_indices()
                .take_while(|(idx, _)| *idx < ctx.max_output_len)
                .last()
                .map(|(idx, c)| idx + c.len_utf8())
                .unwrap_or(0);
            format!(
                "{}\n\n[Content truncated at {} characters]",
                &output[..safe_end],
                safe_end
            )
        } else {
            output
        };

        Ok(ToolResult::success(output))
    }
}

/// Convert HTML to plain text
fn html_to_text(html: &str) -> String {
    let document = Html::parse_document(html);

    // Try to find main content
    let selectors = [
        "article",
        "main",
        "[role='main']",
        ".content",
        "#content",
        ".post",
        ".article",
        "body",
    ];

    for sel_str in selectors {
        if let Ok(selector) = Selector::parse(sel_str) {
            if let Some(element) = document.select(&selector).next() {
                let text = extract_text_from_element(&element);
                if !text.trim().is_empty() {
                    return text;
                }
            }
        }
    }

    // Fallback to body
    if let Ok(body_selector) = Selector::parse("body") {
        if let Some(body) = document.select(&body_selector).next() {
            return extract_text_from_element(&body);
        }
    }

    // Last resort: just extract all text
    document.root_element().text().collect::<Vec<_>>().join(" ")
}

/// Extract text from an HTML element, preserving some structure
fn extract_text_from_element(element: &scraper::ElementRef) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut current_line = String::new();
    let mut skip_depth = 0;

    for edge in element.traverse() {
        match edge {
            ego_tree::iter::Edge::Open(node) => {
                if skip_depth > 0 {
                    if node.value().is_element() {
                        skip_depth += 1;
                    }
                    continue;
                }

                match node.value() {
                    scraper::Node::Text(text) => {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            if !current_line.is_empty() {
                                current_line.push(' ');
                            }
                            current_line.push_str(trimmed);
                        }
                    }
                    scraper::Node::Element(el) => {
                        let tag = el.name();
                        // Skip script and style content
                        if matches!(tag, "script" | "style" | "noscript") {
                            skip_depth = 1;
                            continue;
                        }
                        // Add line breaks for block elements
                        if matches!(tag, "p" | "div" | "br" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "tr") {
                            if !current_line.is_empty() {
                                lines.push(current_line.clone());
                                current_line.clear();
                            }
                        }
                    }
                    _ => {}
                }
            }
            ego_tree::iter::Edge::Close(node) => {
                if skip_depth > 0 && node.value().is_element() {
                    skip_depth -= 1;
                }
            }
        }
    }

    if !current_line.is_empty() {
        lines.push(current_line);
    }

    // Clean up empty lines and excessive whitespace
    lines
        .into_iter()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Extract content using a CSS selector
fn extract_with_selector(html: &str, selector_str: &str) -> Result<String> {
    let document = Html::parse_document(html);
    let selector = Selector::parse(selector_str)
        .map_err(|e| anyhow::anyhow!("Invalid CSS selector: {:?}", e))?;

    let matches: Vec<String> = document
        .select(&selector)
        .map(|el| extract_text_from_element(&el))
        .collect();

    if matches.is_empty() {
        Ok(format!("No elements found matching selector: {}", selector_str))
    } else {
        Ok(matches.join("\n\n---\n\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html_to_text() {
        let html = r#"
            <html>
            <body>
                <h1>Title</h1>
                <p>First paragraph.</p>
                <p>Second paragraph.</p>
            </body>
            </html>
        "#;

        let text = html_to_text(html);
        assert!(text.contains("Title"));
        assert!(text.contains("First paragraph"));
        assert!(text.contains("Second paragraph"));
    }

    #[test]
    fn test_html_to_text_strips_scripts() {
        let html = r#"
            <html>
            <body>
                <p>Content</p>
                <script>alert('evil');</script>
            </body>
            </html>
        "#;

        let text = html_to_text(html);
        assert!(text.contains("Content"));
        assert!(!text.contains("alert"));
    }

    #[test]
    fn test_extract_with_selector() {
        let html = r#"
            <html>
            <body>
                <div class="item">Item 1</div>
                <div class="item">Item 2</div>
                <div class="other">Other</div>
            </body>
            </html>
        "#;

        let result = extract_with_selector(html, ".item").unwrap();
        assert!(result.contains("Item 1"));
        assert!(result.contains("Item 2"));
        assert!(!result.contains("Other"));
    }
}
