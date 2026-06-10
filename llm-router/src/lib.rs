//! llm-router: routing, provider registry, credentials, model catalog, and the
//! failure contract — spec: tech-specs/2026-06-agentic/llm-router.md.

pub mod types;

#[cfg(test)]
mod smoke {
    #[test]
    fn scaffold() {
        assert_eq!(2 + 2, 4);
    }
}
