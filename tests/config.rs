mod common;
use common::validate;

const EXAMPLE: &str = include_str!("../examples/tasks/synthetic-report.toml");

#[test]
fn accepts_typed_inputs_and_synthetic_paths() {
    assert_eq!(validate(EXAMPLE).unwrap().task_count(), 1);
    assert!(common::validate_set(&[]).is_ok());
    assert!(
        validate(
            &EXAMPLE
                .replace("default = 2", "default = -9223372036854775808")
                .replace(", choices = [1, 2, 3]", "")
        )
        .is_ok()
    );
    assert!(validate(&EXAMPLE.replace("--name", "$(touch /synthetic/marker); --flag")).is_ok());
}

#[test]
fn rejects_unknown_fields_at_each_level() {
    for source in [
        format!("unknown = true\n{EXAMPLE}"),
        EXAMPLE.replace("label =", "unknown = true\nlabel ="),
        EXAMPLE.replace("type = \"string\"", "unknown = true, type = \"string\""),
        EXAMPLE.replace(
            "literal = \"--name\"",
            "literal = \"--name\", unknown = true",
        ),
        EXAMPLE.replace("input = \"name\"", "input = \"name\", unknown = true"),
        EXAMPLE.replace(
            "input = \"name\"",
            "input = \"name\", literal = \"ambiguous\"",
        ),
    ] {
        assert!(validate(&source).is_err(), "{source}");
    }
}

#[test]
fn rejects_duplicate_ids_and_keys() {
    for source in [
        EXAMPLE.replace("id = \"count\"", "id = \"name\""),
        EXAMPLE.replace("label =", "label = 'duplicate'\nlabel ="),
        EXAMPLE.replace(
            "type = \"integer\"",
            "type = \"integer\", type = \"string\"",
        ),
        EXAMPLE.replace(
            "literal = \"--name\"",
            "literal = \"--name\", literal = \"other\"",
        ),
        EXAMPLE.replace("input = \"name\"", "input = \"name\", input = \"count\""),
    ] {
        assert!(validate(&source).is_err(), "{source}");
    }
}

#[test]
fn rejects_invalid_defaults_and_choices() {
    for value in [
        "'2'",
        "2.5",
        "true",
        "[]",
        "{}",
        "4",
        "1979-05-27",
        "\"bad\\u0000value\"",
    ] {
        assert!(
            validate(&EXAMPLE.replace("default = 2", &format!("default = {value}")))
                .unwrap_err()
                .contains("invalid default"),
            "{value}"
        );
    }
    for choices in ["[]", "[1, 1]", "[1, '2']", "[true]", "[{}]", "[1979-05-27]"] {
        assert!(
            validate(&EXAMPLE.replace("[1, 2, 3]", choices)).is_err(),
            "{choices}"
        );
    }
    assert!(validate(&EXAMPLE.replace("default = false", "default = 'false'")).is_err());
    assert!(
        validate(&EXAMPLE.replace("required = true", "required = true, default = 'sample'"))
            .is_ok()
    );
}

#[test]
fn rejects_invalid_references() {
    assert!(
        validate(&EXAMPLE.replace("input = \"name\"", "input = \"missing\""))
            .unwrap_err()
            .contains("undeclared input")
    );
    assert!(
        validate(&EXAMPLE.replace("required = true", "required = false"))
            .unwrap_err()
            .contains("required or have a default")
    );
    assert!(validate(&EXAMPLE.replace("input = \"name\"", "input = ''")).is_err());
}

#[test]
fn rejects_invalid_structure_and_command_fields() {
    for source in [
        EXAMPLE.replace("label = \"Synthetic report\"", "label = ''"),
        EXAMPLE.replace("/synthetic/runtime/bin/report", "report"),
        EXAMPLE.replace(
            "working_directory = \"/synthetic/runtime\"",
            "working_directory = 'relative'",
        ),
        EXAMPLE.replace("literal = \"--name\"", "literal = \"bad\\u0000value\""),
        EXAMPLE.replace("type = \"string\"", "type = \"float\""),
        EXAMPLE.replace("literal = \"--name\"", "literal = 123"),
        EXAMPLE.replace("label = \"Synthetic report\"", "label = 1979-05-27"),
        EXAMPLE.replace("default = 2", "default = 9223372036854775808"),
        EXAMPLE.replace("default = 2", "default = null"),
        "version: 1\ntasks: []".into(),
    ] {
        assert!(validate(&source).is_err(), "{source}");
    }
}

#[test]
fn preserves_quoted_strings_and_rejects_bare_words() {
    for text in ["NO", "yes", "on", "null", "true", "2026-11-08"] {
        let source = EXAMPLE.replace(
            "required = true",
            &format!("required = true, default = '{text}'"),
        );
        let config = validate(&source).unwrap();
        assert_eq!(
            config
                .arguments("synthetic-report", &Default::default())
                .unwrap()[1],
            text
        );
    }
    for text in ["NO", "yes", "on", "null"] {
        assert!(
            validate(&EXAMPLE.replace(
                "required = true",
                &format!("required = true, default = {text}")
            ))
            .is_err()
        );
    }
}
#[test]
fn accepts_arrays_of_tables_with_the_same_domain_rules() {
    let source = r#"
label = "Country report"
executable = "/synthetic/report"
working_directory = "/synthetic"
[[arguments]]
input = "country"
[[inputs]]
id = "country"
type = "string"
default = "NO"
choices = ["NO", "on"]
"#;
    let config = validate(source).unwrap();
    assert_eq!(
        config
            .arguments("synthetic-report", &Default::default())
            .unwrap(),
        ["NO"]
    );
    assert!(validate(&source.replace("default = \"NO\"", "default = false")).is_err());
}
