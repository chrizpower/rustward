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

pub struct UserBus {
    connection: Connection,
}
impl UserBus {
    pub fn connect() -> Result<Self, ManagerError> {
        let connection = zbus::blocking::connection::Builder::session()
            .map_err(classify)?
            .method_timeout(Duration::from_secs(5))
            .build()
            .map_err(classify)?;
        Ok(Self { connection })
    }
    fn manager(&self) -> Result<Proxy<'_>, ManagerError> {
        Proxy::new(&self.connection, DESTINATION, PATH, MANAGER).map_err(classify)
    }
    fn properties(
        &self,
        path: &str,
        interface: &str,
    ) -> Result<HashMap<String, OwnedValue>, ManagerError> {
        let proxy = Proxy::new(
            &self.connection,
            DESTINATION,
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
