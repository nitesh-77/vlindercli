//! HTML preprocessing functions (ADR 004)
//!
//! Cleans HTML before readability extraction by finding content
//! containers and removing boilerplate elements.

use scraper::{Html, Selector};

use crate::config::{BOILERPLATE_SELECTORS, CONTENT_SELECTORS};

/// Pre-process HTML to improve readability extraction
///
/// Strategy 1: Find main content container (article, main, etc.)
/// Strategy 2: Remove boilerplate elements from the DOM
pub fn preprocess_html(raw_html: &str) -> String {
    let mut document = Html::parse_document(raw_html);

    // Strategy 1: Try to find main content container
    for selector_str in CONTENT_SELECTORS {
        if let Ok(selector) = Selector::parse(selector_str) {
            if let Some(element) = document.select(&selector).next() {
                return element.html();
            }
        }
    }

    // Strategy 2: Remove boilerplate elements from DOM
    let mut nodes_to_remove = Vec::new();
    for selector_str in BOILERPLATE_SELECTORS {
        if let Ok(selector) = Selector::parse(selector_str) {
            for element in document.select(&selector) {
                nodes_to_remove.push(element.id());
            }
        }
    }

    for node_id in nodes_to_remove {
        if let Some(mut node) = document.tree.get_mut(node_id) {
            node.detach();
        }
    }

    document.html()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_article_content() {
        let html = r#"
            <html>
                <body>
                    <nav>Navigation here</nav>
                    <article>
                        <h1>Article Title</h1>
                        <p>Article content goes here.</p>
                    </article>
                    <footer>Footer content</footer>
                </body>
            </html>
        "#;
        let result = preprocess_html(html);
        assert!(result.contains("Article Title"));
        assert!(result.contains("Article content"));
        assert!(!result.contains("Navigation here"));
        assert!(!result.contains("Footer content"));
    }

    #[test]
    fn extracts_main_content() {
        let html = r#"
            <html>
                <body>
                    <header>Site Header</header>
                    <main>
                        <h1>Main Content</h1>
                        <p>This is the main content area.</p>
                    </main>
                    <aside>Sidebar</aside>
                </body>
            </html>
        "#;
        let result = preprocess_html(html);
        assert!(result.contains("Main Content"));
        assert!(!result.contains("Site Header"));
        assert!(!result.contains("Sidebar"));
    }

    #[test]
    fn removes_boilerplate_when_no_content_container() {
        let html = r#"
            <html>
                <body>
                    <nav>Navigation</nav>
                    <div class="content-wrapper">
                        <h1>Page Title</h1>
                        <p>Actual content here.</p>
                    </div>
                    <footer>Copyright info</footer>
                </body>
            </html>
        "#;
        let result = preprocess_html(html);
        assert!(result.contains("Page Title"));
        assert!(result.contains("Actual content"));
        assert!(!result.contains("Navigation"));
        assert!(!result.contains("Copyright info"));
    }

    #[test]
    fn handles_role_attributes() {
        let html = r#"
            <html>
                <body>
                    <div role="navigation">Nav links</div>
                    <div role="main">
                        <p>Main content here.</p>
                    </div>
                    <div role="contentinfo">Footer info</div>
                </body>
            </html>
        "#;
        let result = preprocess_html(html);
        assert!(result.contains("Main content"));
        assert!(!result.contains("Nav links"));
        assert!(!result.contains("Footer info"));
    }
}
