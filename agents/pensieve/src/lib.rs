//! Pensieve Agent - Extract and recall web content
//!
//! Like Dumbledore's pensieve: store memories (articles) for later review.

use extism_pdk::*;
use readability::extractor;

/// Fetch and process a URL
#[plugin_fn]
pub fn process(url: String) -> FnResult<String> {
    // Fetch the page
    let req = HttpRequest::new(&url);
    let res = http::request::<()>(&req, None)?;
    let html = String::from_utf8(res.body().to_vec())?;

    // Extract article content using Mozilla Readability algorithm
    let url_parsed = url::Url::parse(&url)
        .unwrap_or_else(|_| url::Url::parse("http://example.com").unwrap());

    let content = match extractor::extract(&mut html.as_bytes(), &url_parsed) {
        Ok(product) => {
            format!("{}\n\n{}", product.title, product.text)
        }
        Err(e) => {
            return Ok(format!("Extraction failed: {}", e));
        }
    };

    Ok(format!(
        "Source: {}\nLength: {} chars\n\n---\n{}",
        url,
        content.len(),
        content
    ))
}
