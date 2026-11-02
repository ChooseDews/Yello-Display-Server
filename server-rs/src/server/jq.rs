//! jq over arbitrary JSON, via the Rust `jaq` crates — backs
//! `POST /api/jq`, which the Studio agent's `jq_design` tool uses to read
//! and mechanically edit design documents without re-emitting them whole.
//! Pure function: program + JSON in, the program's outputs out.

use serde_json::Value;

/// Cap on outputs collected from one program run, so a runaway
/// `range(...)` cannot flood the caller.
pub const MAX_OUTPUTS: usize = 512;

/// Cap on program length; real design queries are far shorter.
pub const MAX_PROGRAM_CHARS: usize = 8000;

/// Run `program` over `input` and collect its outputs (at most
/// [`MAX_OUTPUTS`]). Errors are human-readable strings for the caller.
/// Follows the embedding recipe in jaq-core 3's crate docs.
pub fn run_jq(program: &str, input: &Value) -> Result<Vec<Value>, String> {
    use jaq_core::load::{Arena, File, Loader};
    use jaq_core::{data, unwrap_valr, Compiler, Ctx, Vars};

    if program.chars().count() > MAX_PROGRAM_CHARS {
        return Err(format!("jq program is too long (max {MAX_PROGRAM_CHARS} characters)"));
    }
    let input_val = jaq_json::read::parse_single(input.to_string().as_bytes())
        .map_err(|e| format!("invalid input JSON: {e:?}"))?;
    let defs = jaq_core::defs().chain(jaq_std::defs()).chain(jaq_json::defs());
    let funs = jaq_core::funs().chain(jaq_std::funs()).chain(jaq_json::funs());
    let loader = Loader::new(defs);
    let arena = Arena::default();
    let modules = loader
        .load(&arena, File { code: program, path: () })
        .map_err(|e| format!("jq parse error: {e:?}"))?;
    let filter = Compiler::default()
        .with_funs(funs)
        .compile(modules)
        .map_err(|e| format!("jq compile error: {e:?}"))?;
    let ctx = Ctx::<data::JustLut<jaq_json::Val>>::new(&filter.lut, Vars::new([]));
    let mut outputs = Vec::new();
    for item in filter.id.run((ctx, input_val)).map(unwrap_valr) {
        let value = item.map_err(|e| format!("jq error: {e}"))?;
        outputs.push(
            serde_json::from_str(&value.to_string())
                .map_err(|e| format!("jq output is not JSON: {e}"))?,
        );
        if outputs.len() >= MAX_OUTPUTS {
            break;
        }
    }
    Ok(outputs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_slices() {
        let doc = json!({"pages": [{"id": "main", "name": "Main"}, {"id": "p2", "name": "Two"}], "name": "D"});
        let out = run_jq(".pages | map({id, name})", &doc).unwrap();
        assert_eq!(out, vec![json!([{"id": "main", "name": "Main"}, {"id": "p2", "name": "Two"}])]);
        let names = run_jq(".pages[].name", &doc).unwrap();
        assert_eq!(names, vec![json!("Main"), json!("Two")]);
    }

    #[test]
    fn edits_whole_document() {
        let doc = json!({"pages": [{"id": "main", "name": "Main"}], "background": "#000"});
        let out = run_jq(".pages[0].name = \"Renamed\" | .background = \"#111\"", &doc).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["pages"][0]["name"], json!("Renamed"));
        assert_eq!(out[0]["background"], json!("#111"));
        // The input is not mutated.
        assert_eq!(doc["pages"][0]["name"], json!("Main"));
    }

    #[test]
    fn update_operator_and_select() {
        let doc = json!({"elements": [{"type": "text", "size": 10}, {"type": "gauge", "size": 10}]});
        let out = run_jq(
            ".elements |= map(if .type == \"text\" then .size = 20 else . end) | [.elements[] | select(.size == 20)] | length",
            &doc,
        )
        .unwrap();
        assert_eq!(out, vec![json!(1)]);
    }

    #[test]
    fn errors_are_messages() {
        let doc = json!({"a": 1});
        let parse = run_jq(".[", &doc).unwrap_err();
        assert!(parse.contains("error"), "parse error text: {parse}");
        let runtime = run_jq("error(\"boom\")", &doc).unwrap_err();
        assert!(runtime.contains("boom"), "runtime error text: {runtime}");
    }

    #[test]
    fn outputs_are_capped() {
        let out = run_jq("range(0; 100000)", &json!(null)).unwrap();
        assert_eq!(out.len(), MAX_OUTPUTS);
    }
}
