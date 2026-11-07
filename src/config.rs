use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Deserializer};
use serde_yaml_ng::Value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    version: u32,
    tasks: Vec<Task>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Task {
    #[serde(deserialize_with = "read_string")]
    id: String,
    #[serde(deserialize_with = "read_string")]
    label: String,
    #[serde(deserialize_with = "read_string")]
    executable: String,
    #[serde(deserialize_with = "read_string")]
    working_directory: String,
    arguments: Vec<Argument>,
    inputs: Vec<Input>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Argument {
    Literal(Literal),
    Input(InputReference),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Literal {
    #[serde(deserialize_with = "read_string")]
    literal: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputReference {
    #[serde(deserialize_with = "read_string")]
    input: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    #[serde(deserialize_with = "read_string")]
    id: String,
    #[serde(rename = "type")]
    kind: InputType,
    #[serde(default)]
    required: bool,
    #[serde(default, deserialize_with = "read_default")]
    default: Option<Value>,
    #[serde(default, deserialize_with = "read_choices")]
    choices: Option<Vec<Value>>,
}

fn read_string<'de, D: Deserializer<'de>>(reader: D) -> Result<String, D::Error> {
    match Value::deserialize(reader)? {
        Value::String(text) => Ok(text),
        _ => Err(serde::de::Error::custom("expected a string")),
    }
}

fn read_choices<'de, D: Deserializer<'de>>(reader: D) -> Result<Option<Vec<Value>>, D::Error> {
    Vec::<Value>::deserialize(reader).map(Some)
}

fn read_default<'de, D: Deserializer<'de>>(reader: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(reader).map(Some)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum InputType {
    String,
    Integer,
    Boolean,
}

impl Input {
    fn accepts(&self, value: &Value) -> bool {
        match self.kind {
            InputType::String => value.as_str().is_some_and(|text| !text.contains('\0')),
            InputType::Integer => value.as_i64().is_some(),
            InputType::Boolean => value.as_bool().is_some(),
        }
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && id.as_bytes()[0].is_ascii_lowercase()
}

fn valid_path(path: &str) -> bool {
    Path::new(path).is_absolute() && !path.contains('\0')
}

pub fn validate(source: &str) -> Result<Config, String> {
    let config: Config = serde_yaml_ng::from_str(source).map_err(|error| error.to_string())?;
    if config.version != 1 {
        return Err("version: only version 1 is supported".into());
    }
    let mut task_ids = HashSet::new();
    for task in &config.tasks {
        let fail = |message: &str| format!("task {}: {message}", task.id);
        if !valid_id(&task.id) {
            return Err(fail("invalid task ID"));
        }
        if !task_ids.insert(&task.id) {
            return Err(fail("duplicate task ID"));
        }
        if task.label.trim().is_empty() || task.label.contains('\0') {
            return Err(fail("label must contain text without NUL"));
        }
        if !valid_path(&task.executable) || !valid_path(&task.working_directory) {
            return Err(fail(
                "executable and working_directory must be absolute paths without NUL",
            ));
        }
        let mut input_ids = HashSet::new();
        for input in &task.inputs {
            let input_fail = |message: &str| fail(&format!("input {}: {message}", input.id));
            if !valid_id(&input.id) || !input_ids.insert(&input.id) {
                return Err(input_fail("invalid or duplicate input ID"));
            }
            if let Some(choices) = &input.choices {
                if choices.is_empty() || choices.iter().any(|value| !input.accepts(value)) {
                    return Err(input_fail(
                        "choices must be nonempty and match the input type",
                    ));
                }
                for (index, value) in choices.iter().enumerate() {
                    if choices[..index].contains(value) {
                        return Err(input_fail("duplicate choice"));
                    }
                }
            }
            if let Some(default) = &input.default
                && (!input.accepts(default)
                    || input
                        .choices
                        .as_ref()
                        .is_some_and(|choices| !choices.contains(default)))
            {
                return Err(input_fail("invalid default"));
            }
        }
        for argument in &task.arguments {
            match argument {
                Argument::Literal(value) if value.literal.contains('\0') => {
                    return Err(fail("literal argument contains NUL"));
                }
                Argument::Input(reference) => {
                    let Some(input) = task.inputs.iter().find(|input| input.id == reference.input)
                    else {
                        return Err(fail("argument references an undeclared input"));
                    };
                    if !input.required && input.default.is_none() {
                        return Err(fail("referenced input must be required or have a default"));
                    }
                }
                _ => {}
            }
        }
    }
    Ok(config)
}

impl Config {
    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }
}

#[cfg(test)]
mod tests {
    use super::validate;

    const EXAMPLE: &str = include_str!("../examples/tasks.yaml");

    #[test]
    fn accepts_typed_inputs_and_synthetic_paths() {
        assert_eq!(validate(EXAMPLE).unwrap().task_count(), 1);
        assert!(validate("version: 1\ntasks: []").is_ok());
        assert!(
            validate(
                &EXAMPLE
                    .replace("default: 2", "default: -9223372036854775808")
                    .replace("choices: [1, 2, 3]", "")
            )
            .is_ok()
        );
        assert!(
            validate(&EXAMPLE.replace(
                "literal: --name",
                "literal: '$(touch /synthetic/marker); --flag' "
            ))
            .is_ok()
        );
    }

    #[test]
    fn rejects_unknown_fields_at_each_level() {
        for source in [
            format!("{EXAMPLE}\nunknown: true"),
            EXAMPLE.replace("    label:", "    unknown: true\n    label:"),
            EXAMPLE.replace(
                "        type: string",
                "        type: string\n        unknown: true",
            ),
            EXAMPLE.replace("literal: --name", "literal: --name\n        unknown: true"),
            EXAMPLE.replace("input: name", "input: name\n        unknown: true"),
            EXAMPLE.replace("input: name", "input: name\n        literal: ambiguous"),
        ] {
            assert!(validate(&source).is_err(), "{source}");
        }
    }

    #[test]
    fn rejects_duplicate_ids_and_keys() {
        let task = EXAMPLE.split_once("tasks:\n").unwrap().1;
        assert!(
            validate(&format!("{EXAMPLE}{task}"))
                .unwrap_err()
                .contains("duplicate task ID")
        );
        assert!(validate(&EXAMPLE.replace("id: count", "id: name")).is_err());
        assert!(validate(&EXAMPLE.replace("version: 1", "version: 1\nversion: 1")).is_err());
        assert!(
            validate(&EXAMPLE.replace("type: integer", "type: integer\n        type: string"))
                .is_err()
        );
    }

    #[test]
    fn rejects_invalid_defaults() {
        for value in [
            "null",
            "'2'",
            "2.5",
            "true",
            "[]",
            "{}",
            "4",
            "9223372036854775808",
        ] {
            let source = EXAMPLE.replace("default: 2", &format!("default: {value}"));
            assert!(
                validate(&source).unwrap_err().contains("invalid default"),
                "{value}"
            );
        }
        assert!(validate(&EXAMPLE.replace("default: false", "default: 'false'")).is_err());
        assert!(validate(&EXAMPLE.replace("required: true", "default: 12")).is_err());
        assert!(validate(&EXAMPLE.replace("required: true", "default: 'sample'")).is_ok());
    }

    #[test]
    fn rejects_invalid_choices() {
        for choices in [
            "[]", "[1, 1]", "[1, '2']", "[null]", "[true]", "[{}]", "null",
        ] {
            assert!(
                validate(&EXAMPLE.replace("[1, 2, 3]", choices)).is_err(),
                "{choices}"
            );
        }
    }

    #[test]
    fn rejects_invalid_references() {
        assert!(
            validate(&EXAMPLE.replace("input: name", "input: missing"))
                .unwrap_err()
                .contains("undeclared input")
        );
        assert!(
            validate(&EXAMPLE.replace("required: true", "required: false"))
                .unwrap_err()
                .contains("required or have a default")
        );
        assert!(validate(&EXAMPLE.replace("input: name", "input: ''")).is_err());
    }

    #[test]
    fn rejects_invalid_structure_and_command_fields() {
        for source in [
            EXAMPLE.replace("version: 1", "version: 2"),
            EXAMPLE.replace("version: 1\n", ""),
            EXAMPLE.replace("id: synthetic-report", "id: Invalid"),
            EXAMPLE.replace("label: Synthetic report", "label: ''"),
            EXAMPLE.replace("/synthetic/runtime/bin/report", "report"),
            EXAMPLE.replace(
                "working_directory: /synthetic/runtime",
                "working_directory: relative",
            ),
            EXAMPLE.replace("literal: --name", "literal: \"bad\\0value\""),
            EXAMPLE.replace("type: string", "type: float"),
            EXAMPLE.replace("literal: --name", "literal: 123"),
            format!("{EXAMPLE}\n---\nversion: 1\ntasks: []"),
        ] {
            assert!(validate(&source).is_err(), "{source}");
        }
    }
}
