use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use toml::Value;

#[derive(Debug)]
pub struct Config {
    tasks: Vec<TaskDefinition>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    tasks: Vec<TaskDefinition>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TaskDefinition {
    id: String,
    label: String,
    executable: String,
    working_directory: String,
    arguments: Vec<ArgumentSource>,
    inputs: Vec<InputDeclaration>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum ArgumentSource {
    Literal(LiteralArgument),
    InputReference(InputReference),
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LiteralArgument {
    literal: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InputReference {
    input: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InputDeclaration {
    id: String,
    #[serde(rename = "type")]
    kind: InputType,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    default: Option<Value>,
    #[serde(default)]
    choices: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum InputType {
    String,
    Integer,
    Boolean,
}

impl InputDeclaration {
    fn validate_value(&self, value: &Value) -> Result<(), &'static str> {
        if !self.accepts_type(value) {
            return Err("value must match the input type and contain no NUL");
        }
        if self
            .choices
            .as_ref()
            .is_some_and(|choices| !choices.contains(value))
        {
            return Err("value is not a declared choice");
        }
        Ok(())
    }

    fn accepts_type(&self, value: &Value) -> bool {
        match self.kind {
            InputType::String => value.as_str().is_some_and(|text| !text.contains('\0')),
            InputType::Integer => value.as_integer().is_some(),
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
    let config: Document = toml::from_str(source).map_err(|error| error.to_string())?;
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
                if choices.is_empty() || choices.iter().any(|value| !input.accepts_type(value)) {
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
                && input.validate_value(default).is_err()
            {
                return Err(input_fail("invalid default"));
            }
        }
        for argument in &task.arguments {
            match argument {
                ArgumentSource::Literal(value) if value.literal.contains('\0') => {
                    return Err(fail("literal argument contains NUL"));
                }
                ArgumentSource::InputReference(reference) => {
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
    Ok(Config {
        tasks: config.tasks,
    })
}

impl Config {
    pub(crate) fn admission_snapshot(&self, task_id: &str) -> (Vec<u8>, Vec<u8>) {
        let task = self.tasks.iter().find(|task| task.id == task_id).unwrap();
        (
            serde_json::to_vec(&serde_json::to_value(task).unwrap()).unwrap(),
            serde_json::to_vec(&serde_json::to_value(&self.tasks).unwrap()).unwrap(),
        )
    }

    /// Validate submitted values and generate one argument per configured entry.
    /// Missing values use defaults. Errors do not include submitted values.
    pub fn arguments(
        &self,
        task_id: &str,
        values: &BTreeMap<String, Value>,
    ) -> Result<Vec<String>, String> {
        let task = self
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .ok_or_else(|| format!("task {task_id}: unknown task ID"))?;
        let input_values = task.validate_submitted_values(values)?;
        Ok(task
            .arguments
            .iter()
            .map(|argument| match argument {
                ArgumentSource::Literal(value) => value.literal.clone(),
                ArgumentSource::InputReference(reference) => {
                    let value = input_values[reference.input.as_str()];
                    match value {
                        Value::String(text) => text.clone(),
                        Value::Integer(number) => number.to_string(),
                        Value::Boolean(value) => value.to_string(),
                        _ => unreachable!("input values have validated scalar types"),
                    }
                }
            })
            .collect())
    }

    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }
}

impl TaskDefinition {
    fn validate_submitted_values<'a>(
        &'a self,
        submitted_values: &'a BTreeMap<String, Value>,
    ) -> Result<BTreeMap<&'a str, &'a Value>, String> {
        let task_id = &self.id;
        for id in submitted_values.keys() {
            if !self.inputs.iter().any(|input| input.id == *id) {
                return Err(format!("task {task_id}: input {id}: undeclared input"));
            }
        }
        let mut input_values = BTreeMap::new();
        for input in &self.inputs {
            let fail = |message| format!("task {task_id}: input {}: {message}", input.id);
            let Some(value) = submitted_values.get(&input.id).or(input.default.as_ref()) else {
                if input.required {
                    return Err(fail("missing required input"));
                }
                continue;
            };
            input.validate_value(value).map_err(fail)?;
            input_values.insert(input.id.as_str(), value);
        }
        Ok(input_values)
    }
}

#[cfg(test)]
mod tests {
    use super::validate;

    const EXAMPLE: &str = include_str!("../examples/tasks.toml");

    #[test]
    fn accepts_typed_inputs_and_synthetic_paths() {
        assert_eq!(validate(EXAMPLE).unwrap().task_count(), 1);
        assert!(validate("version = 1\ntasks = []").is_ok());
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
        let task = EXAMPLE.split_once("[[tasks]]").unwrap().1;
        assert!(
            validate(&format!("{EXAMPLE}\n[[tasks]]{task}"))
                .unwrap_err()
                .contains("duplicate task ID")
        );
        for source in [
            EXAMPLE.replace("id = \"count\"", "id = \"name\""),
            EXAMPLE.replace("version = 1", "version = 1\nversion = 1"),
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
            EXAMPLE.replace("version = 1", "version = 2"),
            EXAMPLE.replace("version = 1", ""),
            EXAMPLE.replace("id = \"synthetic-report\"", "id = \"Invalid\""),
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
        let source = r#"version = 1
[[tasks]]
id = "country-report"
label = "Country report"
executable = "/synthetic/report"
working_directory = "/synthetic"
[[tasks.arguments]]
input = "country"
[[tasks.inputs]]
id = "country"
type = "string"
default = "NO"
choices = ["NO", "on"]
"#;
        let config = validate(source).unwrap();
        assert_eq!(
            config
                .arguments("country-report", &Default::default())
                .unwrap(),
            ["NO"]
        );
        assert!(validate(&source.replace("default = \"NO\"", "default = false")).is_err());
    }
}
