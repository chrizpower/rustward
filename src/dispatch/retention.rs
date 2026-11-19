use rusqlite::Transaction;

use crate::{
    admission::{Error, storage},
    supervision::RunId,
};

pub(super) type RemoveLogs<'a> = dyn FnMut(&RunId) -> Result<(), Error> + 'a;

pub(super) fn reclaim(
    transaction: &Transaction<'_>,
    mut needed: i64,
    remove_logs: &mut RemoveLogs<'_>,
) -> Result<bool, Error> {
    let eligible = "FROM runs JOIN launches USING(run_id)
        WHERE phase='completed' AND completed_at IS NOT NULL AND cleanup_confirmed=1
        AND review_required=0 AND log_bytes>0";
    let reclaimable: i64 = transaction
        .query_row(
            &format!("SELECT COALESCE(sum(log_bytes),0) {eligible}"),
            [],
            |row| row.get(0),
        )
        .map_err(storage)?;
    if reclaimable < needed {
        return Ok(false);
    }

    let mut oldest = transaction
        .prepare(&format!(
            "SELECT run_id,log_bytes {eligible} ORDER BY completed_at,sequence LIMIT 1"
        ))
        .map_err(storage)?;
    let mut release = transaction
        .prepare("UPDATE launches SET log_bytes=0 WHERE run_id=?1")
        .map_err(storage)?;
    while needed > 0 {
        let (id, bytes): (String, i64) = oldest
            .query_row([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(storage)?;
        remove_logs(&RunId::new(&id).map_err(|_| Error::Storage)?)?;
        // Keep the charge until removal is durable. A retry can remove an
        // already absent file if the transaction did not commit.
        if release.execute([id]).map_err(storage)? != 1 {
            return Err(Error::Storage);
        }
        needed = needed.saturating_sub(bytes);
    }
    Ok(true)
}
