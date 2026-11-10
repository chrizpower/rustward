use super::{Manager, ManagerError, Snapshot};
use crate::supervision::StopMode;
use std::{collections::HashMap, time::Duration};
use zbus::{
    blocking::{Connection, Proxy},
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

const DESTINATION: &str = "org.freedesktop.systemd1";
const MANAGER: &str = "org.freedesktop.systemd1.Manager";
const PATH: &str = "/org/freedesktop/systemd1";

#[derive(Clone)]
pub struct UserBus {
    connection: Connection,
    owner: Option<String>,
}
impl UserBus {
    pub fn connect() -> Result<Self, ManagerError> {
        let connection = zbus::blocking::connection::Builder::session()
            .map_err(classify)?
            .method_timeout(Duration::from_secs(5))
            .build()
            .map_err(classify)?;
        let mut bus = Self {
            connection,
            owner: None,
        };
        bus.authenticated_uid()?;
        Ok(bus)
    }
    fn manager(&self) -> Result<Proxy<'_>, ManagerError> {
        Proxy::new(
            &self.connection,
            self.owner.as_deref().unwrap_or(DESTINATION),
            PATH,
            MANAGER,
        )
        .map_err(classify)
    }
    fn properties(
        &self,
        path: &str,
        interface: &str,
    ) -> Result<HashMap<String, OwnedValue>, ManagerError> {
        let proxy = Proxy::new(
            &self.connection,
            self.owner.as_deref().unwrap_or(DESTINATION),
            path,
            "org.freedesktop.DBus.Properties",
        )
        .map_err(classify)?;
        proxy.call("GetAll", &(interface,)).map_err(classify)
    }
}

fn classify(error: zbus::Error) -> ManagerError {
    if let zbus::Error::MethodError(name, _, _) = error {
        return match name.as_str() {
            "org.freedesktop.systemd1.NoSuchUnit" => ManagerError::Missing,
            "org.freedesktop.systemd1.UnitExists" => ManagerError::Conflict,
            "org.freedesktop.DBus.Error.AccessDenied"
            | "org.freedesktop.DBus.Error.InvalidArgs" => ManagerError::Rejected,
            _ => ManagerError::Unavailable,
        };
    }
    ManagerError::Unavailable
}

fn get<T>(values: &mut HashMap<String, OwnedValue>, name: &str) -> Result<T, ManagerError>
where
    T: TryFrom<OwnedValue>,
{
    values
        .remove(name)
        .and_then(|v| T::try_from(v).ok())
        .ok_or(ManagerError::Unavailable)
}

impl Manager for UserBus {
    fn reference(&mut self, name: &str) -> Result<(), ManagerError> {
        self.manager()?.call("RefUnit", &(name,)).map_err(classify)
    }
    fn environment_names(&mut self) -> Result<Vec<String>, ManagerError> {
        let values: Vec<String> = get(&mut self.properties(PATH, MANAGER)?, "Environment")?;
        if values.len() > 4096 || values.iter().map(String::len).sum::<usize>() > 1024 * 1024 {
            return Err(ManagerError::Unavailable);
        }
        values
            .into_iter()
            .map(|value| {
                value
                    .split_once('=')
                    .map(|(name, _)| name.to_owned())
                    .ok_or(ManagerError::Unavailable)
            })
            .collect()
    }

    fn authenticated_uid(&mut self) -> Result<u32, ManagerError> {
        let bus = Proxy::new(
            &self.connection,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        )
        .map_err(classify)?;
        let owner: String = bus
            .call("GetNameOwner", &(DESTINATION,))
            .map_err(classify)?;
        if self
            .owner
            .as_ref()
            .is_some_and(|previous| previous != &owner)
        {
            return Err(ManagerError::Unavailable);
        }
        let uid: u32 = bus
            .call("GetConnectionUnixUser", &(owner.as_str(),))
            .map_err(classify)?;
        self.owner = Some(owner);
        Ok(uid)
    }

    fn version(&mut self) -> Result<String, ManagerError> {
        get(&mut self.properties(PATH, MANAGER)?, "Version")
    }
    fn start(&mut self, name: &str, properties: &[(&str, Value<'_>)]) -> Result<(), ManagerError> {
        let auxiliary: Vec<(&str, Vec<(&str, Value<'_>)>)> = vec![];
        let _: OwnedObjectPath = self
            .manager()?
            .call("StartTransientUnit", &(name, "fail", properties, auxiliary))
            .map_err(classify)?;
        Ok(())
    }
    fn snapshot(&mut self, name: &str) -> Result<Snapshot, ManagerError> {
        let path: OwnedObjectPath = self
            .manager()?
            .call("GetUnit", &(name,))
            .map_err(classify)?;
        let mut unit = self.properties(path.as_str(), "org.freedesktop.systemd1.Unit")?;
        let mut service = self.properties(path.as_str(), "org.freedesktop.systemd1.Service")?;
        let (job, _): (u32, OwnedObjectPath) = get(&mut unit, "Job")?;
        Ok(Snapshot {
            description: get(&mut unit, "Description")?,
            transient: get(&mut unit, "Transient")?,
            service_type: get(&mut service, "Type")?,
            invocation: get(&mut unit, "InvocationID")?,
            control_group: get(&mut service, "ControlGroup")?,
            active_state: get(&mut unit, "ActiveState")?,
            sub_state: get(&mut unit, "SubState")?,
            job,
            main_pid: get(&mut service, "MainPID")?,
            exit_timestamp: get(&mut service, "ExecMainExitTimestampMonotonic")?,
            exit_code: get(&mut service, "ExecMainCode")?,
            exit_status: get(&mut service, "ExecMainStatus")?,
        })
    }
    fn stop(&mut self, name: &str, mode: StopMode) -> Result<(), ManagerError> {
        let proxy = self.manager()?;
        match mode {
            StopMode::Graceful => {
                let _: OwnedObjectPath = proxy
                    .call("StopUnit", &(name, "replace"))
                    .map_err(classify)?;
            }
            StopMode::Force => {
                let _: () = proxy
                    .call("KillUnit", &(name, "all", 9i32))
                    .map_err(classify)?;
            }
        }
        Ok(())
    }
    fn release(&mut self, name: &str) -> Result<(), ManagerError> {
        self.manager()?
            .call("UnrefUnit", &(name,))
            .map_err(classify)
    }
}

#[cfg(test)]
mod tests;
