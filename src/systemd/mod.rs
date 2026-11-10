mod bus;
mod cgroup;

pub use bus::UserBus;
pub use cgroup::CgroupFs;

use crate::supervision::*;
use std::path::{Component, Path};
use zbus::zvariant::{Fd, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManagerError {
    Missing,
    Conflict,
    Rejected,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub description: String,
    pub transient: bool,
    pub service_type: String,
    pub invocation: Vec<u8>,
    pub control_group: String,
    pub active_state: String,
    pub sub_state: String,
    pub job: u32,
    pub main_pid: u32,
    pub exit_timestamp: u64,
    pub exit_code: i32,
    pub exit_status: i32,
}

/// Systemd transport boundary. Implementations must read fresh properties.
pub trait Manager {
    fn reference(&mut self, _name: &str) -> Result<(), ManagerError> {
        Err(ManagerError::Unavailable)
    }
    fn environment_names(&mut self) -> Result<Vec<String>, ManagerError> {
        Err(ManagerError::Unavailable)
    }
    fn authenticated_uid(&mut self) -> Result<u32, ManagerError> {
        Err(ManagerError::Unavailable)
    }
    fn version(&mut self) -> Result<String, ManagerError>;
    fn start(&mut self, name: &str, properties: &[(&str, Value<'_>)]) -> Result<(), ManagerError>;
    fn snapshot(&mut self, name: &str) -> Result<Snapshot, ManagerError>;
    fn stop(&mut self, name: &str, mode: StopMode) -> Result<(), ManagerError>;
    fn release(&mut self, name: &str) -> Result<(), ManagerError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupState {
    Empty,
    Absent,
    Populated,
    Unknown,
}

pub trait Cgroups {
    fn state(&mut self, path: &str) -> GroupState;
}

#[derive(Clone)]
pub struct Systemd<M, C> {
    manager: M,
    cgroups: C,
}

impl<M: Manager, C: Cgroups> Systemd<M, C> {
    pub fn new(mut manager: M, cgroups: C) -> Result<Self, Error> {
        let version = manager.version().map_err(|_| Error::Unavailable)?;
        let major = version
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .unwrap_or("");
        if major.parse::<u32>().ok().is_none_or(|v| v < 257) {
            return Err(Error::IncompatibleManager);
        }
        Ok(Self { manager, cgroups })
    }

    fn identity(
        &self,
        run_id: &RunId,
        snapshot: &Snapshot,
        expected: Option<&ExecutionIdentity>,
    ) -> Result<Option<ExecutionIdentity>, Error> {
        if snapshot.description != description(run_id)
            || !snapshot.transient
            || snapshot.service_type != "exec"
        {
            return Err(Error::IdentityMismatch);
        }
        let invocation: [u8; 16] = match snapshot.invocation.as_slice().try_into() {
            Ok(value) if value != [0; 16] => value,
            _ => return Ok(None),
        };
        if let Some(expected) = expected {
            if expected.invocation != invocation
                || !valid_group(&expected.control_group)
                || (!snapshot.control_group.is_empty()
                    && snapshot.control_group != expected.control_group)
            {
                return Err(Error::IdentityMismatch);
            }
            return Ok(Some(expected.clone()));
        }
        if !valid_group(&snapshot.control_group) {
            return Ok(None);
        }
        Ok(Some(ExecutionIdentity {
            invocation,
            control_group: snapshot.control_group.clone(),
        }))
    }
}

fn unit_name(run_id: &RunId) -> String {
    format!("rustward-run-{}.service", run_id.as_str())
}
fn description(run_id: &RunId) -> String {
    format!("Rustward run {}", run_id.as_str())
}

fn valid_group(path: &str) -> bool {
    path.starts_with('/')
        && path != "/"
        && !path.contains('\0')
        && path
            .split('/')
            .skip(1)
            .all(|p| !p.is_empty() && p != "." && p != "..")
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
}

impl<M: Manager, C: Cgroups> Systemd<M, C> {
    fn start_unit(
        &mut self,
        launch: &Launch,
        output: Output<'_>,
        profile: Option<&Profile>,
    ) -> Result<StartOutcome, Error> {
        let valid_path = |text: &str| Path::new(text).is_absolute() && !text.contains('\0');
        if !valid_path(&launch.executable)
            || !valid_path(&launch.working_directory)
            || launch.arguments.len() > 65536
            || launch.arguments.iter().any(|a| a.contains('\0'))
            || launch.arguments.iter().map(String::len).sum::<usize>()
                + launch.executable.len()
                + launch.working_directory.len()
                > 1024 * 1024
            || launch.cleanup_grace.as_micros() == 0
            || launch.cleanup_grace.as_micros() > u64::MAX as u128
        {
            return Err(Error::InvalidRequest);
        }
        let mut argv = if let Some(profile) = profile {
            vec![
                profile.launcher(),
                profile.runtime_directory(),
                profile.work_directory(),
            ]
        } else {
            vec![]
        };
        argv.push(launch.executable.as_str());
        argv.extend(launch.arguments.iter().map(String::as_str));
        let label = description(&launch.run_id);
        let mut properties = vec![
            ("Description", Value::from(label.as_str())),
            ("Type", Value::from("exec")),
            ("Restart", Value::from("no")),
            ("RemainAfterExit", Value::from(true)),
            ("AddRef", Value::from(true)),
            ("KillMode", Value::from("control-group")),
            ("SendSIGKILL", Value::from(true)),
            (
                "TimeoutStopUSec",
                Value::from(launch.cleanup_grace.as_micros() as u64),
            ),
            (
                "WorkingDirectory",
                Value::from(launch.working_directory.as_str()),
            ),
            (
                "ExecStartEx",
                Value::new(vec![(
                    profile.map_or(launch.executable.as_str(), Profile::launcher),
                    argv,
                    vec!["no-env-expand"],
                )]),
            ),
            ("StandardInput", Value::from("null")),
            (
                "StandardOutputFileDescriptor",
                Value::from(Fd::from(output.stdout)),
            ),
            (
                "StandardErrorFileDescriptor",
                Value::from(Fd::from(output.stderr)),
            ),
        ];
        let unset = if profile.is_some() {
            let mut names = self
                .manager
                .environment_names()
                .map_err(|_| Error::Unavailable)?;
            if names.len() > 4096
                || names.iter().map(String::len).sum::<usize>() > 1024 * 1024
                || names
                    .iter()
                    .any(|name| name.is_empty() || name.contains(['=', '\0']))
            {
                return Err(Error::InvalidRequest);
            }
            names.extend(
                [
                    "LD_PRELOAD",
                    "LD_LIBRARY_PATH",
                    "LD_AUDIT",
                    "LD_DEBUG",
                    "LD_DEBUG_OUTPUT",
                    "GLIBC_TUNABLES",
                    "GCONV_PATH",
                    "LOCPATH",
                    "NLSPATH",
                    "MALLOC_TRACE",
                    "MALLOC_CONF",
                    "MALLOC_CHECK_",
                    "MALLOC_PERTURB_",
                ]
                .into_iter()
                .map(str::to_owned),
            );
            names.sort();
            names.dedup();
            names
        } else {
            vec![]
        };
        if profile.is_some() {
            properties.push(("NoNewPrivileges", Value::from(true)));
            properties.push(("CapabilityBoundingSet", Value::from(0_u64)));
            properties.push(("AmbientCapabilities", Value::from(0_u64)));
            properties.push((
                "UnsetEnvironment",
                Value::new(unset.iter().map(String::as_str).collect::<Vec<_>>()),
            ));
        }
        Ok(
            match self.manager.start(&unit_name(&launch.run_id), &properties) {
                Ok(()) => StartOutcome::Accepted,
                Err(ManagerError::Rejected) => StartOutcome::Rejected,
                Err(ManagerError::Conflict) => StartOutcome::Conflict,
                Err(_) => StartOutcome::Uncertain,
            },
        )
    }
}

impl<M: Manager, C: Cgroups> Supervisor for Systemd<M, C> {
    fn reference(
        &mut self,
        run_id: &RunId,
        expected: Option<&ExecutionIdentity>,
    ) -> Result<(), Error> {
        let name = unit_name(run_id);
        let before = self
            .manager
            .snapshot(&name)
            .map_err(|_| Error::Unavailable)?;
        let identity = self
            .identity(run_id, &before, expected)?
            .ok_or(Error::IdentityMismatch)?;
        self.manager
            .reference(&name)
            .map_err(|_| Error::Unavailable)?;
        let after = self
            .manager
            .snapshot(&name)
            .map_err(|_| Error::Unavailable)?;
        self.identity(run_id, &after, Some(&identity))?
            .ok_or(Error::IdentityMismatch)?;
        Ok(())
    }
    fn execution_uid(&mut self) -> Result<u32, Error> {
        self.manager
            .authenticated_uid()
            .map_err(|_| Error::Unavailable)
    }
    fn start(&mut self, launch: &Launch, output: Output<'_>) -> Result<StartOutcome, Error> {
        self.start_unit(launch, output, None)
    }
    fn start_controlled(
        &mut self,
        launch: &Launch,
        profile: &Profile,
        output: Output<'_>,
    ) -> Result<StartOutcome, Error> {
        if self.execution_uid()? != profile.execution_uid() {
            return Err(Error::IdentityMismatch);
        }
        self.start_unit(launch, output, Some(profile))
    }

    fn observe(
        &mut self,
        run_id: &RunId,
        expected: Option<&ExecutionIdentity>,
    ) -> Result<Observation, Error> {
        let name = unit_name(run_id);
        let before = match self.manager.snapshot(&name) {
            Ok(snapshot) => snapshot,
            Err(ManagerError::Missing) => return Ok(Observation::Missing),
            Err(_) => return Err(Error::Unavailable),
        };
        let Some(identity) = self.identity(run_id, &before, expected)? else {
            return Ok(Observation::Uncertain);
        };
        let group = self.cgroups.state(&identity.control_group);
        let after = match self.manager.snapshot(&name) {
            Ok(snapshot) => snapshot,
            Err(_) => return Ok(Observation::Uncertain),
        };
        if before != after {
            return Ok(Observation::Uncertain);
        }
        let state = match before.active_state.as_str() {
            "activating" => ExecutionState::Starting,
            "active" if before.sub_state == "running" && before.main_pid > 0 => {
                ExecutionState::Running
            }
            "active" if before.sub_state == "exited" && before.exit_timestamp > 0 => {
                ExecutionState::Exited
            }
            "inactive" | "failed" => ExecutionState::Ended,
            "deactivating" => ExecutionState::Stopping,
            _ => return Ok(Observation::Uncertain),
        };
        let exit = if before.exit_timestamp == 0 {
            None
        } else {
            match before.exit_code {
                1 => Some(ProcessExit::Code(before.exit_status)),
                2 | 3 => Some(ProcessExit::Signal(before.exit_status)),
                _ => None,
            }
        };
        let cleanup = if state == ExecutionState::Ended
            && before.job == 0
            && before.main_pid == 0
            && matches!(group, GroupState::Empty | GroupState::Absent)
        {
            Cleanup::Confirmed
        } else {
            Cleanup::Pending
        };
        Ok(Observation::Known {
            identity,
            state,
            exit,
            cleanup,
        })
    }

    fn stop(
        &mut self,
        run_id: &RunId,
        expected: &ExecutionIdentity,
        mode: StopMode,
    ) -> Result<StopOutcome, Error> {
        match self.observe(run_id, Some(expected))? {
            Observation::Missing => return Ok(StopOutcome::Missing),
            Observation::Uncertain => return Ok(StopOutcome::Uncertain),
            _ => {}
        }
        Ok(match self.manager.stop(&unit_name(run_id), mode) {
            Ok(()) => StopOutcome::Accepted,
            Err(ManagerError::Missing) => StopOutcome::Missing,
            Err(_) => StopOutcome::Uncertain,
        })
    }

    fn release(&mut self, run_id: &RunId, expected: &ExecutionIdentity) -> Result<(), Error> {
        if !matches!(
            self.observe(run_id, Some(expected))?,
            Observation::Known {
                cleanup: Cleanup::Confirmed,
                ..
            }
        ) {
            return Err(Error::CleanupPending);
        }
        self.manager
            .release(&unit_name(run_id))
            .map_err(|_| Error::Unavailable)
    }
}
