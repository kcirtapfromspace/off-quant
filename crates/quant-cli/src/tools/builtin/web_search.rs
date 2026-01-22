//! Web search tool using DuckDuckGo

use anyhow::Result;
use async_trait::async_trait;
use scraper::{Html, Selector};
use serde_json::Value;
use std::sync::OnceLock;
use std::time::Duration;
use tracing::{debug, instrument, warn};

use crate::tools::{ParameterProperty, ParameterSchema, SecurityLevel, Tool, ToolContext, ToolResult};

/// Shared HTTP client for connection pooling
static SHARED_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn get_shared_client() -> &'static reqwest::Client {
    SHARED_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(Duration::from_secs(90))
            .timeout(Duration::from_secs(30))
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            .build()
            .expect("Failed to create HTTP client")
    })
}

/// Tool for searching the web
pub struct WebSearchTool;

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web using DuckDuckGo. Returns search results with titles, URLs, and snippets."
    }

    fn security_level(&self) -> SecurityLevel {
        SecurityLevel::Moderate
    }

    fn parameters_schema(&self) -> ParameterSchema {
        ParameterSchema::new()
            .with_required("query", ParameterProperty::string("The search query"))
            .with_property("limit", ParameterProperty::number("Maximum number of results (default: 10)").with_default(Value::Number(10.into())))
    }

    #[instrument(skip(self, args, ctx), fields(query = tracing::field::Empty))]
    async fn execute(&self, args: &Value, ctx: &ToolContext) -> Result<ToolResult> {
        let query = args.get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: query"))?;

        // Record query in span (truncate for safety)
        tracing::Span::current().record("query", &query.chars().take(50).collect::<String>().as_str());

        let limit = args.get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(10);

        debug!(limit, timeout_secs = ctx.http_timeout_secs, "Web search parameters");

        // Use shared client for connection pooling
        let client = get_shared_client();

        let search_url = format!(
            "https://html.duckduckgo.com/html/?q={}",
            urlencoding::encode(query)
        );

        debug!("Sending search request to DuckDuckGo");
        let response = match client
            .get(&search_url)
            .timeout(Duration::from_secs(ctx.http_timeout_secs))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "Search request failed");
                return Ok(ToolResult::error(format!("Search request failed: {}", e)));
            }
        };

        if !response.status().is_success() {
            warn!(status = %response.status(), "Search returned error status");
            return Ok(ToolResult::error(format!("Search failed with status: {}", response.status())));
        }

        let html = match response.text().await {
            Ok(h) => h,
            Err(e) => return Ok(ToolResult::error(format!("Failed to read response: {}", e))),
        };

        // Parse search results
        let results = parse_duckduckgo_results(&html, limit);

        if results.is_empty() {
            return Ok(ToolResult::success(format!("No results found for: {}", query)));
        }

        let mut output = format!("Search results for '{}':\n\n", query);

        for (i, result) in results.iter().enumerate() {
            output.push_str(&format!(
                "{}. {}\n   {}\n   {}\n\n",
                i + 1,
                result.title,
                result.url,
                result.snippet
            ));
        }

        Ok(ToolResult::success(output))
    }
}

#[derive(Debug)]
struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}

fn parse_duckduckgo_results(html: &str, limit: usize) -> Vec<SearchResult> {
    let document = Html::parse_document(html);
    let mut results = Vec::new();

    // DuckDuckGo HTML results are in elements with class "result"
    let result_selector = Selector::parse(".result").unwrap();
    let title_selector = Selector::parse(".result__a").unwrap();
    let snippet_selector = Selector::parse(".result__snippet").unwrap();

    for result_el in document.select(&result_selector).take(limit) {
        let title = result_el
            .select(&title_selector)
            .next()
            .map(|el| el.text().collect::<Vec<_>>().join(" "))
            .unwrap_or_default()
            .trim()
            .to_string();

        let url = result_el
            .select(&title_selector)
            .next()
            .and_then(|el| el.value().attr("href"))
            .map(|href| {
                // DuckDuckGo wraps URLs, extract the actual URL
                if href.contains("uddg=") {
                    href.split("uddg=")
                        .nth(1)
                        .and_then(|s| urlencoding::decode(s.split('&').next().unwrap_or(s)).ok())
                        .map(|s| s.into_owned())
                        .unwrap_or_else(|| href.to_string())
                } else {
                    href.to_string()
                }
            })
            .unwrap_or_default();

        let snippet = result_el
            .select(&snippet_selector)
            .next()
            .map(|el| el.text().collect::<Vec<_>>().join(" "))
            .unwrap_or_default()
            .trim()
            .to_string();

        if !title.is_empty() && !url.is_empty() {
            results.push(SearchResult { title, url, snippet });
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty_results() {
        let html = "<html><body></body></html>";
        let results = parse_duckduckgo_results(html, 10);
        assert!(results.is_empty());
    }

    // Note: Integration tests for actual search would require network access
    // and could be flaky, so we only test the parsing logic
}
