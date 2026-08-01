use serde_json::Value;
use std::collections::BTreeSet;

#[test]
fn versioned_program_routing_corpus_is_complete_and_content_safe() {
    let raw = include_str!("fixtures/program-routing-v1.json");
    let corpus: Value = serde_json::from_str(raw).expect("valid corpus JSON");
    assert_eq!(corpus["version"], 1);
    assert!(corpus["provenance"]
        .as_str()
        .unwrap()
        .contains("maintainer-authored"));
    let tasks = corpus["tasks"].as_array().unwrap();
    assert!(tasks.len() >= 50);
    let mut ids = BTreeSet::new();
    let mut routes = BTreeSet::new();
    for task in tasks {
        for field in [
            "id",
            "prompt",
            "expected_route",
            "fixture",
            "expected_result",
            "failure_case",
        ] {
            assert!(
                !task[field].as_str().unwrap_or("").is_empty(),
                "missing {field}"
            );
        }
        assert!(task["allowed_effects"].is_array());
        assert!(ids.insert(task["id"].as_str().unwrap()));
        routes.insert(task["expected_route"].as_str().unwrap());
    }
    assert_eq!(
        routes,
        BTreeSet::from(["answer", "clarification", "program", "shell"])
    );
    for forbidden in [
        "telemetry_event",
        "history.jsonl",
        "OPENAI_API_KEY",
        "/home/",
    ] {
        assert!(!raw.contains(forbidden));
    }
}
