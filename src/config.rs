use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use toml::Value;

mod files;
mod schedule;
mod service;
#[cfg(target_os = "linux")]
pub(crate) use files::SERVICE_FILENAME_ERROR;
pub use files::load;
pub use schedule::Schedule;
pub use service::{Definitions, Limits, Retention, RunResources, Service, Storage};

#[derive(Debug)]
pub struct Config {
    tasks: Vec<TaskDefinition>,
    schedules: Vec<Schedule>,
    service: Service,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TaskDefinition {
    #[serde(skip_deserializing)]
    id: String,
    label: String,
    executable: String,
    working_directory: String,
    arguments: Vec<ArgumentSource>,
    inputs: Vec<InputDeclaration>,
    #[serde(default = "default_timeout")]
    timeout_seconds: u32,
    #[serde(default = "default_grace")]
    cleanup_grace_seconds: u32,
}

fn default_timeout() -> u32 {
    3600
}
fn default_grace() -> u32 {
    30
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

    fn resolve<'a>(
        &'a self,
        submitted: Option<&'a Value>,
    ) -> Result<Option<&'a Value>, &'static str> {
        let value = submitted.or(self.default.as_ref());
        if let Some(value) = value {
            self.validate_value(value)?;
        } else if self.required {
            return Err("missing required input");
        }
        Ok(value)
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

fn valid_definition_id(id: &str) -> bool {
    id.split('/').all(valid_id)
}

fn valid_path(path: &str) -> bool {
    Path::new(path).is_absolute() && !path.contains('\0')
}

fn validate_tasks(tasks: &[TaskDefinition]) -> Result<(), String> {
    let mut task_ids = HashSet::new();
    for task in tasks {
        let fail = |message: &str| format!("task {}: {message}", task.id);
        if !valid_definition_id(&task.id) {
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
        if task.timeout_seconds == 0 || task.cleanup_grace_seconds == 0 {
            return Err(fail(
                "timeout_seconds and cleanup_grace_seconds must be positive",
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
    Ok(())
}

impl Config {
    pub(crate) fn admission_snapshot(&self, task_id: &str) -> (Vec<u8>, Vec<u8>) {
        let task = self.tasks.iter().find(|task| task.id == task_id).unwrap();
        (
            serde_json::to_vec(&serde_json::to_value(task).unwrap()).unwrap(),
            serde_json::to_vec(&serde_json::json!({
                "version": 2, "tasks": self.tasks, "schedules": self.schedules,
            }))
            .unwrap(),
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

    #[cfg(target_os = "linux")]
    pub(crate) fn task_cards(&self, after: &str) -> Vec<serde_json::Value> {
        self.tasks
            .iter()
            .filter(|task| task.id.as_str() > after)
            .take(20)
            .map(|task| serde_json::json!({"id": task.id, "label": task.label}))
            .collect()
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn task_form(&self, id: &str) -> Option<serde_json::Value> {
        let task = self.tasks.iter().find(|task| task.id == id)?;
        let inputs: Vec<_> = task.inputs.iter().map(|input| {
            let display = |value: &Value| match value {
                Value::Integer(number) => serde_json::Value::String(number.to_string()),
                _ => serde_json::json!(value),
            };
            serde_json::json!({"id":input.id,"type":input.kind,"required":input.required,
                "default":input.default.as_ref().map(display),
                "choices":input.choices.as_ref().map(|values| values.iter().map(display).collect::<Vec<_>>())})
        }).collect();
        Some(serde_json::json!({"id":task.id,"label":task.label,"inputs":inputs}))
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn input_errors(
        &self,
        id: &str,
        submitted: &BTreeMap<String, Value>,
    ) -> BTreeMap<String, &'static str> {
        let Some(task) = self.tasks.iter().find(|task| task.id == id) else {
            return BTreeMap::new();
        };
        task.inputs
            .iter()
            .filter_map(|input| {
                let error = input.resolve(submitted.get(&input.id)).err()?;
                let message = match error {
                    "missing required input" => "Enter a value.",
                    "value is not a declared choice" => "Select one of the available values.",
                    _ => "Enter a value of the required type without any NUL characters.",
                };
                Some((input.id.clone(), message))
            })
            .collect()
    }

    pub fn service(&self) -> &Service {
        &self.service
    }

    pub fn schedules(&self) -> &[Schedule] {
        &self.schedules
    }

    pub fn task_ids(&self) -> impl Iterator<Item = &str> {
        self.tasks.iter().map(|task| task.id.as_str())
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
            if let Some(value) = input
                .resolve(submitted_values.get(&input.id))
                .map_err(fail)?
            {
                input_values.insert(input.id.as_str(), value);
            }
        }
        Ok(input_values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_complete_identity_uniqueness_and_components() {
        let task = |id: &str| {
            let mut task: TaskDefinition =
                toml::from_str(include_str!("../examples/tasks/synthetic-report.toml")).unwrap();
            task.id = id.into();
            task
        };
        assert!(validate_tasks(&[task("backup/report"), task("maintenance/report")]).is_ok());
        assert!(
            validate_tasks(&[task("backup/report"), task("backup/report")])
                .unwrap_err()
                .contains("duplicate task ID")
        );
        for id in [
            "", "/report", "report/", "a//b", "./a", "a/../b", "A/b", "a/b_c", "a\\b", "1a", "é",
        ] {
            assert!(
                validate_tasks(&[task(id)])
                    .unwrap_err()
                    .contains("invalid task ID"),
                "{id}"
            );
        }
    }
}
