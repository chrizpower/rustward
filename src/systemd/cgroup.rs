use super::{Cgroups, GroupState, valid_group};
use std::{fs::File, io::Read, path::PathBuf};

#[derive(Clone)]
pub struct CgroupFs {
    root: PathBuf,
}
impl Default for CgroupFs {
    fn default() -> Self {
        Self {
            root: PathBuf::from("/sys/fs/cgroup"),
        }
    }
}
impl Cgroups for CgroupFs {
    fn state(&mut self, path: &str) -> GroupState {
        if !valid_group(path) || !self.root.join("cgroup.controllers").is_file() {
            return GroupState::Unknown;
        }
        let file = match File::open(self.root.join(&path[1..]).join("cgroup.events")) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return GroupState::Absent;
            }
            Err(_) => return GroupState::Unknown,
        };
        let mut text = String::new();
        if file.take(4097).read_to_string(&mut text).is_err() || text.len() > 4096 {
            return GroupState::Unknown;
        }
        let mut populated = None;
        for line in text.lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.first() == Some(&"populated") {
                if populated.is_some() {
                    return GroupState::Unknown;
                }
                populated = match fields.as_slice() {
                    ["populated", "0"] => Some(GroupState::Empty),
                    ["populated", "1"] => Some(GroupState::Populated),
                    _ => return GroupState::Unknown,
                };
            }
        }
        populated.unwrap_or(GroupState::Unknown)
    }
}

#[cfg(test)]
mod tests;
