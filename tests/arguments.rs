use std::collections::BTreeMap;

use rustward::config::validate;
use toml::Value;

const EXAMPLE: &str = include_str!("../examples/tasks.toml");

fn values(entries: &[(&str, Value)]) -> BTreeMap<String, Value> {
    entries
        .iter()
        .map(|(id, value)| (id.to_string(), value.clone()))
        .collect()
}

#[test]
fn preserves_argument_boundaries_and_repeated_references() {
    let source = EXAMPLE.replace(
        "    { literal = \"--name\" },",
        "    { literal = '' },\n    { literal = 'literal with spaces; $HOME' },\n    { input = 'name' },",
    );
    let config = validate(&source).unwrap();
    for text in [
        "",
        "two words",
        "  padded  ",
        "日本語 🦀",
        "$(touch /synthetic/marker); * $HOME ' \" \\ \n",
    ] {
        let result = config
            .arguments("synthetic-report", &values(&[("name", text.into())]))
            .unwrap();
        assert_eq!(
            result,
            ["", "literal with spaces; $HOME", text, text, "2", "false"]
        );
    }
}

#[test]
fn applies_defaults_and_explicit_overrides() {
    let config =
        validate(&EXAMPLE.replace("required = true", "required = true, default = 'sample'"))
            .unwrap();
    assert_eq!(
        config
            .arguments("synthetic-report", &BTreeMap::new())
            .unwrap(),
        ["--name", "sample", "2", "false"]
    );
    assert_eq!(
        config
            .arguments(
                "synthetic-report",
                &values(&[
                    ("name", "".into()),
                    ("count", 3.into()),
                    ("enabled", true.into())
                ])
            )
            .unwrap(),
        ["--name", "", "3", "true"]
    );
}

#[test]
fn formats_signed_integer_limits() {
    let config = validate(&EXAMPLE.replace(", choices = [1, 2, 3]", "")).unwrap();
    for number in [i64::MIN, -1, 0, i64::MAX] {
        let args = config
            .arguments(
                "synthetic-report",
                &values(&[("name", "sample".into()), ("count", number.into())]),
            )
            .unwrap();
        assert_eq!(args[2], number.to_string());
    }
}

#[test]
fn rejects_invalid_values_without_default_fallback() {
    let config = validate(EXAMPLE).unwrap();
    for (id, invalid) in [
        ("name", Value::Datetime("1979-05-27".parse().unwrap())),
        ("name", 2.into()),
        ("name", true.into()),
        ("name", "bad\0value".into()),
        ("count", "2".into()),
        ("count", true.into()),
        ("count", 2.5.into()),
        ("count", Value::Float(f64::INFINITY)),
        ("count", 4.into()),
        ("enabled", "false".into()),
        ("enabled", 0.into()),
        ("enabled", Value::Datetime("1979-05-27".parse().unwrap())),
        ("name", Value::Array(vec![])),
        ("name", Value::Table(Default::default())),
    ] {
        let mut submitted = values(&[("name", "sample".into())]);
        submitted.insert(id.into(), invalid);
        assert!(
            config
                .arguments("synthetic-report", &submitted)
                .unwrap_err()
                .contains(&format!("input {id}:"))
        );
    }
}

#[test]
fn rejects_missing_and_unknown_inputs_and_unknown_tasks() {
    let config = validate(EXAMPLE).unwrap();
    assert!(
        config
            .arguments("synthetic-report", &BTreeMap::new())
            .unwrap_err()
            .contains("missing required input")
    );
    assert!(
        config
            .arguments("synthetic-report", &values(&[("other", true.into())]))
            .unwrap_err()
            .contains("undeclared input")
    );
    for id in ["", "missing", "Synthetic-report"] {
        assert!(
            config
                .arguments(id, &BTreeMap::new())
                .unwrap_err()
                .contains("unknown task ID")
        );
    }
}

#[test]
fn selects_task_and_validates_unused_inputs() {
    let source = format!(
        r#"{EXAMPLE}
[[tasks]]
id = "other"
label = "Other"
executable = "/synthetic/other"
working_directory = "/synthetic"
arguments = []
inputs = [
    {{ id = "unused", type = "boolean", required = true, choices = [true] }},
    {{ id = "optional", type = "string" }},
]
"#
    );
    let config = validate(&source).unwrap();
    assert!(config.arguments("other", &BTreeMap::new()).is_err());
    assert!(
        config
            .arguments("other", &values(&[("unused", false.into())]))
            .is_err()
    );
    assert!(
        config
            .arguments(
                "other",
                &values(&[("unused", true.into()), ("optional", 1.into())])
            )
            .is_err()
    );
    assert!(
        config
            .arguments("other", &values(&[("unused", true.into())]))
            .unwrap()
            .is_empty()
    );
    assert!(
        config
            .arguments("other", &values(&[("name", "sample".into())]))
            .is_err()
    );
}

#[test]
fn checks_string_choices_without_conversion() {
    let config = validate(&EXAMPLE.replace(
        "required = true",
        "required = true, choices = ['', '日本語', 'true']",
    ))
    .unwrap();
    for text in ["", "日本語", "true"] {
        assert_eq!(
            config
                .arguments("synthetic-report", &values(&[("name", text.into())]))
                .unwrap()[1],
            text
        );
    }
    for value in ["other".into(), true.into()] {
        assert!(
            config
                .arguments("synthetic-report", &values(&[("name", value)]))
                .is_err()
        );
    }
}

#[test]
fn keeps_submitted_values_separate_from_defaults_and_arguments() {
    let source = EXAMPLE.replace("required = true", "required = true, default = 'sample'");
    let config = validate(&source).unwrap();
    let submitted = values(&[("count", 3.into())]);
    let original = submitted.clone();
    assert_eq!(
        config.arguments("synthetic-report", &submitted).unwrap(),
        ["--name", "sample", "3", "false"]
    );
    assert_eq!(submitted, original);
    assert_eq!(
        config
            .arguments("synthetic-report", &BTreeMap::new())
            .unwrap(),
        ["--name", "sample", "2", "false"]
    );
    let error = config
        .arguments(
            "synthetic-report",
            &values(&[("name", Value::Datetime("1979-05-27".parse().unwrap()))]),
        )
        .unwrap_err();
    assert!(error.contains("input name:"));
}

#[test]
fn validates_unreferenced_values_without_generating_arguments() {
    let source = r#"version = 1
[[tasks]]
id = "sample"
label = "Sample"
executable = "/synthetic/program"
working_directory = "/synthetic"
arguments = []
inputs = [{ id = "selection", type = "string", default = "allowed", choices = ["allowed"] }]
"#;
    let config = validate(source).unwrap();
    assert!(
        config
            .arguments("sample", &BTreeMap::new())
            .unwrap()
            .is_empty()
    );
    for invalid in [
        Value::Datetime("1979-05-27".parse().unwrap()),
        "synthetic-private-value".into(),
        1.into(),
    ] {
        let submitted = values(&[("selection", invalid)]);
        let original = submitted.clone();
        let error = config.arguments("sample", &submitted).unwrap_err();
        assert!(error.contains("task sample: input selection:"));
        assert!(!error.contains("synthetic-private-value"));
        assert_eq!(submitted, original);
    }
}
