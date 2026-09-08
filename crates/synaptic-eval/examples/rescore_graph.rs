//! Re-score saved before/after graphs with the same current evaluator.
//! Usage: cargo run -p synaptic-eval --example rescore_graph -- REPO GRAPH...
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(args.next().ok_or("expected REPO GRAPH...")?);
    let mut results = Vec::new();
    for path in args {
        let graph: synaptic_core::GraphData = serde_json::from_slice(&std::fs::read(&path)?)?;
        results.push(serde_json::json!({
            "graph": path,
            "quality": synaptic_eval::quality::score_graph(&root, &graph),
            "oracle": synaptic_eval::oracle::compare(&root, &graph),
        }));
    }
    if results.is_empty() {
        return Err("expected at least one graph".into());
    }
    println!("{}", serde_json::to_string_pretty(&results)?);
    Ok(())
}
