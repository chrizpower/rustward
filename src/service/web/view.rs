use std::{
    fs::File,
    io,
    os::unix::fs::{FileExt, MetadataExt},
};

use rusqlite::params;
use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
use serde_json::{Value, json};

use crate::{
    admission::{Error, Store, storage},
    runtime::WorkRoot,
    supervision::RunId,
};

pub(super) fn runs(store: &mut Store, before: Option<i64>, queued: bool) -> Result<Value, Error> {
    store.perform(|db| {
        let mut query = db
            .prepare(
                "SELECT sequence,runs.run_id,task_id,phase,result,schedule_id,due_ms
             FROM runs LEFT JOIN occurrences USING(run_id)
             WHERE sequence<?1 AND (?2=0 OR phase='queued') ORDER BY sequence DESC LIMIT 20",
            )
            .map_err(storage)?;
        let rows = query.query_map(params![before.unwrap_or(i64::MAX), queued], |r| Ok(json!({
            "sequence":r.get::<_, i64>(0)?, "run_id":r.get::<_, String>(1)?,
            "task_id":r.get::<_, String>(2)?, "phase":r.get::<_, String>(3)?,
            "result":r.get::<_, Option<String>>(4)?, "schedule_id":r.get::<_, Option<String>>(5)?,
            "due_ms":r.get::<_, Option<i64>>(6)?,
        }))).map_err(storage)?.collect::<Result<Vec<_>,_>>().map_err(storage)?;
        Ok(json!(rows))
    })
}

pub(super) fn schedule(store: &mut Store, id: &str) -> Result<(bool, i64, Option<i64>), Error> {
    store.perform(|db| {
        db.query_row(
            "SELECT state.paused OR control.paused, state.checkpoint_ms,
             (SELECT max(due_ms) FROM occurrences WHERE schedule_id=?1)
             FROM schedule_state state CROSS JOIN schedule_control control WHERE state.schedule_id=?1",
            [id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).map_err(storage)
    })
}

pub(super) fn output(root: &WorkRoot, id: &RunId, out: u64, err: u64) -> Result<Value, io::Error> {
    let root = root
        .checked_directory()
        .map_err(|_| io::Error::other("log root changed"))?;
    let open = |name: &str| {
        openat2(
            root,
            format!("{}/{name}", id.as_str()),
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_XDEV,
        )
    };
    let read = |name, offset: u64| -> io::Result<Value> {
        let file = match open(name) {
            Ok(file) => File::from(file),
            Err(rustix::io::Errno::NOENT) => return Ok(Value::Null),
            Err(error) => return Err(error.into()),
        };
        let meta = file.metadata()?;
        if !meta.is_file()
            || meta.mode() & 0o7777 != 0o600
            || meta.uid() != rustix::process::geteuid().as_raw()
            || meta.nlink() != 1
        {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let offset = offset.min(meta.len());
        let mut bytes = [0; 16_384];
        let read = file.read_at(&mut bytes, offset)?;
        Ok(
            json!({"next": offset + read as u64, "bytes": &bytes[..read], "more": offset + (read as u64) < meta.len()}),
        )
    };
    Ok(json!({"stdout":read("stdout.log", out)?, "stderr":read("stderr.log", err)?}))
}
