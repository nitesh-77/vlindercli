use extism_pdk::*;

#[host_fn]
extern "ExtismHost" {
    fn infer(model: String, prompt: String) -> String;
    fn put_file(path: String, content: Vec<u8>) -> String;
    fn get_file(path: String) -> Vec<u8>;
}

#[plugin_fn]
pub fn process(url: String) -> FnResult<String> {
    // Generate cache key from URL (simple hash)
    let cache_path = format!("/cache/{}.html", url_to_filename(&url));

    // Step 1: Check cache first
    let html = unsafe {
        let cached = get_file(cache_path.clone())?;
        let cached_str = String::from_utf8_lossy(&cached).to_string();
        if !cached_str.starts_with("[error]") {
            cached_str
        } else {
            // Not cached - fetch from network
            let req = HttpRequest::new(&url);
            let res = http::request::<()>(&req, None)?;
            let fetched = String::from_utf8(res.body().to_vec())?;

            // Cache for future use
            let _ = put_file(cache_path, fetched.as_bytes().to_vec())?;
            fetched
        }
    };

    // Step 2: infer to extract clean content
    let clean = unsafe {
        infer("phi3".to_string(), format!("Extract main article text, remove navigation/ads: {}", html))?
    };

    // wasm: trim whitespace
    let trimmed = clean.trim().to_string();

    // Step 3: infer to generate takeaways
    let takeaways = unsafe {
        infer("phi3".to_string(), format!("3 key takeaways from: {}", trimmed))?
    };

    // wasm: format output
    Ok(format!("{}\n\n---\nKey takeaways:\n{}", trimmed, takeaways))
}

/// Convert URL to safe filename
fn url_to_filename(url: &str) -> String {
    url.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}
