use extism_pdk::*;

#[host_fn]
extern "ExtismHost" {
    fn infer(prompt: String) -> String;
}

#[plugin_fn]
pub fn process(url: String) -> FnResult<String> {
    // Step 1: agent fetches directly
    let req = HttpRequest::new(&url);
    let res = http::request::<()>(&req, None)?;
    let html = String::from_utf8(res.body().to_vec())?;

    // Step 2: infer to extract clean content
    let clean = unsafe {
        infer(format!("Extract main article text, remove navigation/ads: {}", html))?
    };

    // wasm: trim whitespace
    let trimmed = clean.trim().to_string();

    // Step 3: infer to generate takeaways
    let takeaways = unsafe {
        infer(format!("3 key takeaways from: {}", trimmed))?
    };

    // wasm: format output
    Ok(format!("{}\n\n---\nKey takeaways:\n{}", trimmed, takeaways))
}
